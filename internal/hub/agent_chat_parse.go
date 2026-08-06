package hub

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"

	"many-ai-cli/internal/sessionlog"
)

const (
	agentChatLineMax = 8 * 1024 * 1024
)

// agentChatMessage is the provider-neutral representation sent to the browser.
// It is deliberately smaller than either provider's native event schema.
type agentChatMessage struct {
	Role     string          `json:"role"`
	Kind     string          `json:"kind,omitempty"`
	Text     string          `json:"text,omitempty"`
	Thinking []string        `json:"thinking,omitempty"`
	Tools    []agentChatTool `json:"tools,omitempty"`
	TS       string          `json:"ts,omitempty"`
}

type agentChatTool struct {
	ID     string `json:"id,omitempty"`
	Name   string `json:"name"`
	Input  string `json:"input,omitempty"`
	Result string `json:"result,omitempty"`
}

type agentChatLineHandler func([]byte) error

// readAgentChatTail reads only complete newline-terminated records. An
// incomplete final record remains at offset so the next poll can read it again.
// A truncated file is frozen until its path changes; resetting to zero here
// would duplicate old transcript messages after a provider-side rewrite.
func readAgentChatTail(path string, offset int64, handle agentChatLineHandler) (int64, error) {
	if offset < 0 {
		offset = 0
	}
	info, err := os.Stat(path)
	if err != nil {
		return offset, err
	}
	if info.IsDir() {
		return offset, fmt.Errorf("transcript path is a directory")
	}
	if info.Size() < offset {
		return offset, nil
	}
	f, err := os.Open(path)
	if err != nil {
		return offset, err
	}
	defer f.Close()
	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return offset, err
	}

	reader := bufio.NewReaderSize(f, 64*1024)
	current := offset
	for {
		line, readErr := reader.ReadBytes('\n')
		if len(line) > 0 && len(line) <= agentChatLineMax && readErr == nil {
			current += int64(len(line))
			if err := handle(line[:len(line)-1]); err != nil {
				return current, err
			}
		} else if len(line) > 0 && readErr == nil {
			// Consume oversized records without exposing their contents.
			current += int64(len(line))
		} else if readErr == io.EOF {
			// The provider may still be writing this record. Do not advance.
			break
		} else if readErr != nil {
			return current, readErr
		}
		if readErr == io.EOF {
			break
		}
	}
	return current, nil
}

type claudeTranscriptLine struct {
	Type        string `json:"type"`
	SessionID   string `json:"sessionId"`
	Timestamp   string `json:"timestamp"`
	IsSidechain bool   `json:"isSidechain"`
	Message     struct {
		Role    string          `json:"role"`
		Content json.RawMessage `json:"content"`
	} `json:"message"`
}

type claudeContentBlock struct {
	Type      string          `json:"type"`
	Text      string          `json:"text"`
	Thinking  string          `json:"thinking"`
	ID        string          `json:"id"`
	Name      string          `json:"name"`
	Input     json.RawMessage `json:"input"`
	ToolUseID string          `json:"tool_use_id"`
	Content   json.RawMessage `json:"content"`
}

type agentChatParseState struct {
	messages []agentChatMessage
	tools    map[string][2]int
}

func newAgentChatParseState() *agentChatParseState {
	return &agentChatParseState{tools: make(map[string][2]int)}
}

func parseClaudeTranscript(path string, offset int64) ([]agentChatMessage, int64, error) {
	state := newAgentChatParseState()
	newOffset, err := readAgentChatTail(path, offset, func(line []byte) error {
		var record claudeTranscriptLine
		if err := json.Unmarshal(line, &record); err != nil {
			return nil
		}
		parseClaudeRecord(state, record)
		return nil
	})
	return state.messages, newOffset, err
}

