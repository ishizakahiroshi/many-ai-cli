package doctor

import (
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
	"many-ai-cli/internal/wrapper"
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
				// 同期の設定は config.yaml にしか無く、UI へ返す subscription.Entry
				// には載らない（画面から設定する項目ではないため）。同期と同じ判断で
				// 報告するには実物が要るので、ここで引き直して渡す。
				profile, _ := cfg.Subscriptions.Find(entry.Provider, p.ID)
				checks = append(checks, subscriptionProfileCheck(ctx, adapter, name, p))
				if drift := subscriptionSeedCheck(entry.Provider, name, p); drift != nil {
					checks = append(checks, *drift)
				}
				if info := subscriptionSyncOverrideCheck(name, profile); info != nil {
					checks = append(checks, *info)
				}
				if drift := subscriptionSyncDriftCheck(entry.Provider, name, p, profile); drift != nil {
					checks = append(checks, *drift)
				}
				if drift := subscriptionRuleFileCheck(entry.Provider, name, p); drift != nil {
					checks = append(checks, *drift)
				}
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

// maxReportedSeedEntries caps the drift list so one stale profile cannot push
// the rest of the diagnosis off the screen.
const maxReportedSeedEntries = 4

// subscriptionSeedCheck reports settings the profile is missing compared with
// the user's own default configuration for that CLI.
//
// This is the half of the fix that keeps working after the fact. Seeding runs
// when the Hub prepares a profile and is additive, so it cannot reach a profile
// created before seeding existed, and it cannot notice that the user added a
// skill directory to their default configuration yesterday. Without this row
// that gap is invisible: a session with no rules and no skills looks exactly
// like a session where the CLI simply chose not to use them, which is what cost
// an afternoon on 2026-08-23.
//
// It returns nil when nothing is missing, following the rule that an unused
// feature must not add lines to the report.
func subscriptionSeedCheck(provider, name string, p subscription.Entry) *Check {
	if !p.Exists || p.ProfileDir == "" {
		// Not signed in yet. The row above already says so, and seeding will
		// run when the profile is first prepared.
		return nil
	}
	pending := subscription.PendingSeedEntries(provider, p.ProfileDir)
	if len(pending) == 0 {
		return nil
	}
	labels := make([]string, 0, len(pending))
	for _, entry := range pending {
		labels = append(labels, entry.Label)
	}
	shown := labels
	suffix := ""
	if len(shown) > maxReportedSeedEntries {
		shown = shown[:maxReportedSeedEntries]
		suffix = fmt.Sprintf(" ほか %d 件", len(labels)-maxReportedSeedEntries)
	}
	return &Check{"subscription", Warn,
		fmt.Sprintf("%s: 既定の設定のうち %d 件がこの profile にありません（%s%s）",
			name, len(labels), strings.Join(shown, " / "), suffix),
		"次回このプロファイルでセッションを起動すると自動で持ち込まれます。すぐ入れたい場合は既定側から手でコピーしてください（既にあるものは上書きされません）"}
}

// subscriptionSyncDriftCheck reports the policy keys a SeedSyncFile entry and
// the user's default configuration currently disagree on: the keys the default
// has and the profile does not, and the keys both have with different values.
//
// subscriptionSeedCheck cannot see this. It asks whether the profile has the
// file at all, and after the first launch it always does — which is exactly the
// state this row is about: a settings.json carried in once, months ago, quietly
// diverging from the default ever since. The sync runs when the Hub prepares a
// profile, so what is reported here disappears at that profile's next launch;
// until then nothing else on screen says the two disagree.
//
// Message and fix name only the profile, the entry's Label and the key names —
// never a value, a path or a hash. The values behind those keys are the user's
// hooks, environment and permissions, and doctor output has to stay safe to
// paste anywhere (the same rule as subscriptionRuleFileCheck).
//
// The profile argument carries that profile's own sync settings, and they are
// applied through the very function the sync itself uses. A profile that turned
// the sync off has no drift to report — nothing is going to move — and a key it
// claimed with profile_owned_keys is its own, exactly as the adapter's built-in
// state keys are.
func subscriptionSyncDriftCheck(provider, name string, p subscription.Entry, profile config.SubscriptionProfile) *Check {
	if !p.Exists || p.ProfileDir == "" {
		return nil
	}
	adapter, ok := subscription.AdapterFor(provider)
	if !ok {
		return nil
	}
	seeder, ok := adapter.(subscription.ProfileSeeder)
	if !ok {
		return nil
	}
	for _, entry := range subscription.ApplyProfileSyncOverrides(seeder.SeedEntries(), profile) {
		if entry.Kind != subscription.SeedSyncFile {
			continue
		}
		if _, err := os.Lstat(entry.Source); err != nil {
			// No default configuration on this machine, so there is nothing to
			// disagree with.
			continue
		}
		dest := filepath.Join(p.ProfileDir, entry.Dest)
		if _, err := os.Lstat(dest); err != nil {
			// Not carried in yet; subscriptionSeedCheck already reports this.
			continue
		}
		added, changed, err := subscription.SyncDrift(entry.Source, dest, entry.StateKeys)
		if err != nil {
			// One of the two files does not parse (JSON for settings.json, TOML
			// for config.toml). The sync refuses to run in that state rather
			// than rewrite a profile from a default it could not read, so
			// saying "no drift" here would be the wrong silence.
			return &Check{"subscription", Warn,
				fmt.Sprintf("%s: %s を既定の設定と比較できません（どちらかが設定ファイルとして読めません）", name, entry.Label),
				"既定側と profile 側の設定ファイルが壊れていないか確認してください。読めない間このファイルは同期されません"}
		}
		var parts []string
		if len(added) > 0 {
			parts = append(parts, fmt.Sprintf("既定にしかない設定 %d 件（%s）", len(added), joinDriftKeys(added)))
		}
		if len(changed) > 0 {
			parts = append(parts, fmt.Sprintf("値が違う設定 %d 件（%s）", len(changed), joinDriftKeys(changed)))
		}
		if len(parts) == 0 {
			continue
		}
		return &Check{"subscription", Warn,
			fmt.Sprintf("%s: %s が既定の設定と食い違っています。%s", name, entry.Label, strings.Join(parts, " / ")),
			"次回このプロファイルでセッションを起動すると既定の値で同期されます。profile 側の値を残したい場合は、同じ変更を既定側の設定にも入れてください"}
	}
	return nil
}

// subscriptionSyncOverrideCheck states, for a profile that hand-wrote any of the
// three sync settings in config.yaml, what those settings currently mean.
//
// It exists because the other two rows go quiet when they are in force, and a
// silent row is indistinguishable from a working one. A profile with the sync
// turned off never reports drift, and a key named in profile_owned_keys never
// counts as a difference — both are correct, and both leave the user with no way
// to tell "this profile agrees with my default" apart from "this profile stopped
// being compared". One line, in the profile's own row, closes that gap.
//
// Level is OK rather than Warn: this is a setting the user wrote on purpose, not
// something to fix. (doctor has no informational level; OK is the closest.) A
// profile that never touched these settings gets no row at all, so for everyone
// else the feature does not exist. Key names only, never their values — the same
// paste-safe rule as the rows above.
func subscriptionSyncOverrideCheck(name string, profile config.SubscriptionProfile) *Check {
	if !profile.HasSyncOverrides() {
		return nil
	}
	if !profile.IsSettingsSyncEnabled() {
		// The key assignments below are dead while the sync is off, so naming
		// them here would only suggest they still do something.
		return &Check{"subscription", OK,
			fmt.Sprintf("%s: ユーザー設定の同期は off です（config.yaml の settings_sync: false。既定側の変更は届きません）", name), ""}
	}
	var parts []string
	if owned := trimmedSyncKeys(profile.ProfileOwnedKeys); len(owned) > 0 {
		parts = append(parts, fmt.Sprintf("この profile が持つ鍵: %s", joinDriftKeys(owned)))
	}
	if wins := trimmedSyncKeys(profile.DefaultWinsKeys); len(wins) > 0 {
		parts = append(parts, fmt.Sprintf("既定に揃える鍵: %s", joinDriftKeys(wins)))
	}
	if len(parts) == 0 {
		// `settings_sync: true` と書いただけ。既定と同じ意味なので行を足さない。
		return nil
	}
	return &Check{"subscription", OK, fmt.Sprintf("%s: %s", name, strings.Join(parts, " / ")), ""}
}

// trimmedSyncKeys drops blank entries, so a stray dash left in config.yaml shows
// up as nothing rather than as a key with no name.
func trimmedSyncKeys(keys []string) []string {
	out := make([]string, 0, len(keys))
	for _, key := range keys {
		if key = strings.TrimSpace(key); key != "" {
			out = append(out, key)
		}
	}
	return out
}

// joinDriftKeys renders at most maxReportedSeedEntries key names, so one badly
// drifted profile cannot push the rest of the diagnosis off the screen. Key
// names only: what they hold is the user's own configuration.
func joinDriftKeys(keys []string) string {
	if len(keys) <= maxReportedSeedEntries {
		return strings.Join(keys, ", ")
	}
	return fmt.Sprintf("%s ほか %d 件", strings.Join(keys[:maxReportedSeedEntries], ", "),
		len(keys)-maxReportedSeedEntries)
}

// subscriptionRuleFileCheck reports when a profile's rule file (CLAUDE.md /
// AGENTS.md) cannot track the user's default configuration: either because
// the default is a symlink but this profile still holds an old-style plain
// copy, or because both are plain files and their content has since diverged.
// A profile whose rule file is itself a symlink is skipped — it always reads
// whatever the default currently says, so it cannot drift.
//
// Message and fix name only the profile and the entry's Label, never file
// contents, a hash, or an absolute path — this mirrors subscriptionSeedCheck's
// rule that doctor output must stay safe to paste anywhere.
func subscriptionRuleFileCheck(provider, name string, p subscription.Entry) *Check {
	if !p.Exists || p.ProfileDir == "" {
		return nil
	}
	adapter, ok := subscription.AdapterFor(provider)
	if !ok {
		return nil
	}
	seeder, ok := adapter.(subscription.ProfileSeeder)
	if !ok {
		return nil
	}
	for _, entry := range seeder.SeedEntries() {
		if entry.Kind != subscription.SeedMirrorFile {
			continue
		}
		dest := filepath.Join(p.ProfileDir, entry.Dest)
		destInfo, err := os.Lstat(dest)
		if err != nil {
			// Not seeded yet; subscriptionSeedCheck already reports this.
			continue
		}
		if destInfo.Mode()&os.ModeSymlink != 0 {
			// The profile mirrors the default as a link; it cannot drift.
			continue
		}
		if subscription.IsLinkedRuleFile(entry.Source) {
			return &Check{"subscription", Warn,
				fmt.Sprintf("%s: %s は既定側がリンクなのに profile はコピーです。既定側の変更が届きません", name, entry.Label),
				fmt.Sprintf("profile 側の %s を削除して次回起動すると、リンクとして持ち込まれます。profile 専用の内容にしたい場合はこのままで構いません", entry.Dest)}
		}
		srcInfo, err := os.Stat(entry.Source)
		if err != nil || srcInfo.IsDir() {
			continue
		}
		same, err := ruleFilesIdentical(entry.Source, dest)
		if err != nil || same {
			continue
		}
		return &Check{"subscription", Warn,
			fmt.Sprintf("%s: %s は既定側と内容が違います", name, entry.Label),
			fmt.Sprintf("意図した差分でなければ profile 側の %s を削除して次回起動で再取得。意図した差分ならこのままで構いません", entry.Dest)}
	}
	return nil
}

// ruleFilesIdentical compares two rule files by content hash, never by
// surfacing either file's bytes in a doctor message.
func ruleFilesIdentical(a, b string) (bool, error) {
	ah, err := sha256File(a)
	if err != nil {
		return false, err
	}
	bh, err := sha256File(b)
	if err != nil {
		return false, err
	}
	return ah == bh, nil
}

// sha256File hashes a rule file after stripping the blocks many-ai-cli itself
// injects (shared approval-rules block, its legacy any-ai-cli name, and the
// delegation block). Without this, a session that is currently running
// injects one of these blocks into the profile's copy but never into the
// default side, and ruleFilesIdentical would report a false "内容が違います"
// for the whole time that session is alive (C5,
// plan_subscription-claude-md-symlink-seed.md).
func sha256File(path string) ([32]byte, error) {
	data, err := os.ReadFile(path) // #nosec G304 -- path comes from an adapter's fixed entry list or a profile dir this process manages
	if err != nil {
		return [32]byte{}, err
	}
	return sha256.Sum256(wrapper.StripInjectedBlocks(data)), nil
}
