package hub

import (
	"encoding/json"

	"many-ai-cli/internal/proxy"
)

// extractTokenUsage は response payload から input/output トークン数を抽出する。
// 失敗・該当なしは 0, 0 を返す。SSE 結合後の payload は data 行が改行区切りで連結されているので
// 各行を順次パースして最大値を採用する（usage は delta ではなく cumulative の場合が多いため最後の値が最終）。
func extractTokenUsage(provider proxy.Provider, body []byte) (tokensIn, tokensOut int) {
	if len(body) == 0 {
		return 0, 0
	}
	// Anthropic / OpenAI どちらも JSON object（または 1 JSON / 行の SSE 結合）。
	// 行単位で試行 → 最後の usage を採用。
	lines := splitJSONLines(body)
	for _, line := range lines {
		in, out := parseTokenUsageOne(provider, line)
		if in > 0 || out > 0 {
			tokensIn, tokensOut = in, out
		}
	}
	return tokensIn, tokensOut
}

func parseTokenUsageOne(provider proxy.Provider, line []byte) (int, int) {
	switch provider {
	case proxy.ProviderAnthropic:
		// 非ストリーミング: {"usage":{"input_tokens":..,"output_tokens":..}}
		// ストリーミング: {"type":"message_start","message":{"usage":{...}}} / {"type":"message_delta","usage":{...}}
		var v struct {
			Usage   *anthropicUsage `json:"usage"`
			Message *struct {
				Usage *anthropicUsage `json:"usage"`
			} `json:"message"`
		}
		if err := json.Unmarshal(line, &v); err != nil {
			return 0, 0
		}
		var u *anthropicUsage
		if v.Usage != nil {
			u = v.Usage
		} else if v.Message != nil && v.Message.Usage != nil {
			u = v.Message.Usage
		}
		if u != nil {
			return u.InputTokens, u.OutputTokens
		}
	case proxy.ProviderOpenAI:
		// chat.completions: {"usage":{"prompt_tokens":..,"completion_tokens":..}}
		// streaming: 最後の chunk に usage が入る場合がある（include_usage:true 指定時）
		var v struct {
			Usage *openAIUsage `json:"usage"`
		}
		if err := json.Unmarshal(line, &v); err != nil {
			return 0, 0
		}
		if v.Usage != nil {
			return v.Usage.PromptTokens, v.Usage.CompletionTokens
		}
	}
	return 0, 0
}

type anthropicUsage struct {
	InputTokens  int `json:"input_tokens"`
	OutputTokens int `json:"output_tokens"`
}

type openAIUsage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
}

// splitJSONLines は SSE 結合済み payload（改行区切り JSON）を行単位に分解する。
// 単一 JSON object も 1 要素として返す。空行はスキップ。
func splitJSONLines(b []byte) [][]byte {
	var out [][]byte
	start := 0
	for i := 0; i < len(b); i++ {
		if b[i] == '\n' {
			if i > start {
				line := b[start:i]
				if hasNonSpace(line) {
					out = append(out, line)
				}
			}
			start = i + 1
		}
	}
	if start < len(b) {
		line := b[start:]
		if hasNonSpace(line) {
			out = append(out, line)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func hasNonSpace(b []byte) bool {
	for _, c := range b {
		if c != ' ' && c != '\t' && c != '\r' {
			return true
		}
	}
	return false
}
