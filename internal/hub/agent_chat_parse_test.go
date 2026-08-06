package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeAgentChatFixture(t *testing.T, lines ...any) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range lines {
		data, err := json.Marshal(line)
		if err != nil {
			_ = f.Close()
			t.Fatal(err)
		}
		if _, err := f.Write(append(data, '\n')); err != nil {
			_ = f.Close()
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestParseClaudeTranscriptSeparatesContent(t *testing.T) {
	path := writeAgentChatFixture(t,
		map[string]any{
			"type": "user", "timestamp": "2026-08-06T00:00:01Z", "message": map[string]any{
				"role": "user", "content": []any{map[string]any{"type": "text", "text": "Fix the parser"}},
			},
		},
		map[string]any{
			"type": "assistant", "timestamp": "2026-08-06T00:00:02Z", "message": map[string]any{
				"role": "assistant", "content": []any{
					map[string]any{"type": "thinking", "thinking": "Inspect PASSWORD=supersecret before editing"},
					map[string]any{"type": "text", "text": "I will update the parser."},
					map[string]any{"type": "tool_use", "id": "tool-1", "name": "Read", "input": map[string]any{"path": "internal/hub/agent_chat.go"}},
				},
			},
		},
		map[string]any{
			"type": "user", "timestamp": "2026-08-06T00:00:03Z", "message": map[string]any{
				"role": "user", "content": []any{map[string]any{"type": "tool_result", "tool_use_id": "tool-1", "content": "done"}},
			},
		},
	)

	messages, offset, err := parseClaudeTranscript(path, 0)
	if err != nil {
		t.Fatal(err)
	}
	if offset == 0 {
		t.Fatal("expected complete records to advance offset")
	}
	if len(messages) != 2 {
		t.Fatalf("got %d messages, want user + assistant", len(messages))
	}
	if messages[0].Role != "user" || messages[0].Text != "Fix the parser" {
		t.Fatalf("unexpected user message: %#v", messages[0])
	}
	assistant := messages[1]
	if assistant.Text != "I will update the parser." {
		t.Errorf("assistant text = %q", assistant.Text)
	}
	if len(assistant.Thinking) != 1 || strings.Contains(assistant.Thinking[0], "supersecret") {
		t.Errorf("thinking was not retained and masked: %#v", assistant.Thinking)
	}
	if len(assistant.Tools) != 1 || assistant.Tools[0].Name != "Read" || assistant.Tools[0].Result != "done" {
		t.Errorf("tool was not retained/result attached: %#v", assistant.Tools)
	}
}

func TestParseClaudeTranscriptSkipsSyntheticAndSidechain(t *testing.T) {
	path := writeAgentChatFixture(t,
		map[string]any{
			"type": "user", "message": map[string]any{
				"role": "user", "content": []any{map[string]any{"type": "text", "text": "<system-reminder>internal"}},
			},
		},
		map[string]any{
			"type": "assistant", "isSidechain": true, "message": map[string]any{
				"role": "assistant", "content": []any{map[string]any{"type": "text", "text": "subagent note"}},
			},
		},
	)
	messages, _, err := parseClaudeTranscript(path, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Kind != "sidechain" || messages[0].Text != "" {
		t.Fatalf("unexpected sidechain result: %#v", messages)
	}
	if len(messages[0].Thinking) != 1 || messages[0].Thinking[0] != "subagent note" {
		t.Fatalf("sidechain was not moved to thinking: %#v", messages[0])
	}
}

func TestParseCodexRolloutSeparatesReasoningAndTools(t *testing.T) {
	path := writeAgentChatFixture(t,
		map[string]any{
			"type": "session_meta", "payload": map[string]any{"cwd": "C:/work", "timestamp": "2026-08-06T00:00:00Z"},
		},
		map[string]any{
			"type": "response_item", "timestamp": "2026-08-06T00:00:01Z", "payload": map[string]any{
				"type": "message", "role": "assistant", "content": []any{
					map[string]any{"type": "summary_text", "text": "Reasoning summary"},
					map[string]any{"type": "output_text", "text": "Here is the result."},
				},
			},
		},
		map[string]any{
			"type": "response_item", "timestamp": "2026-08-06T00:00:02Z", "payload": map[string]any{
				"type": "function_call", "name": "shell", "call_id": "call-1", "arguments": map[string]any{"command": "pwd"},
			},
		},
		map[string]any{
			"type": "response_item", "timestamp": "2026-08-06T00:00:03Z", "payload": map[string]any{
				"type": "function_call_output", "call_id": "call-1", "output": "C:/work",
			},
		},
	)
	messages, _, err := parseCodexRollout(path, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 2 {
		t.Fatalf("got %d messages, want assistant message + tool message", len(messages))
	}
	if messages[0].Text != "Here is the result." || len(messages[0].Thinking) != 1 {
		t.Fatalf("unexpected codex assistant message: %#v", messages[0])
	}
	if len(messages[1].Tools) != 1 || messages[1].Tools[0].Name != "shell" || messages[1].Tools[0].Result != "C:/work" {
		t.Fatalf("unexpected codex tool message: %#v", messages[1])
	}
}

func TestParseAgentChatLeavesIncompleteTailAtOffset(t *testing.T) {
	path := writeAgentChatFixture(t, map[string]any{
		"type": "user", "message": map[string]any{"role": "user", "content": "first"},
	})
	stat, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	incomplete := `{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"unfinished"}]}`
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(incomplete); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	messages, offset, err := parseClaudeTranscript(path, stat.Size())
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 0 {
		t.Fatalf("incomplete record produced messages: %#v", messages)
	}
	if offset != stat.Size() {
		t.Fatalf("offset advanced across incomplete record: got %d want %d", offset, stat.Size())
	}
}
