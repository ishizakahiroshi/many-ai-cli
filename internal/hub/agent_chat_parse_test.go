package hub

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
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

	state := newAgentChatParseState()
	messages, offset, err := parseClaudeTranscriptWithState(path, stat.Size(), state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 0 {
		t.Fatalf("incomplete record produced messages: %#v", messages)
	}
	if offset <= stat.Size() {
		t.Fatalf("partial record did not advance physical cursor: got %d start %d", offset, stat.Size())
	}
	f, err = os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString("}\n"); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	messages, offset, err = parseClaudeTranscriptWithState(path, offset, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Text != "unfinished" {
		t.Fatalf("completed partial record was not recovered: %#v", messages)
	}
	if stat, err := os.Stat(path); err != nil || offset != stat.Size() {
		t.Fatalf("cursor did not reach completed record boundary: offset=%d stat=%v err=%v", offset, stat, err)
	}
}

func TestReadAgentChatTailSkipsOversizedRecordBeforeNextRecord(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	valid := `{"type":"user","message":{"role":"user","content":"after oversized"}}` + "\n"
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(strings.Repeat("x", agentChatLineMax+1) + "\n" + valid); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	var lines [][]byte
	offset, err := readAgentChatTail(path, 0, func(line []byte) error {
		lines = append(lines, append([]byte(nil), line...))
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(lines) != 1 || string(lines[0]) != strings.TrimSuffix(valid, "\n") {
		t.Fatalf("unexpected records after oversized line: %q", lines)
	}
	stat, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if offset != stat.Size() {
		t.Fatalf("offset did not advance past complete records: got %d want %d", offset, stat.Size())
	}
}

func TestParseClaudeTranscriptCarriesToolResultAcrossPolls(t *testing.T) {
	path := writeAgentChatFixture(t, map[string]any{
		"type": "assistant", "timestamp": "2026-08-06T00:00:01Z", "message": map[string]any{
			"role": "assistant", "content": []any{
				map[string]any{"type": "tool_use", "id": "tool-cross-poll", "name": "Read", "input": map[string]any{"path": "file.txt"}},
			},
		},
	})
	state := newAgentChatParseState()
	messages, offset, err := parseClaudeTranscriptWithState(path, 0, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Tools[0].Result != "" {
		t.Fatalf("unexpected first poll: %#v", messages)
	}
	result := map[string]any{
		"type": "user", "timestamp": "2026-08-06T00:00:02Z", "message": map[string]any{
			"role": "user", "content": []any{map[string]any{"type": "tool_result", "tool_use_id": "tool-cross-poll", "content": "done later"}},
		},
	}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.Write(append(data, '\n')); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	messages, _, err = parseClaudeTranscriptWithState(path, offset, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || len(messages[0].Tools) != 1 || messages[0].Tools[0].Result != "done later" {
		t.Fatalf("cross-poll Claude result was not attached: %#v", messages)
	}
	if messages[0].MessageID != "tool:tool-cross-poll" || len(messages) != 1 {
		t.Fatalf("cross-poll Claude update lost stable identity: %#v", messages)
	}
}

func TestParseCodexRolloutCarriesToolResultAcrossPolls(t *testing.T) {
	path := writeAgentChatFixture(t, map[string]any{
		"type": "response_item", "timestamp": "2026-08-06T00:00:01Z", "payload": map[string]any{
			"type": "function_call", "name": "shell", "call_id": "call-cross-poll", "arguments": map[string]any{"command": "pwd"},
		},
	})
	state := newAgentChatParseState()
	messages, offset, err := parseCodexRolloutWithState(path, 0, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Tools[0].Result != "" {
		t.Fatalf("unexpected first poll: %#v", messages)
	}
	result := map[string]any{
		"type": "response_item", "timestamp": "2026-08-06T00:00:02Z", "payload": map[string]any{
			"type": "function_call_output", "call_id": "call-cross-poll", "output": "C:/work",
		},
	}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.Write(append(data, '\n')); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	messages, _, err = parseCodexRolloutWithState(path, offset, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || len(messages[0].Tools) != 1 || messages[0].Tools[0].Result != "C:/work" {
		t.Fatalf("cross-poll Codex result was not attached: %#v", messages)
	}
	if messages[0].MessageID != "tool:call-cross-poll" || len(messages) != 1 {
		t.Fatalf("cross-poll Codex update lost stable identity: %#v", messages)
	}
}

func TestParseAgentChatTailUsesBoundedPage(t *testing.T) {
	lines := make([]any, 0, 300)
	for i := 0; i < 300; i++ {
		lines = append(lines, map[string]any{
			"type": "user", "message": map[string]any{"role": "user", "content": fmt.Sprintf("message-%03d", i)},
		})
	}
	path := writeAgentChatFixture(t, lines...)
	state := newAgentChatParseStateWithPage(5, agentChatBatchBytesMax, -1)
	messages, _, err := parseClaudeTranscriptTailPage(path, state, -1)
	if err != nil {
		t.Fatal(err)
	}
	if state.parsedMessages <= 0 || state.parsedMessages > agentChatTailRecordLimit(5) || len(messages) != 5 {
		t.Fatalf("bounded page mismatch: parsed=%d messages=%d", state.parsedMessages, len(messages))
	}
	if messages[0].Text != "message-295" || messages[4].Text != "message-299" {
		t.Fatalf("bounded page did not retain the tail: %#v", messages)
	}
}

func TestParseAgentChatTailPageDoesNotScanWholeLongHistory(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	payload := strings.Repeat("x", 1500)
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 20000; i++ {
		if _, err := fmt.Fprintf(f, `{"type":"user","message":{"role":"user","content":"message-%05d-%s"}}`+"\n", i, payload); err != nil {
			_ = f.Close()
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	stat, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	state := newAgentChatParseStateWithPage(5, agentChatBatchBytesMax, -1)
	messages, _, err := parseClaudeTranscriptTailPage(path, state, -1)
	if err != nil {
		t.Fatal(err)
	}
	if stat.Size() <= agentChatPageBytesMax || state.lastRead.BytesRead > agentChatPageBytesMax || state.lastRead.BytesRead >= stat.Size() {
		t.Fatalf("tail page read was not bounded: file=%d read=%d stats=%+v", stat.Size(), state.lastRead.BytesRead, state.lastRead)
	}
	if state.lastRead.Records > agentChatPageRecordsMax || len(messages) != 5 {
		t.Fatalf("tail page decode was not bounded: records=%d messages=%d", state.lastRead.Records, len(messages))
	}
	if !strings.HasPrefix(messages[len(messages)-1].Text, "message-19999-") {
		t.Fatalf("tail page did not return the latest message: %#v", messages[len(messages)-1])
	}
}

func TestAgentChatPendingToolsStayBoundedAcrossBatches(t *testing.T) {
	state := newAgentChatParseStateWithPage(agentChatLiveMessageMax, agentChatBatchBytesMax, -1)
	for batch := 0; batch < 2; batch++ {
		state.beginBatch()
		for i := 0; i < 200; i++ {
			id := fmt.Sprintf("pending-%03d", batch*200+i)
			message := &agentChatMessage{
				Role:     "assistant",
				Thinking: make([]string, agentChatThinkingMax+10),
				Tools: []agentChatTool{{
					ID: id, Name: "shell", Input: strings.Repeat("x", agentChatTextMax),
				}},
			}
			state.appendMessage(message, true)
			registerAgentChatTools(state, message)
		}
	}
	if state.pendingBytes > agentChatPendingBytesMax || len(state.tools) > agentChatPendingToolMax {
		t.Fatalf("pending state exceeded bounds: bytes=%d tools=%d", state.pendingBytes, len(state.tools))
	}
	if _, exists := state.tools["pending-000"]; exists {
		t.Fatal("pending eviction was not FIFO/deterministic")
	}
	if _, exists := state.tools["pending-399"]; !exists {
		t.Fatal("latest pending tool was evicted unexpectedly")
	}
	for _, message := range state.messages {
		if len(message.Thinking) > agentChatThinkingMax || len(message.Tools) > agentChatToolsMax {
			t.Fatalf("message component bounds were not applied: thinking=%d tools=%d", len(message.Thinking), len(message.Tools))
		}
	}
}

func TestAgentChatRangeAdvancesAcrossFourMiBLine(t *testing.T) {
	tests := []struct {
		name  string
		large string
		parse func(string, int64, *agentChatParseState) ([]agentChatMessage, int64, error)
	}{
		{
			name:  "claude",
			large: `{"type":"user","message":{"role":"user","content":"` + strings.Repeat("x", agentChatReadBytesMax+1024) + `"}}` + "\n",
			parse: parseClaudeTranscriptWithState,
		},
		{
			name:  "codex",
			large: `{"type":"event_msg","payload":{"type":"user_message","message":"` + strings.Repeat("x", agentChatReadBytesMax+1024) + `"}}` + "\n",
			parse: parseCodexRolloutWithState,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var after string
			if tt.name == "claude" {
				after = `{"type":"user","message":{"role":"user","content":"after-large"}}` + "\n"
			} else {
				after = `{"type":"event_msg","payload":{"type":"user_message","message":"after-large"}}` + "\n"
			}
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			if err := os.WriteFile(path, []byte(tt.large+after), 0o600); err != nil {
				t.Fatal(err)
			}
			stat, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			state := newAgentChatParseState()
			var offset int64
			var seenAfter bool
			for poll := 0; poll < 8 && (!seenAfter || offset < stat.Size()); poll++ {
				messages, next, err := tt.parse(path, offset, state)
				if err != nil {
					t.Fatal(err)
				}
				if next <= offset {
					t.Fatalf("cursor stopped at poll %d: offset=%d next=%d stats=%+v", poll, offset, next, state.lastRead)
				}
				if len(state.readState.record) > agentChatLineMax {
					t.Fatalf("partial line exceeded bounded memory: %d", len(state.readState.record))
				}
				for _, message := range messages {
					if message.Text == "after-large" {
						seenAfter = true
					}
				}
				offset = next
			}
			if !seenAfter || offset != stat.Size() {
				t.Fatalf("large complete line or following record was lost: seenAfter=%v offset=%d size=%d state=%+v", seenAfter, offset, stat.Size(), state.readState)
			}
		})
	}
}

func TestAgentChatRangeSkipsOverEightMiBLineAcrossPolls(t *testing.T) {
	tests := []struct {
		name  string
		valid string
		parse func(string, int64, *agentChatParseState) ([]agentChatMessage, int64, error)
	}{
		{
			name:  "claude",
			valid: `{"type":"user","message":{"role":"user","content":"after-oversized"}}` + "\n",
			parse: parseClaudeTranscriptWithState,
		},
		{
			name:  "codex",
			valid: `{"type":"event_msg","payload":{"type":"user_message","message":"after-oversized"}}` + "\n",
			parse: parseCodexRolloutWithState,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			data := strings.Repeat("x", agentChatLineMax+1024) + "\n" + tt.valid
			if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
				t.Fatal(err)
			}
			stat, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			state := newAgentChatParseState()
			var offset int64
			var seenAfter bool
			for poll := 0; poll < 8 && !seenAfter; poll++ {
				messages, next, err := tt.parse(path, offset, state)
				if err != nil {
					t.Fatal(err)
				}
				if next <= offset {
					t.Fatalf("oversized skip cursor stopped at poll %d: offset=%d next=%d", poll, offset, next)
				}
				if len(state.readState.record) > agentChatLineMax {
					t.Fatalf("oversized line allocated beyond bound: %d", len(state.readState.record))
				}
				for _, message := range messages {
					if message.Text == "after-oversized" {
						seenAfter = true
					}
				}
				offset = next
			}
			if !seenAfter || offset != stat.Size() || state.readState.oversized {
				t.Fatalf("oversized line blocked following record: seenAfter=%v offset=%d size=%d state=%+v", seenAfter, offset, stat.Size(), state.readState)
			}
		})
	}
}

func TestAgentChatPrimeUsesSafeSnapshotBoundaryForClaudeAndCodex(t *testing.T) {
	tests := []struct {
		name      string
		complete  string
		partial   string
		parsePage func(string, *agentChatParseState, int64) ([]agentChatMessage, int64, error)
		parseLive func(string, int64, *agentChatParseState) ([]agentChatMessage, int64, error)
	}{
		{
			name:      "claude",
			complete:  `{"type":"user","message":{"role":"user","content":"before-prime"}}` + "\n",
			partial:   `{"type":"user","message":{"role":"user","content":"after-prime"}`,
			parsePage: parseClaudeTranscriptTailPage,
			parseLive: parseClaudeTranscriptWithState,
		},
		{
			name:      "codex",
			complete:  `{"type":"event_msg","payload":{"type":"user_message","message":"before-prime"}}` + "\n",
			partial:   `{"type":"event_msg","payload":{"type":"user_message","message":"after-prime"}`,
			parsePage: parseCodexRolloutTailPage,
			parseLive: parseCodexRolloutWithState,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			if err := os.WriteFile(path, []byte(tt.complete+tt.partial), 0o600); err != nil {
				t.Fatal(err)
			}
			state := newAgentChatParseStateWithPage(agentChatLiveMessageMax, agentChatBatchBytesMax, -1)
			if _, _, err := tt.parsePage(path, state, -1); err != nil {
				t.Fatal(err)
			}
			safeOffset := state.lastRead.SafeOffset
			if safeOffset != int64(len(tt.complete)) {
				t.Fatalf("prime cursor crossed incomplete tail: safe=%d complete=%d stats=%+v", safeOffset, len(tt.complete), state.lastRead)
			}
			f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := f.WriteString("}\n"); err != nil {
				_ = f.Close()
				t.Fatal(err)
			}
			if err := f.Close(); err != nil {
				t.Fatal(err)
			}

			reattached := newAgentChatParseState()
			messages, next, err := tt.parseLive(path, safeOffset, reattached)
			if err != nil {
				t.Fatal(err)
			}
			if next <= safeOffset || len(messages) != 1 || messages[0].Text != "after-prime" {
				t.Fatalf("record appended after prime was not recovered: next=%d messages=%#v stats=%+v", next, messages, reattached.lastRead)
			}
		})
	}
}

func TestAgentChatTailPaginationHasNoGapForClaudeAndCodex(t *testing.T) {
	tests := []struct {
		name  string
		line  func(int) string
		parse func(string, *agentChatParseState, int64) ([]agentChatMessage, int64, error)
	}{
		{
			name: "claude",
			line: func(i int) string {
				return fmt.Sprintf(`{"type":"user","message":{"role":"user","content":"page-%03d"}}`, i)
			},
			parse: parseClaudeTranscriptTailPage,
		},
		{
			name: "codex",
			line: func(i int) string {
				return fmt.Sprintf(`{"type":"event_msg","payload":{"type":"user_message","message":"page-%03d"}}`, i)
			},
			parse: parseCodexRolloutTailPage,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			var data strings.Builder
			for i := 0; i < 120; i++ {
				data.WriteString(tt.line(i))
				data.WriteByte('\n')
			}
			if err := os.WriteFile(path, []byte(data.String()), 0o600); err != nil {
				t.Fatal(err)
			}
			seen := make(map[string]struct{})
			cursor := int64(-1)
			for page := 0; page < 40; page++ {
				state := newAgentChatParseStateWithPage(5, agentChatBatchBytesMax, -1)
				messages, next, err := tt.parse(path, state, cursor)
				if err != nil {
					t.Fatal(err)
				}
				if len(messages) == 0 && next > 0 {
					t.Fatalf("page parser returned no visible records before older cursor: parsed=%d next=%d stats=%+v", state.parsedMessages, next, state.lastRead)
				}
				for _, message := range messages {
					if _, exists := seen[message.Text]; exists {
						t.Fatalf("duplicate message across cursor pages: %q", message.Text)
					}
					seen[message.Text] = struct{}{}
				}
				if next == 0 {
					break
				}
				if cursor >= 0 && next >= cursor {
					t.Fatalf("cursor did not move backward: previous=%d next=%d", cursor, next)
				}
				cursor = next
			}
			if len(seen) != 120 {
				t.Fatalf("pagination lost records: got=%d want=120", len(seen))
			}
		})
	}
}

func TestAgentChatTailDeadlinePreservesRetryCursorForClaudeAndCodex(t *testing.T) {
	deadline := time.Unix(1_700_000_000, 0)
	tests := []struct {
		name      string
		line      func(string) string
		parsePage func(string, *agentChatParseState, int64, agentChatReadBudget) ([]agentChatMessage, int64, error)
	}{
		{
			name: "claude",
			line: func(text string) string {
				return fmt.Sprintf(`{"type":"user","message":{"role":"user","content":"%s"}}`, text)
			},
			parsePage: parseClaudeTranscriptTailPageWithBudget,
		},
		{
			name: "codex",
			line: func(text string) string {
				return fmt.Sprintf(`{"type":"event_msg","payload":{"type":"user_message","message":"%s"}}`, text)
			},
			parsePage: parseCodexRolloutTailPageWithBudget,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			var data strings.Builder
			for i := 0; i < 4; i++ {
				data.WriteString(tt.line(fmt.Sprintf("deadline-%d", i)))
				data.WriteByte('\n')
			}
			if err := os.WriteFile(path, []byte(data.String()), 0o600); err != nil {
				t.Fatal(err)
			}
			endOffset := int64(data.Len())
			state := newAgentChatParseStateWithPage(8, agentChatBatchBytesMax, -1)
			messages, next, err := tt.parsePage(path, state, -1, agentChatReadBudget{
				MaxBytes:   agentChatPageBytesMax,
				MaxRecords: agentChatPageRecordsMax,
				Deadline:   deadline,
				Clock:      func() time.Time { return deadline.Add(time.Second) },
			})
			if err != nil {
				t.Fatal(err)
			}
			if len(messages) != 0 || !state.lastRead.HitBudget || state.lastRead.DecodedRecords != 0 || state.lastRead.DecodeCommitted {
				t.Fatalf("expired tail budget decoded data: messages=%#v stats=%+v", messages, state.lastRead)
			}
			if next != endOffset || state.lastRead.BytesRead > agentChatPageBytesMax {
				t.Fatalf("expired tail budget moved cursor: next=%d end=%d stats=%+v", next, endOffset, state.lastRead)
			}

			resumed := newAgentChatParseStateWithPage(8, agentChatBatchBytesMax, -1)
			messages, resumedNext, err := tt.parsePage(path, resumed, next, agentChatReadBudget{
				MaxBytes:   agentChatPageBytesMax,
				MaxRecords: agentChatPageRecordsMax,
				Deadline:   deadline.Add(time.Second),
				Clock:      func() time.Time { return deadline },
			})
			if err != nil {
				t.Fatal(err)
			}
			if len(messages) != 4 || resumedNext != 0 || !resumed.lastRead.DecodeCommitted {
				t.Fatalf("retry after expired tail budget lost records: messages=%#v next=%d stats=%+v", messages, resumedNext, resumed.lastRead)
			}
		})
	}
}

func TestAgentChatTailDeadlineAfterPartialDecodeKeepsPageContinuousForClaudeAndCodex(t *testing.T) {
	deadline := time.Unix(1_700_000_000, 0)
	tests := []struct {
		name      string
		line      func(int) string
		parsePage func(string, *agentChatParseState, int64, agentChatReadBudget) ([]agentChatMessage, int64, error)
		parseLive func(string, int64, *agentChatParseState) ([]agentChatMessage, int64, error)
	}{
		{
			name: "claude",
			line: func(i int) string {
				return fmt.Sprintf(`{"type":"user","message":{"role":"user","content":"partial-%d"}}`, i)
			},
			parsePage: parseClaudeTranscriptTailPageWithBudget,
			parseLive: parseClaudeTranscriptWithState,
		},
		{
			name: "codex",
			line: func(i int) string {
				return fmt.Sprintf(`{"type":"event_msg","payload":{"type":"user_message","message":"partial-%d"}}`, i)
			},
			parsePage: parseCodexRolloutTailPageWithBudget,
			parseLive: parseCodexRolloutWithState,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "transcript.jsonl")
			var data strings.Builder
			for i := 0; i < 6; i++ {
				data.WriteString(tt.line(i))
				data.WriteByte('\n')
			}
			if err := os.WriteFile(path, []byte(data.String()), 0o600); err != nil {
				t.Fatal(err)
			}

			clockCalls := 0
			clock := func() time.Time {
				clockCalls++
				// The tail reader checks once before loading and once per record.
				// Expire on the first post-decode check, after the page was selected
				// and at least one record has been decoded.
				if clockCalls <= 8 {
					return deadline.Add(-time.Millisecond)
				}
				return deadline.Add(time.Second)
			}
			state := newAgentChatParseStateWithPage(3, agentChatBatchBytesMax, -1)
			messages, next, err := tt.parsePage(path, state, -1, agentChatReadBudget{
				MaxBytes:   agentChatPageBytesMax,
				MaxRecords: agentChatPageRecordsMax,
				Deadline:   deadline,
				Clock:      clock,
			})
			if err != nil {
				t.Fatal(err)
			}
			if !state.lastRead.HitBudget || state.lastRead.DecodedRecords != 6 || clockCalls < 9 {
				t.Fatalf("deadline did not occur after page decode started: calls=%d messages=%#v stats=%+v", clockCalls, messages, state.lastRead)
			}
			if len(messages) != 3 || messages[0].Text != "partial-3" || messages[2].Text != "partial-5" {
				t.Fatalf("bounded page did not retain newest records after deadline: %#v", messages)
			}
			safeOffset := state.lastRead.SafeOffset
			if safeOffset != int64(data.Len()) {
				t.Fatalf("live prime safe offset did not commit the complete snapshot: safe=%d size=%d stats=%+v", safeOffset, data.Len(), state.lastRead)
			}
			f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := f.WriteString(tt.line(6) + "\n"); err != nil {
				_ = f.Close()
				t.Fatal(err)
			}
			if err := f.Close(); err != nil {
				t.Fatal(err)
			}
			liveState := newAgentChatParseState()
			liveMessages, liveNext, err := tt.parseLive(path, safeOffset, liveState)
			if err != nil {
				t.Fatal(err)
			}
			if len(liveMessages) != 1 || liveMessages[0].Text != "partial-6" || liveNext <= safeOffset {
				t.Fatalf("live poll did not recover the record after the deadline prime: safe=%d next=%d messages=%#v stats=%+v", safeOffset, liveNext, liveMessages, liveState.lastRead)
			}

			seen := make(map[string]struct{}, len(messages))
			for _, message := range messages {
				seen[message.Text] = struct{}{}
			}
			cursor := next
			for page := 0; page < 4 && cursor > 0; page++ {
				olderState := newAgentChatParseStateWithPage(3, agentChatBatchBytesMax, -1)
				older, olderCursor, err := tt.parsePage(path, olderState, cursor, agentChatReadBudget{
					MaxBytes:   agentChatPageBytesMax,
					MaxRecords: agentChatPageRecordsMax,
					Deadline:   deadline.Add(time.Second),
					Clock:      func() time.Time { return deadline },
				})
				if err != nil {
					t.Fatal(err)
				}
				for _, message := range older {
					if _, exists := seen[message.Text]; exists {
						t.Fatalf("duplicate message after deadline retry: %q", message.Text)
					}
					seen[message.Text] = struct{}{}
				}
				if olderCursor >= cursor {
					t.Fatalf("retry cursor did not move backward: cursor=%d next=%d", cursor, olderCursor)
				}
				cursor = olderCursor
			}
			if len(seen) != 6 || cursor != 0 {
				t.Fatalf("partial deadline caused a pagination gap: seen=%v cursor=%d", seen, cursor)
			}
		})
	}
}
