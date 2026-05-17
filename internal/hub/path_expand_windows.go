//go:build windows

package hub

import (
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/sys/windows/registry"
)

// expandPathEntries は Windows でユーザー Path に含まれる `%VAR%` 形式の
// 未展開エントリを spawn 直前に再展開する。pnpm setup のように
// USER PATH へ `%PNPM_HOME%\bin` を書き込む方式の CLI を、Hub プロセス
// 起動時に `PNPM_HOME` が export されていなくても拾えるようにする。
//
// 展開順:
//  1. 現プロセス env (os.Getenv)
//  2. Windows registry HKCU\Environment の最新値（PNPM_HOME 等の単発取得）
//  3. `PNPM_HOME` に限り `%LOCALAPPDATA%\pnpm` を fallback（ディレクトリ実在チェック付き）
//
// いずれでも展開できなかったエントリは元のまま温存する（破壊しない）。
//
// 仕上げに以下を順に append する（既に PATH に乗っているものは大小無視で重複排除）:
//   - HKCU\Environment\Path をレジストリ生値で読み、プロセス env から脱落した
//     エントリを救済する（`%VAR%` 未解決で完全脱落するケースに加え、空エントリ
//     以降が打ち切られたケースも救う）
//   - HKLM\...\Session Manager\Environment\Path も同様に救済
//   - provider CLI の典型インストール先（pnpm / npm / scoop / .local/bin）を
//     実在チェック付きで append
//
// これにより「ターミナルで claude が動くのに Hub UI からの spawn だけ
// `exec.LookPath` が失敗する」状況を、ユーザー設定変更なしで救う。
func expandPathEntries(entries []string) []string {
	cache := map[string]string{}
	out := make([]string, 0, len(entries))
	seen := map[string]bool{}
	add := func(p string) {
		k := pathKey(p)
		if k == "" || seen[k] {
			return
		}
		out = append(out, p)
		seen[k] = true
	}
	for _, raw := range entries {
		if !strings.Contains(raw, "%") {
			add(raw)
			continue
		}
		expanded, ok := expandWinVarRefs(raw, cache)
		if !ok {
			add(raw)
			continue
		}
		add(expanded)
	}
	for _, recovered := range readRegistryPathEntries(readUserEnvFromRegistry, cache) {
		add(recovered)
	}
	for _, recovered := range readRegistryPathEntries(readMachineEnvFromRegistry, cache) {
		add(recovered)
	}
	for _, fb := range providerFallbackDirs(cache) {
		add(fb)
	}
	return out
}

// pathKey は PATH エントリの重複検出用キーを返す。大文字小文字無視＋末尾区切り
// 文字無視で、`C:\Foo\` と `c:\foo` を同一とみなす。
func pathKey(p string) string {
	trimmed := strings.TrimRight(p, `\/`)
	return strings.ToLower(trimmed)
}

// readRegistryPathEntries はレジストリ Path 値を読み、エントリごとに分割して
// `%VAR%` を展開した結果を返す。読み取り失敗・未解決エントリは黙ってスキップ。
// 重複排除や seen 管理は呼び出し側 (add クロージャ) に任せる。
func readRegistryPathEntries(read func(string) string, cache map[string]string) []string {
	raw := read("Path")
	if raw == "" {
		return nil
	}
	var out []string
	for _, entry := range strings.Split(raw, string(os.PathListSeparator)) {
		entry = strings.TrimSpace(entry)
		if entry == "" {
			continue
		}
		if strings.Contains(entry, "%") {
			expanded, ok := expandWinVarRefs(entry, cache)
			if !ok {
				continue
			}
			entry = expanded
		}
		out = append(out, entry)
	}
	return out
}