func parseClaudeRecord(state *agentChatParseState, record claudeTranscriptLine) {
	if state == nil || record.Type != "user" && record.Type != "assistant" {
		return
	}
	var blocks []claudeContentBlock
	if err := json.Unmarshal(record.Message.Content, &blocks); err != nil {
		var text string
		if json.Unmarshal(record.Message.Content, &text) == nil && strings.TrimSpace(text) != "" {
			blocks = []claudeContentBlock{{Type: "text", Text: text}}
		} else {
			return
		}
	}
	if record.Message.Role == "" {
		record.Message.Role = record.Type
	}
	if record.Message.Role == "user" {
		parseClaudeUserRecord(state, record, blocks)
		return
	}
	if record.Message.Role == "assistant" {
		parseClaudeAssistantRecord(state, record, blocks)
	}
}

func parseClaudeUserRecord(state *agentChatParseState, record claudeTranscriptLine, blocks []claudeContentBlock) {
	var textParts []string
	for _, block := range blocks {
		switch block.Type {
		case "text":
			if strings.TrimSpace(block.Text) != "" {
				textParts = append(textParts, block.Text)
			}
		case "tool_result":
			attachClaudeToolResult(state, block)
		}
	}
	text := strings.TrimSpace(strings.Join(textParts, "\n"))
	if text == "" || isClaudeSyntheticUserText(text) {
		return
	}
	state.messages = append(state.messages, agentChatMessage{
		Role: "user",
		Kind: "text",
		Text: maskAgentChatText(text),
		TS:   record.Timestamp,
	})
}

func parseClaudeAssistantRecord(state *agentChatParseState, record claudeTranscriptLine, blocks []claudeContentBlock) {
	var textParts []string
	var thinking []string
	var tools []agentChatTool
	for _, block := range blocks {
		switch block.Type {
		case "text":
			if strings.TrimSpace(block.Text) != "" {
				textParts = append(textParts, block.Text)
			}
		case "thinking":
			if strings.TrimSpace(block.Thinking) != "" {
				thinking = append(thinking, maskAgentChatText(block.Thinking))
			}
		case "tool_use":
			tool := agentChatTool{
				ID:    block.ID,
				Name:  maskAgentChatText(block.Name),
				Input: summarizeAgentChatJSON(block.Input),
			}
			tools = append(tools, tool)
		}
	}
	text := strings.TrimSpace(strings.Join(textParts, "\n"))
	if record.IsSidechain {
		if text != "" {
			thinking = append(thinking, maskAgentChatText(text))
		}
		text = ""
	}
	if text == "" && len(thinking) == 0 && len(tools) == 0 {
		return
	}
	kind := "text"
	if record.IsSidechain {
		kind = "sidechain"
	} else if text == "" && len(tools) > 0 {
		kind = "tool"
	} else if text == "" {
		kind = "thinking"
	}
	messageIndex := len(state.messages)
	state.messages = append(state.messages, agentChatMessage{
		Role:     "assistant",
		Kind:     kind,
		Text:     maskAgentChatText(text),
		Thinking: thinking,
		Tools:    tools,
		TS:       record.Timestamp,
	})
	for toolIndex := range tools {
		if tools[toolIndex].ID != "" {
			state.tools[tools[toolIndex].ID] = [2]int{messageIndex, toolIndex}
		}
	}
}

func attachClaudeToolResult(state *agentChatParseState, block claudeContentBlock) {
	if state == nil {
		return
	}
	toolID := block.ToolUseID
	if toolID == "" {
		return
	}
	position, ok := state.tools[toolID]
	if !ok || position[0] < 0 || position[0] >= len(state.messages) {
		return
	}
	result := extractAgentChatText(block.Content)
	if result == "" {
		result = block.Text
	}
	if result == "" {
		return
	}
	tools := state.messages[position[0]].Tools
	if position[1] < 0 || position[1] >= len(tools) {
		return
	}
	tools[position[1]].Result = maskAgentChatText(result)
	state.messages[position[0]].Tools = tools
}

func isClaudeSyntheticUserText(text string) bool {
	return strings.Contains(text, "<command-name>") || strings.Contains(text, "<system-reminder>")
}

type codexRolloutLine struct {
	Type      string          `json:"type"`
	Timestamp string          `json:"timestamp"`
	Payload   json.RawMessage `json:"payload"`
}

