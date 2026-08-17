package subscription

import (
	"errors"
	"fmt"
	"os"
	"sort"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/securefile"
)

var (
	// ErrProfileNotFound は指定 ID の profile が config に無い場合。
	ErrProfileNotFound = errors.New("subscription profile not found")
	// ErrProfileDisabled は profile が enabled: false の場合。
	ErrProfileDisabled = errors.New("subscription profile is disabled")
	// ErrProviderUnsupported は adapter 未登録の provider を指定された場合。
	ErrProviderUnsupported = errors.New("provider does not support subscription profiles")
	// ErrNoSelectableProfile は auto 指定なのに選べる profile が 1 つも無い場合。
	ErrNoSelectableProfile = errors.New("no enabled subscription profile to choose from")
)

// Selectable は auto 選択の候補、つまり有効かつ設定が壊れていない profile の ID を
// config の並び順で返す。並び順を保つのは round-robin の再現性のため。
func Selectable(cfg *config.Config, provider string) []string {
	if cfg == nil {
		return nil
	}
	if _, ok := AdapterFor(provider); !ok {
		return nil
	}
	seen := map[string]bool{}
	out := make([]string, 0, len(cfg.Subscriptions[provider]))
	for _, p := range cfg.Subscriptions[provider] {
		id := config.NormalizeSubscriptionID(p.ID)
		if err := config.ValidateSubscriptionID(id); err != nil || seen[id] || !p.IsEnabled() {
			continue
		}
		seen[id] = true
		out = append(out, id)
	}
	return out
}

// Resolved はセッション起動に必要な profile の確定情報。
type Resolved struct {
	Provider   string
	ID         string
	Name       string
	Plan       string
	ProfileDir string
	EnvVar     string
	// Env は子プロセス env に重ねる KEY=VALUE 列。
	Env []string
}

// Resolve は spawn 要求 1 件分の profile を確定する。
//
// profileID が空なら (nil, nil) を返す。これが「従来どおり CLI 自身のログイン環境を
// 使う」既定動作で、この経路では env に 1 バイトも変化を加えない。
//
// **見つからない / 無効化されている profile を黙って別 profile や既定ログインへ
// フォールバックさせない。** 誤ったアカウントで実行されることの方が、起動できない
// ことより重い（親 plan の判断原則）。
func Resolve(cfg *config.Config, configDir, provider, profileID string) (*Resolved, error) {
	id := config.NormalizeSubscriptionID(profileID)
	if id == "" {
		return nil, nil
	}
	if cfg == nil {
		return nil, fmt.Errorf("%w: %s/%s", ErrProfileNotFound, provider, id)
	}
	adapter, ok := AdapterFor(provider)
	if !ok {
		return nil, fmt.Errorf("%w: %s", ErrProviderUnsupported, provider)
	}
	profile, found := cfg.Subscriptions.Find(provider, id)
	if !found {
		return nil, fmt.Errorf("%w: %s/%s", ErrProfileNotFound, provider, id)
	}
	if !profile.IsEnabled() {
		return nil, fmt.Errorf("%w: %s/%s", ErrProfileDisabled, provider, id)
	}
	dir, err := config.ResolveSubscriptionProfileDir(configDir, provider, profile)
	if err != nil {
		return nil, err
	}
	return &Resolved{
		Provider:   provider,
		ID:         id,
		Name:       profile.Name,
		Plan:       profile.Plan,
		ProfileDir: dir,
		EnvVar:     adapter.EnvVar(),
		Env:        adapter.LaunchEnv(dir),
	}, nil
}

