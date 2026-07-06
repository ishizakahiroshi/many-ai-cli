package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"time"
)

// modelsDefaults は GitHub から取得する resources/models/defaults.json のスキーマ。
// 形式: {"anthropic": [{"id":"...", "label":"..."}, ...], "openai": [...], "copilot": [...]}
type modelsDefaults struct {
	Anthropic []Model `json:"anthropic"`
	OpenAI    []Model `json:"openai"`
	// Copilot は GitHub Copilot CLI の `--model` に渡す候補。route/env 注入は行わず、
	// 対応可否は Copilot CLI 側に委ねる（slug が外れても datalist の自由入力で補える）。
	Copilot []Model `json:"copilot"`
	// CursorAgent は cursor-agent CLI の `--model` に渡す候補。route/env 注入は行わず、
	// 対応可否は cursor-agent 側に委ねる（全 ID は `cursor-agent --list-models` 参照、datalist で自由入力可）。
	CursorAgent []Model `json:"cursor-agent"`
}

const (
	modelsRemoteCacheTTL    = 24 * time.Hour
	modelsRemoteNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

// hardcodedModelsDefaults は GitHub fetch 失敗時のフォールバック値。
// resources/models/defaults.json と同じ値を保つ。
var hardcodedModelsDefaults = modelsDefaults{
	Anthropic: []Model{
		{ID: "claude-opus-4-8", Label: "Claude Opus 4.8"},
		{ID: "claude-opus-4-7", Label: "Claude Opus 4.7"},
		{ID: "claude-sonnet-4-6", Label: "Claude Sonnet 4.6"},
		{ID: "claude-haiku-4-5", Label: "Claude Haiku 4.5"},
		{ID: "claude-sonnet-4-5", Label: "Claude Sonnet 4.5"},
		{ID: "claude-3-7-sonnet-latest", Label: "Claude 3.7 Sonnet"},
		{ID: "claude-3-5-sonnet-latest", Label: "Claude 3.5 Sonnet"},
	},
	OpenAI: []Model{
		{ID: "gpt-5.5", Label: "GPT-5.5"},
		{ID: "gpt-5.4", Label: "GPT-5.4"},
		{ID: "gpt-5.4-mini", Label: "GPT-5.4 mini"},
		{ID: "gpt-5.4-nano", Label: "GPT-5.4 nano"},
	},
	// GitHub Copilot CLI の `--model` で選べるモデル（GA ファミリ）。
	// slug 規則は実機ログ（claude-sonnet-4 / gpt-5.5 / claude-haiku-4.5 等）で確認。
	// Gemini 系は標準 slug を踏襲した推定。Raptor mini は slug 不確実のため除外。
	Copilot: []Model{
		{ID: "claude-sonnet-4.6", Label: "Claude Sonnet 4.6"},
		{ID: "claude-sonnet-4.5", Label: "Claude Sonnet 4.5"},
		{ID: "claude-opus-4.8", Label: "Claude Opus 4.8"},
		{ID: "claude-opus-4.7", Label: "Claude Opus 4.7"},
		{ID: "claude-opus-4.6", Label: "Claude Opus 4.6"},
		{ID: "claude-haiku-4.5", Label: "Claude Haiku 4.5"},
		{ID: "gpt-5.5", Label: "GPT-5.5"},
		{ID: "gpt-5.4", Label: "GPT-5.4"},
		{ID: "gpt-5.4-mini", Label: "GPT-5.4 mini"},
		{ID: "gpt-5.3-codex", Label: "GPT-5.3-Codex"},
		{ID: "gpt-5-mini", Label: "GPT-5 mini"},
		{ID: "gemini-3.5-flash", Label: "Gemini 3.5 Flash"},
		{ID: "gemini-3.1-pro", Label: "Gemini 3.1 Pro"},
		{ID: "gemini-3-flash", Label: "Gemini 3 Flash"},
		{ID: "gemini-2.5-pro", Label: "Gemini 2.5 Pro"},
	},
	// cursor-agent CLI の `--model` で選べるモデルの代表セット。
	// ID/label は実機 `cursor-agent --list-models`（v2026.05.28）で確認した正本値。
	// effort/thinking/fast の全 permutation（約135件）は出さず主要モデルのみ。残りは datalist 自由入力で渡せる。
	CursorAgent: []Model{
		{ID: "auto", Label: "Auto"},
		{ID: "composer-2.5", Label: "Composer 2.5"},
		{ID: "composer-2.5-fast", Label: "Composer 2.5 Fast"},
		{ID: "claude-opus-4-8-high", Label: "Opus 4.8 1M"},
		{ID: "claude-opus-4-8-thinking-high", Label: "Opus 4.8 1M Thinking"},
		{ID: "claude-opus-4-7-xhigh", Label: "Opus 4.7 1M"},
		{ID: "claude-4.6-opus-high", Label: "Opus 4.6 1M"},
		{ID: "claude-4.5-opus-high", Label: "Opus 4.5"},
		{ID: "claude-4.6-sonnet-medium", Label: "Sonnet 4.6 1M"},
		{ID: "claude-4.5-sonnet", Label: "Sonnet 4.5"},
		{ID: "claude-4-sonnet", Label: "Sonnet 4"},
		{ID: "gpt-5.5-medium", Label: "GPT-5.5 1M"},
		{ID: "gpt-5.4-medium", Label: "GPT-5.4 1M"},
		{ID: "gpt-5.3-codex", Label: "Codex 5.3"},
		{ID: "gpt-5.2", Label: "GPT-5.2"},
		{ID: "gpt-5.1", Label: "GPT-5.1"},
		{ID: "gpt-5-mini", Label: "GPT-5 Mini"},
		{ID: "gemini-3.1-pro", Label: "Gemini 3.1 Pro"},
		{ID: "gemini-3.5-flash", Label: "Gemini 3.5 Flash"},
		{ID: "grok-4.3", Label: "Grok 4.3 1M"},
		{ID: "kimi-k2.5", Label: "Kimi K2.5"},
	},
}

// newModelsRemoteCache は GitHub のモデル defaults を扱う TTL キャッシュを生成する。
// fetch 成功値は格納前にハードコード値とマージする（リモートに無い既知モデルを補う）。
func newModelsRemoteCache() *ttlCache[modelsDefaults] {
	return &ttlCache[modelsDefaults]{
		ttl:         modelsRemoteCacheTTL,
		negativeTTL: modelsRemoteNegativeTTL,
		fallback:    hardcodedModelsDefaults,
		fetch:       fetchModelsDefaults,
		transform: func(fetched modelsDefaults) modelsDefaults {
			return mergeModelsDefaults(fetched, hardcodedModelsDefaults)
		},
	}
}

func fetchModelsDefaults(sourceURL string) (modelsDefaults, error) {
	client := makeExternalHTTPClient(10 * time.Second)
	resp, err := client.Get(sourceURL)
	if err != nil {
		return modelsDefaults{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// エラーページを読み捨てて早期リターン（ボディは読まない）
		return modelsDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 256*1024))
	if err != nil {
		return modelsDefaults{}, err
	}
	var d modelsDefaults
	if err := json.Unmarshal(body, &d); err != nil {
		return modelsDefaults{}, err
	}
	return d, nil
}

func mergeModelsDefaults(preferred, fallback modelsDefaults) modelsDefaults {
	return modelsDefaults{
		Anthropic:   mergeModelList(preferred.Anthropic, fallback.Anthropic),
		OpenAI:      mergeModelList(preferred.OpenAI, fallback.OpenAI),
		Copilot:     mergeModelList(preferred.Copilot, fallback.Copilot),
		CursorAgent: mergeModelList(preferred.CursorAgent, fallback.CursorAgent),
	}
}

func mergeModelList(preferred, fallback []Model) []Model {
	out := make([]Model, 0, len(preferred)+len(fallback))
	seen := map[string]bool{}
	for _, m := range preferred {
		if m.ID == "" || seen[m.ID] {
			continue
		}
		seen[m.ID] = true
		out = append(out, m)
	}
	for _, m := range fallback {
		if m.ID == "" || seen[m.ID] {
			continue
		}
		seen[m.ID] = true
		out = append(out, m)
	}
	return out
}
