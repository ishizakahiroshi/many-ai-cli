package hub

import "strings"

// Route 名の定数。空文字は「指定なし（既定 provider）」を意味する。
const (
	RouteAnthropic = "anthropic"
	RouteOpenAI    = "openai"
	RouteOllama    = "ollama"
)

// validRoute は spawn API で受け取り得る route 値の whitelist。
func validRoute(route string) bool {
	switch route {
	case "", RouteAnthropic, RouteOpenAI, RouteOllama:
		return true
	default:
		return false
	}
}

// EnvPresetFor は provider × route の組み合わせから子プロセスへ追加注入すべき
// env 変数列を返す。`KEY=VALUE` 形式。route が空 / provider 既定の場合は nil。
//
// 注: Anthropic 公式接続では ANTHROPIC_API_KEY をユーザー shell の値からそのまま継承する。
// Ollama route では `ANTHROPIC_API_KEY=` を明示空文字で上書きしないと Claude Code が
// 純正 Anthropic にフォールバックする実装がある（manual_ollama-cloud-routing.md 参照）。
func EnvPresetFor(provider, route string) []string {
	switch provider {
	case "claude":
		if route == RouteOllama {
			return []string{
				"ANTHROPIC_AUTH_TOKEN=ollama",
				"ANTHROPIC_API_KEY=",
				"ANTHROPIC_BASE_URL=http://localhost:11434",
			}
		}
	case "codex":
		if route == RouteOllama {
			return []string{
				"OPENAI_API_KEY=ollama",
				"OPENAI_BASE_URL=http://localhost:11434/v1",
			}
		}
	}
	return nil
}

// RouteForModel は model 名と既知の Ollama モデル集合から route を推定する。
// 明示指定（API body の route フィールド）が空のときに使う。
//
// 判定優先順位:
//  1. knownOllama[model] が true → "ollama"
//  2. model に ":cloud" を含む（":120b-cloud" 等を含む）→ "ollama"
//  3. provider == "claude" → "anthropic"
//  4. provider == "codex"  → "openai"
//  5. 上記いずれも非該当 → "" （env 注入なし）
func RouteForModel(provider, model string, knownOllama map[string]bool) string {
	m := strings.TrimSpace(model)
	if m == "" {
		return ""
	}
	if knownOllama != nil && knownOllama[m] {
		return RouteOllama
	}
	if strings.Contains(m, ":cloud") {
		return RouteOllama
	}
	switch provider {
	case "claude":
		return RouteAnthropic
	case "codex":
		return RouteOpenAI
	}
	return ""
}

// envKeyList は `KEY=VALUE` 形式の env 列から KEY だけ抜いた slice を返す。
// ログ出力で値を漏らさないために使う。
func envKeyList(env []string) []string {
	out := make([]string, 0, len(env))
	for _, kv := range env {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		out = append(out, kv[:eq])
	}
	return out
}

// mergeEnvOverrides は既存 env 列に追加 env を merge する。同名キーがあれば
// 後勝ち（追加側で上書き）。`KEY=VALUE` の形式以外はそのまま末尾に付ける。
func mergeEnvOverrides(base, overrides []string) []string {
	if len(overrides) == 0 {
		return base
	}
	keyIdx := map[string]int{}
	out := make([]string, 0, len(base)+len(overrides))
	for _, kv := range base {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		key := kv[:eq]
		keyIdx[key] = len(out)
		out = append(out, kv)
	}
	for _, kv := range overrides {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		key := kv[:eq]
		if i, ok := keyIdx[key]; ok {
			out[i] = kv
		} else {
			keyIdx[key] = len(out)
			out = append(out, kv)
		}
	}
	return out
}
