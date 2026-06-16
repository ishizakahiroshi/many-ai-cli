package proxy

import (
	"bytes"
	"encoding/json"
	"regexp"
)

// 既知シークレットパターンの補助マスキング（完全防御ではない・README に明記）。
// payload 全体を文字列として走査するので過剰マスキング気味だが、安全側に倒す。
var secretPatterns = []*regexp.Regexp{
	regexp.MustCompile(`sk-[A-Za-z0-9_\-]{20,}`),        // OpenAI / Anthropic / 多くの形式
	regexp.MustCompile(`ghp_[A-Za-z0-9]{20,}`),          // GitHub PAT
	regexp.MustCompile(`gho_[A-Za-z0-9]{20,}`),          // GitHub OAuth
	regexp.MustCompile(`AKIA[0-9A-Z]{16}`),              // AWS access key id
	regexp.MustCompile(`xox[abprs]-[A-Za-z0-9\-]{10,}`), // Slack
	regexp.MustCompile(`AIza[0-9A-Za-z\-_]{35}`),        // Google API key
}

// MaskSecrets は既知シークレットパターンを `***` に置換する。
// JSON 構造を破壊しないよう、string token のみを走査する。
//
// payload が JSON でない場合は文字列全体に regex を適用する（best-effort）。
func MaskSecrets(payload []byte) []byte {
	if len(payload) == 0 {
		return payload
	}
	var any interface{}
	if err := json.Unmarshal(payload, &any); err != nil {
		// JSON でなければ生バイトに対して regex 置換
		return applyMaskRegex(payload)
	}
	masked := walkAndMask(any)
	out, err := json.Marshal(masked)
	if err != nil {
		return payload
	}
	return out
}

func walkAndMask(v interface{}) interface{} {
	switch t := v.(type) {
	case string:
		s := t
		for _, re := range secretPatterns {
			s = re.ReplaceAllString(s, "***")
		}
		return s
	case []interface{}:
		for i := range t {
			t[i] = walkAndMask(t[i])
		}
		return t
	case map[string]interface{}:
		for k, val := range t {
			t[k] = walkAndMask(val)
		}
		return t
	default:
		return v
	}
}

func applyMaskRegex(b []byte) []byte {
	out := b
	for _, re := range secretPatterns {
		out = re.ReplaceAll(out, []byte("***"))
	}
	return out
}

// SummarizeAnthropicRequest は Anthropic /v1/messages のリクエスト本文から
// 表示しやすい構造化サマリを取り出す（モデル / メッセージ数 / system 有無）。
// 失敗時は zero value。
type AnthropicRequestSummary struct {
	Model        string `json:"model,omitempty"`
	MessageCount int    `json:"message_count,omitempty"`
	HasSystem    bool   `json:"has_system,omitempty"`
	ToolCount    int    `json:"tool_count,omitempty"`
}

func SummarizeAnthropicRequest(body []byte) AnthropicRequestSummary {
	var v struct {
		Model    string            `json:"model"`
		Messages []json.RawMessage `json:"messages"`
		System   json.RawMessage   `json:"system"`
		Tools    []json.RawMessage `json:"tools"`
	}
	if err := json.Unmarshal(body, &v); err != nil {
		return AnthropicRequestSummary{}
	}
	return AnthropicRequestSummary{
		Model:        v.Model,
		MessageCount: len(v.Messages),
		HasSystem:    len(bytes.TrimSpace(v.System)) > 0,
		ToolCount:    len(v.Tools),
	}
}

// SummarizeOpenAIRequest は OpenAI /v1/chat/completions のリクエスト本文から
// 同様のサマリを取り出す。
type OpenAIRequestSummary struct {
	Model        string `json:"model,omitempty"`
	MessageCount int    `json:"message_count,omitempty"`
	ToolCount    int    `json:"tool_count,omitempty"`
}

func SummarizeOpenAIRequest(body []byte) OpenAIRequestSummary {
	var v struct {
		Model    string            `json:"model"`
		Messages []json.RawMessage `json:"messages"`
		Tools    []json.RawMessage `json:"tools"`
	}
	if err := json.Unmarshal(body, &v); err != nil {
		return OpenAIRequestSummary{}
	}
	return OpenAIRequestSummary{
		Model:        v.Model,
		MessageCount: len(v.Messages),
		ToolCount:    len(v.Tools),
	}
}
