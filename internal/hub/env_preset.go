package hub

import (
	"strings"

	"many-ai-cli/internal/config"
)

// Route 名の定数。空文字は「指定なし（既定 provider）」を意味する。
const (
	RouteAnthropic = "anthropic"
	RouteOpenAI    = "openai"
	RouteOllama    = "ollama"
	RouteLMStudio  = "lm-studio"
)

// isLocalRoute はローカル LLM サーバーへの route かどうかを返す。
// Ollama と LM Studio が該当する。ローカル route は spawn 時に env を焼き付け、
// /model コマンドをブロックし、last_model に残さない共通挙動を持つ。
func isLocalRoute(route string) bool {
	return route == RouteOllama || route == RouteLMStudio
}

// validRoute は spawn API で受け取り得る route 値の whitelist。
func validRoute(route string) bool {
	switch route {
	case "", RouteAnthropic, RouteOpenAI, RouteOllama, RouteLMStudio:
		return true
	default:
		return false
	}
}

// EnvPresetFor は provider × route の組み合わせから子プロセスへ追加注入すべき
// env 変数列を返す。`KEY=VALUE` 形式。route が空 / provider 既定の場合は nil。
//
// proxyBaseURL が非空の場合、route が anthropic/openai/空 のときに ANTHROPIC_BASE_URL /
// OPENAI_BASE_URL を内蔵プロキシ宛て（`<proxyBaseURL>/anthropic` / `<proxyBaseURL>/openai/v1`）
// に差し替える。これにより wrap 対象 CLI の API リクエストが Hub 内プロキシ経由になり、
// payload を構造化済みチャット履歴として捕捉できる。Ollama route のときは差し替えない
// （ローカル Ollama が正本のため）。
//
// 注: Anthropic 公式接続では ANTHROPIC_API_KEY をユーザー shell の値からそのまま継承する。
// Ollama route では `ANTHROPIC_API_KEY=` を明示空文字で上書きしないと Claude Code が
// 純正 Anthropic にフォールバックする実装がある（manual_ollama-cloud-routing.md 参照）。
func EnvPresetFor(provider, route string) []string {
	return EnvPresetForProxy(provider, route, "", "")
}

// EnvPresetForProxy は EnvPresetFor のプロキシ対応版。proxyBaseURL は
// `http://127.0.0.1:<port>` 形式（末尾スラッシュなし）。空なら従来挙動。
// proxyToken が非空のとき、URL に `/s/<token>` を埋め込み、MANY_AI_CLI_PROXY_TOKEN env も付与する。
// wrapper はこの env を読んで register 時に Hub へ伝え、Hub が token → session ID を解決する。
func EnvPresetForProxy(provider, route, proxyBaseURL, proxyToken string) []string {
	return EnvPresetForProxyWithOllamaBase(provider, route, proxyBaseURL, proxyToken, "", "")
}

func EnvPresetForProxyWithOllamaBase(provider, route, proxyBaseURL, proxyToken, ollamaBaseURL, lmStudioBaseURL string) []string {
	pathPrefix := ""
	if proxyToken != "" {
		pathPrefix = "/s/" + proxyToken
	}
	ollamaBase := config.EffectiveOllamaBaseURL(ollamaBaseURL)
	lmStudioBase := config.EffectiveLMStudioBaseURL(lmStudioBaseURL)
	switch provider {
	case "claude":
		if route == RouteOllama {
			return []string{
				"ANTHROPIC_AUTH_TOKEN=ollama",
				"ANTHROPIC_API_KEY=",
				"ANTHROPIC_BASE_URL=" + ollamaBase,
			}
		}
		if route == RouteLMStudio {
			return []string{
				"ANTHROPIC_AUTH_TOKEN=lmstudio",
				"ANTHROPIC_API_KEY=",
				"ANTHROPIC_BASE_URL=" + lmStudioBase,
			}
		}
		if proxyBaseURL != "" {
			out := []string{
				"ANTHROPIC_BASE_URL=" + proxyBaseURL + pathPrefix + "/anthropic",
			}
			if proxyToken != "" {
				out = append(out, "MANY_AI_CLI_PROXY_TOKEN="+proxyToken)
			}
			return out
		}
	case "codex":
		if route == RouteOllama {
			return []string{
				"OPENAI_API_KEY=ollama",
				"OPENAI_BASE_URL=" + ollamaBase + "/v1",
			}
		}
		if route == RouteLMStudio {
			return []string{
				"OPENAI_API_KEY=lmstudio",
				"OPENAI_BASE_URL=" + lmStudioBase + "/v1",
			}
		}
		if proxyBaseURL != "" {
			out := []string{
				"OPENAI_BASE_URL=" + proxyBaseURL + pathPrefix + "/openai/v1",
			}
			if proxyToken != "" {
				out = append(out, "MANY_AI_CLI_PROXY_TOKEN="+proxyToken)
			}
			return out
		}
	}
	return nil
}

// RouteForModel は model 名と既知のモデル集合から route を推定する。
// 明示指定（API body の route フィールド）が空のときに使う。
//
// 判定優先順位:
//  1. knownLmStudio[model] が true → "lm-studio"
//  2. knownOllama[model] が true → "ollama"
//  3. model に ":cloud" を含む（":120b-cloud" 等を含む）→ "ollama"
//  4. provider == "claude" → "anthropic"
//  5. provider == "codex"  → "openai"
//  6. 上記いずれも非該当 → "" （env 注入なし）
func RouteForModel(provider, model string, knownOllama map[string]bool, knownLmStudio map[string]bool) string {
	m := strings.TrimSpace(model)
	if m == "" {
		return ""
	}
	if knownLmStudio != nil && knownLmStudio[m] {
		return RouteLMStudio
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