type codexPayload struct {
	Type      string          `json:"type"`
	Role      string          `json:"role"`
	Text      string          `json:"text"`
	Message   string          `json:"message"`
	Name      string          `json:"name"`
	CallID    string          `json:"call_id"`
	Arguments json.RawMessage `json:"arguments"`
	Input     json.RawMessage `json:"input"`
	Output    json.RawMessage `json:"output"`
	Content   json.RawMessage `json:"content"`
	Summary   json.RawMessage `json:"summary"`
}

func parseCodexRollout(path string, offset int64) ([]agentChatMessage, int64, error) {
	state := newAgentChatParseState()
	newOffset, err := readAgentChatTail(path, offset, func(line []byte) error {
		var record codexRolloutLine
		if err := json.Unmarshal(line, &record); err != nil {
			return nil
		}
		parseCodexRecord(state, record)
		return nil
	})
	return state.messages, newOffset, err
}

func parseCodexRecord(state *agentChatParseState, record codexRolloutLine) {
	if state == nil || len(record.Payload) == 0 {
		return
	}
	var payload codexPayload
	if json.Unmarshal(record.Payload, &payload) != nil {
		return
	}
	ts := record.Timestamp
	switch record.Type {
	case "response_item":
		parseCodexResponseItem(state, payload, ts)
	case "event_msg":
		parseCodexEventMessage(state, payload, ts)
	}
}

func parseCodexResponseItem(state *agentChatParseState, payload codexPayload, ts string) {
	switch payload.Type {
	case "message":
		role := payload.Role
		if role != "user" && role != "assistant" {
			return
		}
		text, thinking, tools := parseCodexContent(payload.Content)
		if role == "user" && text == "" {
			return
		}
		if text == "" && len(thinking) == 0 && len(tools) == 0 {
			return
		}
		kind := "text"
		if role == "assistant" && text == "" && len(tools) > 0 {
			kind = "tool"
		} else if role == "assistant" && text == "" {
			kind = "thinking"
		}
		appendCodexMessage(state, agentChatMessage{
			Role:     role,
			Kind:     kind,
			Text:     maskAgentChatText(text),
			Thinking: thinking,
			Tools:    tools,
			TS:       ts,
		})
	case "reasoning":
		thinking := extractCodexSummary(payload.Summary)
		if len(thinking) == 0 {
			return
		}
		appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "thinking", Thinking: thinking, TS: ts})
	case "function_call", "custom_tool_call":
		name := payload.Name
		input := payload.Arguments
		if len(input) == 0 {
			input = payload.Input
		}
		if name == "" && len(input) == 0 {
			return
		}
		appendCodexMessage(state, agentChatMessage{
			Role:  "assistant",
			Kind:  "tool",
			Tools: []agentChatTool{{ID: payload.CallID, Name: maskAgentChatText(name), Input: summarizeAgentChatJSON(input)}},
			TS:    ts,
		})
	case "function_call_output", "custom_tool_call_output":
		attachCodexToolResult(state, payload)
	}
}

func parseCodexEventMessage(state *agentChatParseState, payload codexPayload, ts string) {
	switch payload.Type {
	case "user_message":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "user", Kind: "text", Text: maskAgentChatText(text), TS: ts})
		}
	case "agent_message":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "text", Text: maskAgentChatText(text), TS: ts})
		}
	case "agent_reasoning":
		text := payload.Message
		if text == "" {
			text = payload.Text
		}
		if strings.TrimSpace(text) != "" {
			appendCodexMessage(state, agentChatMessage{Role: "assistant", Kind: "thinking", Thinking: []string{maskAgentChatText(text)}, TS: ts})
		}
	}
}

