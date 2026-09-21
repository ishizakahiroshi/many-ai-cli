package config

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"

	"gopkg.in/yaml.v3"
)

// SubscriptionsDirName は ~/.many-ai-cli 配下で profile 実体を置くディレクトリ名。
const SubscriptionsDirName = "subscriptions"

// MaxSubscriptionIDLen は profile ID の最大長。ファイル名に使うため短く抑える。
const MaxSubscriptionIDLen = 64

// SubscriptionAutoID は spawn 時に「有効な profile から 1 つ選ぶ」を意味する予約語。
// 実在する profile ID として登録できないようにしておかないと、"auto" という名前の
// profile を作った利用者が二度とその profile を明示指定できなくなる。
const SubscriptionAutoID = "auto"

// subscriptionIDRe は profile ID として許可する文字集合。
// 小文字・数字始まりに限定し、以降は `.` `_` `-` を許す。大文字を許さないのは、
// NTFS が大小を区別しない一方で ext4 は区別するため、同じ config が OS によって
// 別ディレクトリを指す状態を作らないため（ID は保存前に必ず小文字化する）。
var subscriptionIDRe = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]*$`)

// SubscriptionProfile は config.yaml に保存する「契約 1 件」のメタデータ。
//
// **secret を持たせない。** access token / refresh token / cookie の類は一切
// このフィールドに現れてはならず、認証実体は ProfileDir の中で vendor 公式 CLI
// 自身が管理する。many-ai-cli はディレクトリを指し示すだけで中身に触らない。
type SubscriptionProfile struct {
	ID   string `yaml:"id" json:"id"`
	Name string `yaml:"name,omitempty" json:"name,omitempty"`
	// Plan は表示用の契約名（"max" / "pro" 等）。公式 CLI から取得できた場合に
	// 記録する。取得できない provider では空のままでよい。
	Plan string `yaml:"plan,omitempty" json:"plan,omitempty"`
	// Enabled はポインタ。nil（未設定）は有効として扱う。明示的な false を
	// omitempty で落とさないための三値表現（UserPrefsTokenStatusbar と同じ理由）。
	Enabled *bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
	// ProfileDir は既定位置（~/.many-ai-cli/subscriptions/<provider>/<id>）以外へ
	// 実体を置きたいときだけ手で書く上書き。API からは設定できない。
	ProfileDir string `yaml:"profile_dir,omitempty" json:"profile_dir,omitempty"`

	// 以下 3 項目は起動時同期（settings.json を既定設定と揃える動作）の profile ごとの
	// 上書き。**ProfileDir と同じく手書き専用で、Hub UI / API からは設定しない。**
	// 標準は「既定が正典・CLI が書く鍵は profile のもの」で、それで足りない利用者
	// （profile ごとに plugin の組み合わせを変える、profile の設定をわざと既定から
	// 離している）のための逃げ道として config.yaml にだけ口を開ける。API 側は
	// 名前・有効フラグ・plan を書き換えるときもこの 3 項目に触れないので、画面から
	// の操作で手書きの取り決めが消えることはない。

	// SettingsSync は起動時同期を行うかどうか。nil（未設定）と true は同期する。
	// false にすると、その profile の同期対象ファイルは「無ければコピー・あれば
	// 触らない」という従来どおりの持ち込みに戻る。
	//
	// ポインタなのは Enabled と同じ理由（明示的な false を omitempty で落とさない）。
	SettingsSync *bool `yaml:"settings_sync,omitempty" json:"settings_sync,omitempty"`
	// ProfileOwnedKeys は、この profile が自分で持つ設定鍵を adapter の一覧へ足す。
	// 足した鍵は既定側の値で上書きされず、doctor も食い違いとして数えない。
	// plugin の組み合わせを profile ごとに変えたい場合の `enabledPlugins` が実例。
	ProfileOwnedKeys []string `yaml:"profile_owned_keys,omitempty" json:"profile_owned_keys,omitempty"`
	// DefaultWinsKeys は逆に、adapter が profile の持ち物として扱っている鍵を既定側が
	// 勝つ側へ戻す。全 profile で `theme` を既定に揃えたい場合が実例。同じ鍵を両方へ
	// 書いたときは DefaultWinsKeys が勝つ（「既定に揃える」と明示した側を優先する）。
	DefaultWinsKeys []string `yaml:"default_wins_keys,omitempty" json:"default_wins_keys,omitempty"`
}

// IsEnabled は Enabled が nil（未設定）または true のとき true を返す。
func (p SubscriptionProfile) IsEnabled() bool {
	return p.Enabled == nil || *p.Enabled
}

// IsSettingsSyncEnabled は SettingsSync が nil（未設定）または true のとき true を返す。
func (p SubscriptionProfile) IsSettingsSyncEnabled() bool {
	return p.SettingsSync == nil || *p.SettingsSync
}

// HasSyncOverrides は同期の既定から外れる設定が 1 つでも書かれているかを返す。
//
// `many-ai-cli doctor` はこれが true の profile にだけ 1 行足す。既定のままの利用者に
// とって、この機能は存在しないのと同じでなければならない（使っていない機能で診断出力
// が伸びると、本当に見るべき行が埋もれる）。
func (p SubscriptionProfile) HasSyncOverrides() bool {
	return p.SettingsSync != nil || len(p.ProfileOwnedKeys) > 0 || len(p.DefaultWinsKeys) > 0
}

// SubscriptionProfiles は provider 名 → profile 一覧。map なので、未知の provider
// キーがあっても構造体フィールドの増減と違って読み込みが壊れない。
type SubscriptionProfiles map[string][]SubscriptionProfile

// UnmarshalYAML は壊れた `subscriptions:` を config 全体の破損として扱わない。
//
// LoadOrCreate は yaml.Unmarshal がエラーを返すと config.yaml 全体を .bak へ
// 退避してデフォルト再生成する（token も作り直しになり Hub URL が変わる）。
// 手書きされうるセクションで 1 項目の型違いがその代償を払うのは割に合わないため、
// SessionOrderIDs と同じく寛容にデコードし、解釈できない要素だけ捨てる。
func (s *SubscriptionProfiles) UnmarshalYAML(value *yaml.Node) error {
	var raw map[string]yaml.Node
	if err := value.Decode(&raw); err != nil {
		*s = nil
		return nil
	}
	if len(raw) == 0 {
		*s = nil
		return nil
	}
	out := make(SubscriptionProfiles, len(raw))
	for provider, node := range raw {
		var list []SubscriptionProfile
		if err := node.Decode(&list); err != nil {
			// yaml.v3 は項目単位の型違いを *yaml.TypeError にまとめて返し、解釈でき
			// た項目はそのまま out へ入れる（要素ごと解釈できなかったものは列から
			// 落ちる）。1 項目の型違いで profile を丸ごと捨てると、`profile_owned_keys`
			// を書き間違えただけでその profile が一覧から消え、spawn 画面で選べなく
			// なる＝別のアカウントで起動することになる。型違いだけは許容して、解釈
			// できた項目を残す。それ以外のエラー（列そのものが map や scalar）は
			// 従来どおり provider ごと捨てる。
			var typeErr *yaml.TypeError
			if !errors.As(err, &typeErr) {
				continue
			}
		}
		kept := make([]SubscriptionProfile, 0, len(list))
		for _, p := range list {
			if strings.TrimSpace(p.ID) == "" {
				continue
			}
			kept = append(kept, p)
		}
		if len(kept) > 0 {
			out[provider] = kept
		}
	}
	if len(out) == 0 {
		*s = nil
		return nil
	}
	*s = out
	return nil
}

// Clone returns a deep copy so callers can mutate or marshal it without racing
// with the live server config.
func (s SubscriptionProfiles) Clone() SubscriptionProfiles {
	if s == nil {
		return nil
	}
	out := make(SubscriptionProfiles, len(s))
	for provider, list := range s {
		copied := make([]SubscriptionProfile, len(list))
		copy(copied, list)
		for i := range copied {
			if list[i].Enabled != nil {
				v := *list[i].Enabled
				copied[i].Enabled = &v
			}
			if list[i].SettingsSync != nil {
				v := *list[i].SettingsSync
				copied[i].SettingsSync = &v
			}
			copied[i].ProfileOwnedKeys = slices.Clone(list[i].ProfileOwnedKeys)
			copied[i].DefaultWinsKeys = slices.Clone(list[i].DefaultWinsKeys)
		}
		out[provider] = copied
	}
	return out
}

// Find は provider + id の profile を返す。id は正規化してから比較する。
func (s SubscriptionProfiles) Find(provider, id string) (SubscriptionProfile, bool) {
	want := NormalizeSubscriptionID(id)
	if want == "" {
		return SubscriptionProfile{}, false
	}
	for _, p := range s[provider] {
		if NormalizeSubscriptionID(p.ID) == want {
			return p, true
		}
	}
	return SubscriptionProfile{}, false
}

// NormalizeSubscriptionID は入力を保存・比較に使う正規形へ寄せる。
func NormalizeSubscriptionID(raw string) string {
	return strings.ToLower(strings.TrimSpace(raw))
}

// ValidateSubscriptionID は profile ID をファイル名として信用してよいか検証する。
// 正規化済みの値を渡すこと。
func ValidateSubscriptionID(id string) error {
	if id == "" {
		return fmt.Errorf("subscription id is required")
	}
	if len(id) > MaxSubscriptionIDLen {
		return fmt.Errorf("subscription id is longer than %d characters", MaxSubscriptionIDLen)
	}
	if !subscriptionIDRe.MatchString(id) {
		return fmt.Errorf("subscription id %q may only contain lowercase letters, digits, dot, underscore and hyphen, and must start with a letter or digit", id)
	}
	// 正規表現は `.` を許すため "a..b" や "..x" 相当が通る。パス要素として
	// 連結する前にここで落とす。
	if strings.Contains(id, "..") {
		return fmt.Errorf("subscription id %q must not contain a %q sequence", id, "..")
	}
	if id == SubscriptionAutoID {
		return fmt.Errorf("subscription id %q is reserved for automatic selection", id)
	}
	return nil
}

// ValidateSubscriptionProvider は config の provider キーがパス要素として安全かを検証する。
func ValidateSubscriptionProvider(provider string) error {
	if strings.TrimSpace(provider) == "" {
		return fmt.Errorf("subscription provider is required")
	}
	if provider != strings.ToLower(provider) {
		return fmt.Errorf("subscription provider %q must be lowercase", provider)
	}
	if !subscriptionIDRe.MatchString(provider) || strings.Contains(provider, "..") {
		return fmt.Errorf("subscription provider %q is not a valid provider key", provider)
	}
	return nil
}

// SubscriptionsRoot は profile 実体の親ディレクトリ（<dir>/subscriptions）を返す。
func SubscriptionsRoot(dir string) string {
	return filepath.Join(dir, SubscriptionsDirName)
}

// DefaultSubscriptionProfileDir は既定の profile ディレクトリを返す。
// 呼び出し前に ID / provider を検証すること（ResolveSubscriptionProfileDir 推奨）。
func DefaultSubscriptionProfileDir(dir, provider, id string) string {
	return filepath.Join(SubscriptionsRoot(dir), provider, id)
}

// ResolveSubscriptionProfileDir は profile の実ディレクトリを決定する。
//
// profile ID を filesystem path として直接信用せず、正規化・文字種検証を通してから
// join し、join 後に <root>/<provider> の外へ出ていないことを再確認する
// （`..` や絶対パスによる脱出の二重防御）。
func ResolveSubscriptionProfileDir(dir, provider string, p SubscriptionProfile) (string, error) {
	if err := ValidateSubscriptionProvider(provider); err != nil {
		return "", err
	}
	id := NormalizeSubscriptionID(p.ID)
	if err := ValidateSubscriptionID(id); err != nil {
		return "", err
	}
	if custom := strings.TrimSpace(p.ProfileDir); custom != "" {
		expanded, err := expandSubscriptionHome(custom)
		if err != nil {
			return "", err
		}
		if !filepath.IsAbs(expanded) {
			return "", fmt.Errorf("subscription profile_dir %q must be an absolute path", p.ProfileDir)
		}
		return filepath.Clean(expanded), nil
	}
	base := filepath.Join(SubscriptionsRoot(dir), provider)
	resolved := filepath.Clean(filepath.Join(base, id))
	if !pathWithin(base, resolved) {
		return "", fmt.Errorf("subscription profile dir for %q escapes %s", id, base)
	}
	return resolved, nil
}

// expandSubscriptionHome expands a leading ~ so hand-written config paths behave
// the way the YAML example in the docs suggests.
func expandSubscriptionHome(path string) (string, error) {
	if path != "~" && !strings.HasPrefix(path, "~/") && !strings.HasPrefix(path, `~\`) {
		return path, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("expand ~ in profile_dir: %w", err)
	}
	if path == "~" {
		return home, nil
	}
	return filepath.Join(home, path[2:]), nil
}

// pathWithin reports whether target is strictly inside base.
func pathWithin(base, target string) bool {
	rel, err := filepath.Rel(filepath.Clean(base), filepath.Clean(target))
	if err != nil {
		return false
	}
	if rel == "." || rel == ".." || filepath.IsAbs(rel) {
		return false
	}
	return !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// subscriptionWarnings は「起動は止めないが利用者に伝えるべき」subscription 設定の
// 問題を返す。Validate() 側（＝致命的で起動を止める）には入れない: 任意機能の
// 設定不備で serve / wrap / version まで落とさないため（validateOllama と同じ判断）。
func (cfg *Config) subscriptionWarnings() []string {
	if cfg == nil || len(cfg.Subscriptions) == 0 {
		return nil
	}
	providers := make([]string, 0, len(cfg.Subscriptions))
	for provider := range cfg.Subscriptions {
		providers = append(providers, provider)
	}
	slices.Sort(providers)

	var warnings []string
	for _, provider := range providers {
		if err := ValidateSubscriptionProvider(provider); err != nil {
			warnings = append(warnings, fmt.Sprintf("subscriptions: %v; its profiles are ignored", err))
			continue
		}
		seen := map[string]bool{}
		for _, p := range cfg.Subscriptions[provider] {
			id := NormalizeSubscriptionID(p.ID)
			if err := ValidateSubscriptionID(id); err != nil {
				warnings = append(warnings, fmt.Sprintf("subscriptions.%s: %v; this profile cannot be selected", provider, err))
				continue
			}
			if seen[id] {
				warnings = append(warnings, fmt.Sprintf("subscriptions.%s: duplicate profile id %q; only the first entry is used", provider, id))
				continue
			}
			seen[id] = true
			if custom := strings.TrimSpace(p.ProfileDir); custom != "" {
				if expanded, err := expandSubscriptionHome(custom); err != nil || !filepath.IsAbs(expanded) {
					warnings = append(warnings, fmt.Sprintf("subscriptions.%s.%s: profile_dir %q must be an absolute path; this profile cannot be selected", provider, id, custom))
				}
			}
		}
	}
	return warnings
}
