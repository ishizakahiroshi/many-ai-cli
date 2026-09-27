package headless

import (
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

// Every format name config.yaml will accept must have a parser here. Without
// this, adding a name to the schema and forgetting the parser would be caught
// only at run time, by a launch that fails after the child was already created.
func TestEveryKnownFormatHasAParser(t *testing.T) {
	for _, format := range config.KnownHeadlessFormats() {
		if _, ok := ParserFor(format); !ok {
			t.Errorf("format %q is accepted by config but has no parser", format)
		}
	}
	if len(parsers) != len(config.KnownHeadlessFormats()) {
		t.Errorf("parsers has %d entries, config knows %d format names", len(parsers), len(config.KnownHeadlessFormats()))
	}
	if _, ok := ParserFor("made-up"); ok {
		t.Error("an unknown format must not resolve to a parser")
	}
}

// format: text is the pass-through. It is what lets any print-mode CLI become
// an unattended child from a definition alone, so it must add nothing.
func TestTextParserPassesLinesThrough(t *testing.T) {
	parser, _ := ParserFor(config.HeadlessFormatText)
	events := parser.Parse([]byte("building 3 targets"))
	if len(events) != 1 || events[0].Type != EventOutput {
		t.Fatalf("events = %+v, want one output event", events)
	}
	if got := events[0].Line(); got != "building 3 targets" {
		t.Errorf("Line() = %q, want the line unchanged", got)
	}
	// An empty line is part of a CLI's own layout and is kept.
	if events := parser.Parse([]byte("")); len(events) != 1 || events[0].Line() != "" {
		t.Errorf("empty line = %+v, want one empty output event", events)
	}
}

// The fixture is synthetic: the shapes are taken from `claude --help`'s
// description of stream-json, not copied from a real session.
func TestClaudeStreamJSONParser(t *testing.T) {
	parser, _ := ParserFor(config.HeadlessFormatClaudeStreamJSON)
	lines := []string{
		`{"type":"system","subtype":"init","session_id":"11111111-2222-3333-4444-555555555555","model":"test-model"}`,
		`{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Looking at the file."},{"type":"tool_use","id":"tool_1","name":"Read","input":{"file_path":"/work/example/main.go"}}]}}`,
		`{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_1","content":"package main\nfunc main() {}"}]}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta"}}`,
		`{"type":"result","subtype":"success","is_error":false,"result":"Done: one file read."}`,
	}
	var got []Event
	for _, line := range lines {
		got = append(got, parser.Parse([]byte(line))...)
	}
	want := []struct{ typ, contains string }{
		{EventRunStarted, "test-model"},
		{EventAssistant, "Looking at the file."},
		{EventToolStarted, "/work/example/main.go"},
		{EventToolResult, "package main"},
		{EventRunCompleted, "Done: one file read."},
	}
	if len(got) != len(want) {
		t.Fatalf("got %d events %+v, want %d", len(got), got, len(want))
	}
	for i, w := range want {
		if got[i].Type != w.typ {
			t.Errorf("event %d type = %q, want %q", i, got[i].Type, w.typ)
		}
		if !strings.Contains(got[i].Line(), w.contains) {
			t.Errorf("event %d line = %q, want it to contain %q", i, got[i].Line(), w.contains)
		}
	}
	if tool := got[2].Tool; tool != "Read" {
		t.Errorf("tool = %q, want Read", tool)
	}
}

// 元設計 10 節: one bad line must not fail a run. It falls back to plain text so
// a CLI that dies before it streams anything still says why.
func TestClaudeStreamJSONParserFallsBackOnBrokenLines(t *testing.T) {
	parser, _ := ParserFor(config.HeadlessFormatClaudeStreamJSON)
	events := parser.Parse([]byte(`{"type":"assistant","message":{`))
	if len(events) != 1 || events[0].Type != EventOutput {
		t.Fatalf("events = %+v, want the unparsable line as plain output", events)
	}
	if !strings.Contains(events[0].Line(), `"type":"assistant"`) {
		t.Errorf("line = %q, want the original text kept", events[0].Line())
	}
	// A blank line carries nothing at all.
	if events := parser.Parse([]byte("   ")); len(events) != 0 {
		t.Errorf("blank line = %+v, want no events", events)
	}
	// An error verdict is marked as one.
	events = parser.Parse([]byte(`{"type":"result","subtype":"error_during_execution","is_error":true,"error":"quota exhausted"}`))
	if len(events) != 1 || events[0].Type != EventRunFailed {
		t.Fatalf("events = %+v, want a failed run event", events)
	}
	if !strings.Contains(events[0].Line(), "quota exhausted") {
		t.Errorf("line = %q, want the provider's own message", events[0].Line())
	}
}

// The rendered line is drawn by xterm.js. An escape sequence in provider output
// would not be text there — it could clear the pane or switch it to the
// alternate screen, which nothing can undo (CLAUDE.md 設計原則: 案 H).
func TestEventLineStripsControlSequences(t *testing.T) {
	line := Event{Type: EventAssistant, Text: "red \x1b[31malert\x1b[0m\x07 done\ttab"}.Line()
	if strings.ContainsAny(line, "\x1b\x07") {
		t.Fatalf("line = %q, want no escape or bell characters", line)
	}
	if !strings.Contains(line, "alert") || !strings.Contains(line, "done tab") {
		t.Errorf("line = %q, want the readable text kept with tabs as spaces", line)
	}
	if !strings.HasPrefix(line, "[assistant] ") {
		t.Errorf("line = %q, want the assistant tag", line)
	}
	// One event is one line, whatever the content says.
	multi := Event{Type: EventOutput, Text: "first\nsecond"}.Line()
	if strings.Contains(multi, "\n") {
		t.Errorf("line = %q, want a single line", multi)
	}
	// A pathological payload is bounded.
	long := Event{Type: EventOutput, Text: strings.Repeat("x", maxEventTextBytes*2)}.Line()
	if len(long) > maxEventTextBytes+32 {
		t.Errorf("len(line) = %d, want it truncated near %d", len(long), maxEventTextBytes)
	}
}

// BuildArgv keeps the definition's flags first and the launch's own arguments
// last, and puts an argv-delivered prompt where a variadic flag cannot swallow
// it.
func TestBuildArgv(t *testing.T) {
	def := config.HeadlessDef{Args: []string{"-p", "--output-format", "stream-json"}, PromptVia: config.HeadlessPromptViaStdin}
	launch := []string{"--model", "test-model", "--allowedTools", "Read", "Edit"}
	got := BuildArgv(def, launch, "do the thing")
	want := []string{"-p", "--output-format", "stream-json", "--model", "test-model", "--allowedTools", "Read", "Edit"}
	if strings.Join(got, "\x00") != strings.Join(want, "\x00") {
		t.Fatalf("argv = %v, want %v (stdin prompts never appear in argv)", got, want)
	}

	argDef := config.HeadlessDef{Args: []string{"run", "--format", "json"}, PromptVia: config.HeadlessPromptViaArg}
	got = BuildArgv(argDef, launch, "do the thing")
	if got[3] != "do the thing" {
		t.Fatalf("argv = %v, want the prompt right after the definition's flags", got)
	}
	if got[len(got)-1] != "Edit" {
		t.Fatalf("argv = %v, want the launch arguments last", got)
	}
}
