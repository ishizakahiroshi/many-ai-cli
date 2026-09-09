package hub

import (
	"fmt"
	"strings"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// subscriptionConfigDir は profile 実体を置く親ディレクトリ（~/.many-ai-cli）。
// 解決できない環境では空文字を返し、subscription 機能だけが無効になる
// （Hub 全体は従来どおり動く）。
func subscriptionConfigDir() string {
	dir, err := config.Dir()
	if err != nil {
		return ""
	}
	return dir
}

// resolveSubscriptionLabel は wrapper が申告した profile ID を検証し、表示名を引く。
//
// 表示名は config から都度引く。ID を正本にして名前を派生値にしているので、
// profile を rename しても過去セッションの追跡（ID 一致）は壊れない。
// config に無い ID（profile 削除後に残った古いセッション等）でも ID はそのまま
// 残し、名前だけ空にする。**ここで別 profile へ寄せない。**
func (s *Server) resolveSubscriptionLabel(provider, rawID string) (string, string) {
	id := config.NormalizeSubscriptionID(rawID)
	if id == "" {
		return "", ""
	}
	if err := config.ValidateSubscriptionID(id); err != nil {
		return "", ""
	}
	s.cfgMu.Lock()
	profile, found := s.cfg.Subscriptions.Find(provider, id)
	s.cfgMu.Unlock()
	if !found {
		return id, ""
	}
	return id, strings.TrimSpace(profile.Name)
}

// subscriptionProfileInUse reports whether a live Hub session still carries
// the profile ID. Credential directories must not be removed while such a
// session is active: its child process already has the profile-specific
// environment and may still refresh tokens or reconnect.
func (s *Server) subscriptionProfileInUse(provider, rawID string) bool {
	provider = strings.TrimSpace(provider)
	id := config.NormalizeSubscriptionID(rawID)
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	for _, ses := range s.sessions {
		if ses == nil || isTerminalSessionState(ses.State) {
			continue
		}
		if !strings.EqualFold(strings.TrimSpace(ses.Provider), provider) {
			continue
		}
		if config.NormalizeSubscriptionID(ses.SubscriptionProfileID) == id {
			return true
		}
	}
	return false
}

// pickAutoSubscription は auto 指定のときに使う profile を 1 つ選ぶ。
//
// # 設計不変条件: 残量を見て自動で契約を切り替えない（正本）
//
// ここは有効な profile を spawn 時に round-robin するだけで、残量・上限・リセット時刻の
// たぐいを一切参照しない。これは実装上の制限ではなく、意図して固定している境界である。
// 初版のこのコメントは「対応 provider のどれも残量を出さないから round-robin にした」と
// 理由を書いていたが、その前提はすでに崩れている（subscription_usage.go が Claude /
// Codex / Grok の残量を持っている）。取れるようになった今も見ないのは、見ないと決めた
// からである。
//
// なぜ固定するか。「上限に当たったら別アカウントへ自動で切り替える」は、各ベンダーが
// 広く禁じている「レート制限・保護措置の回避」そのものの自動化になる。利用者が spawn
// 画面で契約を選ぶのは利用者の判断だが、Hub が残量を見て勝手に乗り換えれば、それは本
// ツールが回避を実装したことになる。README の「自分のアカウントを複数積む使い方は自己
// 責任」節が成立するのは、ツール側が回避を自動化していないという前提の上である。
//
// したがって、この関数に残量・quota・リセット時刻を持ち込む変更は入れない。「残量の多い
// 方を選ぶ」「上限に当たった profile を飛ばす」も同じ理由で入れない。残量は表示（Usage
// メニュー）までに留め、どれを使うかは利用者が決める。機械的な裏づけは
// TestAutoSubscriptionNeverConsultsUsage。
//
// **選んだ結果は具体的な ID として記録される**ので、後からどのセッションがどの契約を
// 使ったかは追える。
func (s *Server) pickAutoSubscription(cfg *config.Config, provider string) (string, error) {
	candidates := subscription.Selectable(cfg, provider)
	if len(candidates) == 0 {
		// 既定ログインへ黙って落とさない。auto を選んだ利用者は「登録した契約の
		// どれかで動く」ことを期待しており、0 件は設定の問題として見せるべき。
		return "", fmt.Errorf("%w: %s", subscription.ErrNoSelectableProfile, provider)
	}
	s.subscriptionRRMu.Lock()
	if s.subscriptionRR == nil {
		s.subscriptionRR = map[string]int{}
	}
	idx := s.subscriptionRR[provider] % len(candidates)
	s.subscriptionRR[provider] = (idx + 1) % len(candidates)
	s.subscriptionRRMu.Unlock()
	return candidates[idx], nil
}

// subscriptionLaunch は spawn 1 件分の profile を解決し、子プロセス env に重ねる
// KEY=VALUE 列を返す。
//
// profileID が空なら (nil, nil, nil) を返す。この場合、呼び出し側の env は 1 バイトも
// 変わらない＝ subscription を設定していない利用者の起動経路は完全に従来どおり。
// `auto` は予約語で、有効な profile から 1 つ選ぶ。
func (s *Server) subscriptionLaunch(provider, profileID string) ([]string, *subscription.Resolved, error) {
	if strings.TrimSpace(profileID) == "" {
		return nil, nil, nil
	}
	cfg := s.snapshotCfg()
	dir := subscriptionConfigDir()
	if dir == "" {
		return nil, nil, fmt.Errorf("cannot resolve the many-ai-cli config directory")
	}
	if config.NormalizeSubscriptionID(profileID) == config.SubscriptionAutoID {
		picked, err := s.pickAutoSubscription(cfg, provider)
		if err != nil {
			return nil, nil, err
		}
		profileID = picked
	}
	resolved, err := subscription.Resolve(cfg, dir, provider, profileID)
	if err != nil {
		return nil, nil, err
	}
	if resolved == nil {
		return nil, nil, nil
	}
	// vendor CLI は指定されたディレクトリが無いと自分で作る場合と落ちる場合がある。
	// 起動前に本人のみアクセス可の権限で用意し、利用者の既定設定から不足分
	// （共通ルール・スキル・承認設定など）を持ち込む。
	seeded, err := subscription.EnsureProfileDir(provider, resolved.ProfileDir)
	if err != nil {
		return nil, nil, err
	}
	if seeded.Any() {
		// 何を持ち込んだかは残す。持ち込みは additive なので既存の値を壊さないが、
		// 「なぜ profile にこのファイルがあるのか」を後から辿れるようにする。
		s.logger.Info("subscription profile seeded",
			"provider", provider, "id", resolved.ID,
			"applied", seeded.Applied, "failed", seeded.Failed, "degraded", seeded.Degraded)
	}
	env := append([]string(nil), resolved.Env...)
	// wrapper がこの値を register で申告し、Hub が「実際に何で起動したか」を記録する。
	env = append(env, subscription.SessionEnvVar+"="+resolved.ID)
	return env, resolved, nil
}