func parseCodexContent(raw json.RawMessage) (string, []string, []agentChatTool) {
	var blocks []map[string]json.RawMessage
	if json.Unmarshal(raw, &blocks) != nil {
		text := extractAgentChatText(raw)
		return text, nil, nil
	}
	var textParts []string
	var thinking []string
	var tools []agentChatTool
	for _, block := range blocks {
		var kind string
		_ = json.Unmarshal(block["type"], &kind)
		switch kind {
		case "output_text", "input_text", "text":
			if text := extractAgentChatText(block["text"]); text != "" {
				textParts = append(textParts, text)
			} else if text := extractAgentChatText(block["content"]); text != "" {
				textParts = append(textParts, text)
			}
		case "summary_text", "reasoning", "reasoning_summary":
			if text := extractAgentChatText(block["text"]); text != "" {
				thinking = append(thinking, maskAgentChatText(text))
			}
		case "function_call", "custom_tool_call":
			var name, callID string
			_ = json.Unmarshal(block["name"], &name)
			_ = json.Unmarshal(block["call_id"], &callID)
			input := block["arguments"]
			if len(input) == 0 {
				input = block["input"]
			}
			tools = append(tools, agentChatTool{ID: callID, Name: maskAgentChatText(name), Input: summarizeAgentChatJSON(input)})
		}
	}
	return strings.TrimSpace(strings.Join(textParts, "\n")), thinking, tools
}

func appendCodexMessage(state *agentChatParseState, message agentChatMessage) {
	if state == nil {
		return
	}
	message.Text = maskAgentChatText(message.Text)
	for i := range message.Thinking {
		message.Thinking[i] = maskAgentChatText(message.Thinking[i])
	}
	for i := range message.Tools {
		message.Tools[i].Name = maskAgentChatText(message.Tools[i].Name)
		message.Tools[i].Input = maskAgentChatText(message.Tools[i].Input)
		if message.Tools[i].ID != "" {
			state.tools[message.Tools[i].ID] = [2]int{len(state.messages), i}
		}
	}
	state.messages = append(state.messages, message)
}

func attachCodexToolResult(state *agentChatParseState, payload codexPayload) {
	if state == nil || payload.CallID == "" {
		return
	}
	position, ok := state.tools[payload.CallID]
	if !ok || position[0] < 0 || position[0] >= len(state.messages) {
		return
	}
	result := extractAgentChatText(payload.Output)
	if result == "" {
		result = extractAgentChatText(payload.Content)
	}
	if result == "" {
		return
	}
	tools := state.messages[position[0]].Tools
	if position[1] >= len(tools) {
		return
	}
	tools[position[1]].Result = maskAgentChatText(result)
	state.messages[position[0]].Tools = tools
}

func extractCodexSummary(raw json.RawMessage) []string {
	var items []map[string]json.RawMessage
	if json.Unmarshal(raw, &items) == nil {
		out := make([]string, 0, len(items))
		for _, item := range items {
			text := extractAgentChatText(item["text"])
			if text == "" {
				text = extractAgentChatText(item["summary_text"])
			}
			if text != "" {
				out = append(out, maskAgentChatText(text))
			}
		}
		return out
	}
	if text := extractAgentChatText(raw); text != "" {
		return []string{maskAgentChatText(text)}
	}
	return nil
}

func extractAgentChatText(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	var text string
	if json.Unmarshal(raw, &text) == nil {
		return strings.TrimSpace(text)
	}
	var items []map[string]json.RawMessage
	if json.Unmarshal(raw, &items) == nil {
		var parts []string
		for _, item := range items {
			if value := extractAgentChatText(item["text"]); value != "" {
				parts = append(parts, value)
			} else if value := extractAgentChatText(item["content"]); value != "" {
				parts = append(parts, value)
			}
		}
		return strings.TrimSpace(strings.Join(parts, "\n"))
	}
	return ""
}

func summarizeAgentChatJSON(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	if text := extractAgentChatText(raw); text != "" {
		return maskAgentChatText(text)
	}
	var value any
	if json.Unmarshal(raw, &value) != nil {
		return maskAgentChatText(string(raw))
	}
	data, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	const maxSummary = 1200
	if len(data) > maxSummary {
		data = append(data[:maxSummary], "..."...)
	}
	return maskAgentChatText(string(data))
}

func maskAgentChatText(text string) string {
	return sessionlog.MaskSecrets(strings.TrimSpace(text))
}
