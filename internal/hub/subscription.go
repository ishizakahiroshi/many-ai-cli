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

// pickAutoSubscription は auto 指定のときに使う profile を 1 つ選ぶ。
//
// remaining quota で選びたいところだが、対応 provider のどれも公式 CLI から残量を
// 出さない（親 C1 の調査結果）。取れない値を根拠にした選択を装うより、有効な
// profile を素直に round-robin する。**選んだ結果は具体的な ID として記録される**ので、
// 後からどのセッションがどの契約を使ったかは追える。
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
	// 起動前に本人のみアクセス可の権限で用意しておく。
	if err := subscription.EnsureProfileDir(resolved.ProfileDir); err != nil {
		return nil, nil, err
	}
	env := append([]string(nil), resolved.Env...)
	// wrapper がこの値を register で申告し、Hub が「実際に何で起動したか」を記録する。
	env = append(env, subscription.SessionEnvVar+"="+resolved.ID)
	return env, resolved, nil
}
