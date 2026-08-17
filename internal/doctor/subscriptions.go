package doctor

import (
	"context"
	"fmt"
	"os"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// subscriptionStatusTimeout は 1 profile あたりの公式 CLI 呼び出し上限。
// profile 数だけプロセスを起動するので、1 件が詰まっても診断全体が固まらない長さにする。
const subscriptionStatusTimeout = 5 * time.Second

// subscriptions は登録済み subscription profile の診断行を返す。
//
// **profile を 1 件も登録していない環境では 1 行も返さない。** 使っていない機能で
// 診断出力が伸びると、本当に見るべき行が埋もれる（residue と同じ方針）。
//
// 出力に含めるのは profile 名・ディレクトリの有無・ログイン状態だけで、token や
// 認証ファイルの中身には触れない。
func subscriptions(ctx context.Context, cfg *config.Config) []Check {
	if cfg == nil || len(cfg.Subscriptions) == 0 {
		return nil
	}
	dir, err := config.Dir()
	if err != nil {
		return []Check{{"subscription", Warn, "サブスクリプション profile の場所を特定できません", "HOME/USERPROFILE を確認してください"}}
	}

	var checks []Check
	for _, entry := range subscription.List(cfg, dir) {
		if len(entry.Profiles) == 0 {
			continue
		}
		adapter, supported := subscription.AdapterFor(entry.Provider)
		for _, p := range entry.Profiles {
			name := profileLabel(entry.Provider, p)
			switch {
			case p.Issue != "":
				checks = append(checks, Check{"subscription", Fail,
					fmt.Sprintf("%s: 設定が壊れています（%s）", name, p.Issue),
					"~/.many-ai-cli/config.yaml の subscriptions セクションを修正してください"})
			case !supported:
				checks = append(checks, Check{"subscription", Warn,
					fmt.Sprintf("%s: この provider は profile 分離に未対応です", name),
					"セッション起動時には選べません。設定を残しても害はありません"})
			case !p.Enabled:
				checks = append(checks, Check{"subscription", OK,
					fmt.Sprintf("%s: 無効化されています", name), ""})
			default:
				checks = append(checks, subscriptionProfileCheck(ctx, adapter, name, p))
			}
		}
	}
	return checks
}

func profileLabel(provider string, p subscription.Entry) string {
	if p.Name != "" {
		return fmt.Sprintf("%s / %s", provider, p.Name)
	}
	return fmt.Sprintf("%s / %s", provider, p.ID)
}

func subscriptionProfileCheck(ctx context.Context, adapter subscription.Adapter, name string, p subscription.Entry) Check {
	if p.ProfileDir == "" {
		return Check{"subscription", Warn, name + ": profile ディレクトリを解決できません", ""}
	}
	if info, err := os.Stat(p.ProfileDir); err != nil || !info.IsDir() {
		return Check{"subscription", Warn, name + ": ログインが必要です（profile ディレクトリがまだありません）",
			"Settings のサブスクリプション欄で「ログイン」を実行してください"}
	}
	statusCtx, cancel := context.WithTimeout(ctx, subscriptionStatusTimeout)
	defer cancel()
	status, err := adapter.Status(statusCtx, p.ProfileDir)
	if err != nil {
		return Check{"subscription", Warn, fmt.Sprintf("%s: ログイン状態を確認できません（%v）", name, err),
			"対応する公式 CLI が PATH にあるか確認してください"}
	}
	if !status.LoggedIn {
		return Check{"subscription", Warn, name + ": ログインが必要です",
			"Settings のサブスクリプション欄で「ログイン」を実行してください"}
	}
	if status.Plan != "" {
		return Check{"subscription", OK, fmt.Sprintf("%s: ログイン済み（%s）", name, status.Plan), ""}
	}
	return Check{"subscription", OK, name + ": ログイン済み", ""}
}
