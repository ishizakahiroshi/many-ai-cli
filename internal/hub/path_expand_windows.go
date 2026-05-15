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
// Machine PATH の再読込はしない（副作用が大きいため USER PATH 由来の
// `%VAR%` 再展開のみに限定）。
//
// 仕上げに HKCU\Environment\Path を REG_EXPAND_SZ 生値で読み、
// プロセス起動時に `%VAR%` 未解決で **完全に脱落** したエントリも救済する。
// 一部の Windows ビルドでは未解決の `%VAR%` がプロセス env の PATH 文字列に
// literal として残らないため、プロセス PATH を見ているだけでは復元できない
// ケースが現実に存在する (例: pnpm setup 後に PNPM_HOME 未 export な GUI 親
// から立ち上げた Hub)。
func expandPathEntries(entries []string) []string {
	cache := map[string]string{}
	out := make([]string, 0, len(entries))
	seen := map[string]bool{}
	add := func(p string) {
		out = append(out, p)
		seen[strings.ToLower(p)] = true
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
	for _, recovered := range recoverDroppedUserPathEntries(seen, cache) {
		add(recovered)
	}
	return out
}

// recoverDroppedUserPathEntries は HKCU\Environment\Path を生で読み、
// `%VAR%` を含むエントリのうち展開できて、かつまだ PATH に無いものを返す。
// `%VAR%` を含まないエントリは Windows が起動時に既に PATH に乗せている
// 想定なので追加しない (重複防止)。読み取り失敗時は空。
func recoverDroppedUserPathEntries(seen map[string]bool, cache map[string]string) []string {
	raw := readUserEnvFromRegistry("Path")
	if raw == "" {
		return nil
	}
	var out []string
	for _, entry := range strings.Split(raw, string(os.PathListSeparator)) {
		entry = strings.TrimSpace(entry)
		if entry == "" || !strings.Contains(entry, "%") {
			continue
		}
		expanded, ok := expandWinVarRefs(entry, cache)
		if !ok {
			continue
		}
		if seen[strings.ToLower(expanded)] {
			continue
		}
		out = append(out, expanded)
		seen[strings.ToLower(expanded)] = true
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