// expandWinVarRefs は `%VAR%` 参照を1パスだけ置換して返す。
// 1個でも未解決の参照が残った場合は (zero, false) を返し、
// 呼び出し側で元エントリを温存させる。
func expandWinVarRefs(s string, cache map[string]string) (string, bool) {
	var b strings.Builder
	resolvedAny := false
	for i := 0; i < len(s); {
		start := strings.IndexByte(s[i:], '%')
		if start < 0 {
			b.WriteString(s[i:])
			break
		}
		b.WriteString(s[i : i+start])
		rest := s[i+start+1:]
		end := strings.IndexByte(rest, '%')
		if end < 0 {
			b.WriteString(s[i+start:])
			break
		}
		name := rest[:end]
		if name == "" {
			b.WriteString("%%")
			i = i + start + 2
			continue
		}
		val, found := resolveWinEnvVar(name, cache)
		if !found {
			return "", false
		}
		b.WriteString(val)
		i = i + start + 1 + end + 1
		resolvedAny = true
	}
	if !resolvedAny {
		return "", false
	}
	return b.String(), true
}

func resolveWinEnvVar(name string, cache map[string]string) (string, bool) {
	key := strings.ToUpper(name)
	if v, ok := cache[key]; ok {
		return v, v != ""
	}
	if v := os.Getenv(name); v != "" {
		cache[key] = v
		return v, true
	}
	if v := readUserEnvFromRegistry(name); v != "" && !strings.Contains(v, "%") {
		cache[key] = v
		return v, true
	}
	if strings.EqualFold(name, "PNPM_HOME") {
		if la := os.Getenv("LOCALAPPDATA"); la != "" {
			candidate := filepath.Join(la, "pnpm")
			if st, err := os.Stat(candidate); err == nil && st.IsDir() {
				cache[key] = candidate
				return candidate, true
			}
		}
	}
	cache[key] = ""
	return "", false
}

// readUserEnvFromRegistry はテストから差し替え可能。
var readUserEnvFromRegistry = defaultReadUserEnvFromRegistry

func defaultReadUserEnvFromRegistry(name string) string {
	k, err := registry.OpenKey(registry.CURRENT_USER, "Environment", registry.QUERY_VALUE)
	if err != nil {
		return ""
	}
	defer k.Close()
	v, _, err := k.GetStringValue(name)
	if err != nil {
		return ""
	}
	return v
}

// readMachineEnvFromRegistry はテストから差し替え可能。
// HKLM\System\CurrentControlSet\Control\Session Manager\Environment は
// Machine 全体の環境変数（システム PATH 等）を保持する標準ロケーション。
var readMachineEnvFromRegistry = defaultReadMachineEnvFromRegistry

func defaultReadMachineEnvFromRegistry(name string) string {
	k, err := registry.OpenKey(
		registry.LOCAL_MACHINE,
		`System\CurrentControlSet\Control\Session Manager\Environment`,
		registry.QUERY_VALUE,
	)
	if err != nil {
		return ""
	}
	defer k.Close()
	v, _, err := k.GetStringValue(name)
	if err != nil {
		return ""
	}
	return v
}

// providerFallbackDirs はテストから差し替え可能。provider CLI の典型
// インストール先のうち、実在するものを返す（PATH に追加する候補）。
var providerFallbackDirs = defaultProviderFallbackDirs

// defaultProviderFallbackDirs は、claude / codex 等の Node 系 CLI が
// インストールされがちな bin ディレクトリのうち、実在するものを列挙する。
// レジストリ / プロセス PATH のどちらにも乗っていないケースを救うための
// 最終 fallback。
func defaultProviderFallbackDirs(cache map[string]string) []string {
	var candidates []string
	if la, ok := resolveWinEnvVar("LOCALAPPDATA", cache); ok {
		candidates = append(candidates, filepath.Join(la, "pnpm"))
	}
	if ap, ok := resolveWinEnvVar("APPDATA", cache); ok {
		candidates = append(candidates, filepath.Join(ap, "npm"))
	}
	if up, ok := resolveWinEnvVar("USERPROFILE", cache); ok {
		candidates = append(candidates,
			filepath.Join(up, "scoop", "shims"),
			filepath.Join(up, ".local", "bin"),
		)
	}
	var out []string
	for _, c := range candidates {
		st, err := os.Stat(c)
		if err != nil || !st.IsDir() {
			continue
		}
		out = append(out, c)
	}
	return out
}
