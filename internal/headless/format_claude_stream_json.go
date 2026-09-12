package headless

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

// format_claude_stream_json.go reads `claude -p --output-format stream-json
// --verbose`: one JSON object per line.
//
// The shapes below are the subset this parser needs, not the whole schema. That
// is deliberate: every field is optional in practice, an unknown `type` is
// skipped rather than reported, and a line that is not JSON at all falls back
// to being shown as plain output. **One bad line must never fail a run**
// (元設計 10 節): the process exit code is the verdict, and a parser that can
// veto it would make the least reliable part of the pipeline the deciding one.

type claudeStreamJSONParser struct{}

type claudeStreamLine struct {
	Type    string          `json:"type"`
	Subtype string          `json:"subtype"`
	IsError bool            `json:"is_error"`
	Result  string          `json:"result"`
	Model   string          `json:"model"`
	Message *claudeMessage  `json:"message"`
	Error   json.RawMessage `json:"error"`
}

type claudeMessage struct {
	Role    string          `json:"role"`
	Model   string          `json:"model"`
	Content json.RawMessage `json:"content"`
}

type claudeContentBlock struct {
	Type    string          `json:"type"`
	Text    string          `json:"text"`
	Name    string          `json:"name"`
	Input   json.RawMessage `json:"input"`
	Content json.RawMessage `json:"content"`
	IsError bool            `json:"is_error"`
}

func (claudeStreamJSONParser) Parse(line []byte) []Event {
	trimmed := bytes.TrimSpace(line)
	if len(trimmed) == 0 {
		return nil
	}
	var parsed claudeStreamLine
	if err := json.Unmarshal(trimmed, &parsed); err != nil {
		// Text fallback (元設計 10 節). A CLI that fails before it starts
		// streaming prints a plain sentence; swallowing it would leave the
		// session showing nothing but a non-zero exit.
		return []Event{{Type: EventOutput, Text: string(trimmed)}}
	}
	switch parsed.Type {
	case "system":
		if parsed.Subtype != "init" {
			return nil
		}
		text := "started"
		if model := strings.TrimSpace(parsed.Model); model != "" {
			text += " model=" + model
		}
		return []Event{{Type: EventRunStarted, Text: text}}
	case "assistant":
		return claudeAssistantEvents(parsed.Message)
	case "user":
		return claudeToolResultEvents(parsed.Message)
	case "result":
		event := Event{Type: EventRunCompleted, Text: claudeResultText(parsed)}
		if parsed.IsError || (parsed.Subtype != "" && parsed.Subtype != "success") {
			event.Type = EventRunFailed
			event.IsError = true
		}
		return []Event{event}
	default:
		// Partial message chunks, hook events, anything added later: the run
		// is understood well enough without them.
		return nil
	}
}

func claudeAssistantEvents(msg *claudeMessage) []Event {
	if msg == nil {
		return nil
	}
	blocks, text := claudeContent(msg.Content)
	if len(blocks) == 0 {
		if text == "" {
			return nil
		}
		return []Event{{Type: EventAssistant, Text: text}}
	}
	var out []Event
	for _, block := range blocks {
		switch block.Type {
		case "text":
			if trimmed := strings.TrimSpace(block.Text); trimmed != "" {
				out = append(out, Event{Type: EventAssistant, Text: trimmed})
			}
		case "tool_use":
			out = append(out, Event{
				Type: EventToolStarted,
				Tool: block.Name,
				Text: claudeToolInputSummary(block.Input),
			})
		}
	}
	return out
}

func claudeToolResultEvents(msg *claudeMessage) []Event {
	if msg == nil {
		return nil
	}
	blocks, _ := claudeContent(msg.Content)
	var out []Event
	for _, block := range blocks {
		if block.Type != "tool_result" {
			continue
		}
		_, text := claudeContent(block.Content)
		out = append(out, Event{
			Type:    EventToolResult,
			Text:    firstLine(text),
			IsError: block.IsError,
		})
	}
	return out
}

// claudeContent decodes a `content` field that is either a list of blocks or a
// bare string, which is how the same key is used for both a model turn and a
// tool result.
func claudeContent(raw json.RawMessage) ([]claudeContentBlock, string) {
	if len(bytes.TrimSpace(raw)) == 0 {
		return nil, ""
	}
	var blocks []claudeContentBlock
	if err := json.Unmarshal(raw, &blocks); err == nil {
		var texts []string
		for _, block := range blocks {
			if block.Type == "text" && strings.TrimSpace(block.Text) != "" {
				texts = append(texts, strings.TrimSpace(block.Text))
			}
		}
		return blocks, strings.Join(texts, " ")
	}
	var text string
	if err := json.Unmarshal(raw, &text); err == nil {
		return nil, strings.TrimSpace(text)
	}
	return nil, ""
}

// claudeToolInputSummary picks the one field that says what a tool call is
// about. The whole input is deliberately not rendered: it can be a file's
// entire new contents.
func claudeToolInputSummary(raw json.RawMessage) string {
	if len(bytes.TrimSpace(raw)) == 0 {
		return ""
	}
	var input map[string]any
	if err := json.Unmarshal(raw, &input); err != nil {
		return ""
	}
	for _, key := range []string{"command", "file_path", "path", "pattern", "url", "query", "description"} {
		if value, ok := input[key].(string); ok {
			if trimmed := firstLine(value); trimmed != "" {
				return trimmed
			}
		}
	}
	return ""
}

func claudeResultText(parsed claudeStreamLine) string {
	parts := make([]string, 0, 2)
	if subtype := strings.TrimSpace(parsed.Subtype); subtype != "" {
		parts = append(parts, subtype)
	}
	if summary := firstLine(parsed.Result); summary != "" {
		parts = append(parts, summary)
	} else if len(bytes.TrimSpace(parsed.Error)) > 0 {
		var message string
		if err := json.Unmarshal(parsed.Error, &message); err == nil {
			if summary := firstLine(message); summary != "" {
				parts = append(parts, summary)
			}
		} else {
			var object map[string]any
			if err := json.Unmarshal(parsed.Error, &object); err == nil {
				if text, ok := object["message"].(string); ok {
					parts = append(parts, firstLine(text))
				}
			}
		}
	}
	if len(parts) == 0 {
		return fmt.Sprintf("is_error=%v", parsed.IsError)
	}
	return strings.Join(parts, ": ")
}