// EnsureProfileDir は profile ディレクトリを本人のみアクセス可の権限で用意する。
// 既にあれば権限だけ締め直す。
func EnsureProfileDir(dir string) error {
	if dir == "" {
		return errors.New("profile dir is empty")
	}
	if err := os.MkdirAll(dir, config.DirMode); err != nil {
		return fmt.Errorf("create profile dir: %w", err)
	}
	if err := os.Chmod(dir, config.DirMode); err != nil {
		return fmt.Errorf("chmod profile dir: %w", err)
	}
	// Windows は Chmod が DACL を狭めないので、継承 ACE を明示的に切る。
	// 失敗しても作成自体は成功しているため呼び出し元は成功として扱う
	// （config.ensurePrivateDir と同じ扱い）。
	_ = securefile.EnsurePrivateDir(dir)
	return nil
}

// Entry は UI へ返す profile 1 件。**secret を含まない。**
type Entry struct {
	Provider   string `json:"provider"`
	ID         string `json:"id"`
	Name       string `json:"name,omitempty"`
	Plan       string `json:"plan,omitempty"`
	Enabled    bool   `json:"enabled"`
	ProfileDir string `json:"profile_dir,omitempty"`
	// Exists は profile ディレクトリが実在するか。false は「まだログインしていない」
	// ことを示唆するが、判定そのものは Status（Test ボタン）が行う。
	Exists bool `json:"exists"`
	// Issue は設定が壊れていて選択できない理由（空なら選択可能）。
	Issue string `json:"issue,omitempty"`
}

// ProviderEntry は provider 1 つ分のセクション。
type ProviderEntry struct {
	Provider string `json:"provider"`
	// Supported が false のとき、その provider は profile 分離に未対応
	// （adapter が無い）。UI は理由付きの無効表示にする。
	Supported bool    `json:"supported"`
	EnvVar    string  `json:"env_var,omitempty"`
	Profiles  []Entry `json:"profiles"`
}

// List は UI 表示用に profile 一覧を組み立てる。
//
// 壊れた設定でもエラーを返さない: 直せるように画面へ出すのが目的で、ここで
// 失敗すると Settings 画面ごと開けなくなる。
func List(cfg *config.Config, configDir string) []ProviderEntry {
	providers := SupportedProviders()
	known := make(map[string]bool, len(providers))
	for _, p := range providers {
		known[p] = true
	}
	if cfg != nil {
		extra := make([]string, 0, len(cfg.Subscriptions))
		for provider := range cfg.Subscriptions {
			if !known[provider] {
				extra = append(extra, provider)
				known[provider] = true
			}
		}
		sort.Strings(extra)
		providers = append(providers, extra...)
	}

	out := make([]ProviderEntry, 0, len(providers))
	for _, provider := range providers {
		adapter, supported := AdapterFor(provider)
		entry := ProviderEntry{Provider: provider, Supported: supported, Profiles: []Entry{}}
		if supported {
			entry.EnvVar = adapter.EnvVar()
		}
		if cfg == nil {
			out = append(out, entry)
			continue
		}
		seen := map[string]bool{}
		for _, p := range cfg.Subscriptions[provider] {
			id := config.NormalizeSubscriptionID(p.ID)
			item := Entry{
				Provider: provider,
				ID:       id,
				Name:     p.Name,
				Plan:     p.Plan,
				Enabled:  p.IsEnabled(),
			}
			switch {
			case config.ValidateSubscriptionProvider(provider) != nil:
				item.Issue = config.ValidateSubscriptionProvider(provider).Error()
			case config.ValidateSubscriptionID(id) != nil:
				item.ID = p.ID
				item.Issue = config.ValidateSubscriptionID(id).Error()
			case seen[id]:
				item.Issue = fmt.Sprintf("duplicate profile id %q", id)
			default:
				seen[id] = true
				dir, err := config.ResolveSubscriptionProfileDir(configDir, provider, p)
				if err != nil {
					item.Issue = err.Error()
					break
				}
				item.ProfileDir = dir
				if info, statErr := os.Stat(dir); statErr == nil && info.IsDir() {
					item.Exists = true
				}
			}
			entry.Profiles = append(entry.Profiles, item)
		}
		out = append(out, entry)
	}
	return out
}
