package hub

import (
	"encoding/json"
	"errors"
	"strings"

	"many-ai-cli/internal/config"
)

// Route 名の定数。空文字は「指定なし（既定 provider）」を意味する。
const (
	RouteAnthropic = "anthropic"
	RouteOpenAI    = "openai"
	RouteOllama    = "ollama"
	RouteLMStudio  = "lm-studio"
	RouteNVIDIANIM = "nvidia-nim"
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
	case "", RouteAnthropic, RouteOpenAI, RouteOllama, RouteLMStudio, RouteNVIDIANIM:
		return true
	default:
		return false
	}
}

var errNVIDIANIMRouteRequiresOpenCode = errors.New("NVIDIA NIM route is only available for OpenCode")
var errInvalidOpenCodeNIMConfig = errors.New("invalid OpenCode runtime config")

// validateProviderRoute rejects the hosted NIM route for providers that do not
// understand OpenCode's Chat Completions provider configuration.
func validateProviderRoute(provider, route string) error {
	if route == RouteNVIDIANIM && provider != "opencode" {
		return errNVIDIANIMRouteRequiresOpenCode
	}
	return nil
}

// parseOpenCodeNVIDIANIMModel converts OpenCode's provider/model form to the
// exact NVIDIA model ID used as the key in the runtime provider config.
func parseOpenCodeNVIDIANIMModel(model string) (string, bool) {
	model = strings.TrimSpace(model)
	const prefix = "nvidia/"
	if !strings.HasPrefix(model, prefix) {
		return "", false
	}
	modelID := strings.TrimPrefix(model, prefix)
	if modelID == "" {
		return "", false
	}
	for _, segment := range strings.Split(modelID, "/") {
		if segment == "" || segment == "." || segment == ".." {
			return "", false
		}
		for _, r := range segment {
			if (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') || r == '-' || r == '_' || r == '.' {
				continue
			}
			return "", false
		}
	}
	return modelID, true
}

// mergeOpenCodeNVIDIANIMConfig adds the NVIDIA OpenAI-compatible provider and
// selected model to OPENCODE_CONFIG_CONTENT while retaining other providers.
// Parse failures deliberately return a fixed error without echoing the source.
func mergeOpenCodeNVIDIANIMConfig(existing, modelID string) (string, error) {
	root := map[string]any{}
	if strings.TrimSpace(existing) != "" {
		if err := json.Unmarshal([]byte(existing), &root); err != nil || root == nil {
			return "", errInvalidOpenCodeNIMConfig
		}
	}

	providers, ok := root["provider"].(map[string]any)
	if !ok {
		if _, exists := root["provider"]; exists {
			return "", errInvalidOpenCodeNIMConfig
		}
		providers = map[string]any{}
	}
	nvidia, ok := providers["nvidia"].(map[string]any)
	if !ok {
		if _, exists := providers["nvidia"]; exists {
			return "", errInvalidOpenCodeNIMConfig
		}
		nvidia = map[string]any{}
	}
	options, ok := nvidia["options"].(map[string]any)
	if !ok {
		if _, exists := nvidia["options"]; exists {
			return "", errInvalidOpenCodeNIMConfig
		}
		options = map[string]any{}
	}
	models, ok := nvidia["models"].(map[string]any)
	if !ok {
		if _, exists := nvidia["models"]; exists {
			return "", errInvalidOpenCodeNIMConfig
		}
		models = map[string]any{}
	}

	nvidia["name"] = "NVIDIA NIM"
	nvidia["npm"] = "@ai-sdk/openai-compatible"
	options["baseURL"] = "https://integrate.api.nvidia.com/v1"
	options["apiKey"] = "{env:NVIDIA_API_KEY}"
	nvidia["options"] = options
	models[modelID] = map[string]any{"name": modelID}
	nvidia["models"] = models
	providers["nvidia"] = nvidia
	root["provider"] = providers

	merged, err := json.Marshal(root)
	if err != nil {
		return "", errInvalidOpenCodeNIMConfig
	}
	return string(merged), nil
}

// EnvPresetFor は provider × route の組み合わせから子プロセスへ追加注入すべき
// env 変数列を返す。`KEY=VALUE` 形式。route が空 / provider 既定の場合は nil。
//
// Ollama / LM Studio route のときのみ ANTHROPIC_BASE_URL / OPENAI_BASE_URL を
// ローカル LLM サーバー宛てに差し替える。Anthropic / OpenAI 公式接続では
// BASE_URL を注入しない（純正エンドポイントへ素通しし、Sonnet 5 以降の 1M
// コンテキスト等のクライアント側判定を阻害しないため）。
//
// 注: Ollama route では `ANTHROPIC_API_KEY=` を明示空文字で上書きしないと
// Claude Code が純正 Anthropic にフォールバックする実装がある
// （manual_ollama-cloud-routing.md 参照）。
func EnvPresetFor(provider, route string) []string {
	return EnvPresetForWithOllamaBase(provider, route, "", "")
}

func EnvPresetForWithOllamaBase(provider, route, ollamaBaseURL, lmStudioBaseURL string) []string {
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
	if provider == "opencode" && strings.HasPrefix(m, "nvidia/") {
		return RouteNVIDIANIM
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
