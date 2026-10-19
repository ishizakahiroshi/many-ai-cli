package headless

import (
	"strings"
	"unicode/utf8"
)

// event.go is the normalized shape every headless provider is reduced to
// (元設計 10 節). A provider's own stream is parsed into these events and
// nothing else reaches the rest of many-ai-cli, so adding a provider cannot
// add a new kind of thing for the Hub or the UI to understand.
//
// The rendering (Line) is deliberately one plain text line per event: a
// headless session is delivered to the Hub through the same channel as PTY
// output, which means the browser draws it in a terminal. One line in, one
// line out, no escape sequences — see sanitizeEventText for why that last part
// is not optional.

// Event types. They are the subset of 元設計 10 節 that the first two formats
// can actually produce; a new format adds a parser, not a new vocabulary.
const (
	// EventRunStarted is the provider announcing its own run (model, session).
	EventRunStarted = "run.started"
	// EventAssistant is a message the model produced.
	EventAssistant = "assistant.message"
	// EventToolStarted / EventToolResult bracket one tool call.
	EventToolStarted = "tool.started"
	EventToolResult  = "tool.completed"
	// EventOutput is a line that carries no structure at all: the whole of
	// `format: text`, and the fallback for a line a structured parser could
	// not read.
	EventOutput = "output"
	// EventRunCompleted / EventRunFailed are the provider's own verdict. The
	// authoritative one is still the process exit code (元設計 11 節); these
	// only carry what the provider said about itself.
	EventRunCompleted = "run.completed"
	EventRunFailed    = "run.failed"
	// EventStderr is one line the provider wrote to stderr.
	EventStderr = "stderr"
)

// maxEventTextBytes bounds one rendered line. A provider that prints a whole
// file into one JSON string must not turn into a single multi-megabyte
// WebSocket frame; the raw log (when enabled) still has the untruncated bytes.
const maxEventTextBytes = 8192

// Event is one normalized thing that happened during a headless run.
type Event struct {
	// Type is one of the constants above.
	Type string
	// Tool is the tool name for the two tool events, empty otherwise.
	Tool string
	// Text is the human-readable payload: the message, the tool's argument
	// summary, the provider's own result line.
	Text string
	// IsError marks a tool result or a run verdict the provider itself called
	// a failure. It changes the rendered prefix, never the run's outcome.
	IsError bool
}

// Line renders the event as the single line a person reads in the session's
// terminal pane. EventOutput is rendered bare — `format: text` is supposed to
// look exactly like the CLI's own stdout — and everything else carries a short
// tag so a structured stream stays readable without a viewer.
func (e Event) Line() string {
	text := sanitizeEventText(e.Text)
	switch e.Type {
	case EventOutput:
		return text
	case EventRunStarted:
		return joinTag("[run]", text)
	case EventAssistant:
		return joinTag("[assistant]", text)
	case EventToolStarted:
		return joinTag("[tool]", strings.TrimSpace(sanitizeEventText(e.Tool)+" "+text))
	case EventToolResult:
		tag := "[tool result]"
		if e.IsError {
			tag = "[tool error]"
		}
		return joinTag(tag, strings.TrimSpace(sanitizeEventText(e.Tool)+" "+text))
	case EventRunCompleted:
		return joinTag("[result]", text)
	case EventRunFailed:
		return joinTag("[error]", text)
	case EventStderr:
		return joinTag("[stderr]", text)
	default:
		return joinTag("["+sanitizeEventText(e.Type)+"]", text)
	}
}

func joinTag(tag, text string) string {
	if text == "" {
		return tag
	}
	return tag + " " + text
}

// sanitizeEventText makes one provider-supplied string safe to write into a
// terminal.
//
// Everything below 0x20 except tab is dropped, and so is DEL. That includes
// ESC, which is the whole point: this text is drawn by xterm.js, and an escape
// sequence from a provider's output would not be text at all — it could move
// the cursor, clear the screen, or switch the pane into the alternate screen
// buffer, which leaves a residue nothing can clear
// (web/src/app/hub-marker-filter.ts 案 H is the same lesson from the other
// direction). A newline is dropped for the same reason one event is one line:
// the caller frames the lines, the content does not get to.
func sanitizeEventText(s string) string {
	if s == "" {
		return ""
	}
	var b strings.Builder
	b.Grow(len(s))
	for _, r := range s {
		switch {
		case r == '\t':
			b.WriteRune(' ')
		case r < 0x20 || r == 0x7f:
			// dropped
		case r == utf8.RuneError:
			// dropped: an invalid byte sequence, not a character
		default:
			b.WriteRune(r)
		}
	}
	out := strings.TrimRight(b.String(), " ")
	if len(out) > maxEventTextBytes {
		// Cut on a rune boundary so the truncated line is still valid UTF-8.
		cut := maxEventTextBytes
		for cut > 0 && !utf8.RuneStart(out[cut]) {
			cut--
		}
		out = out[:cut] + " …(truncated)"
	}
	return out
}

// firstLine returns the first non-empty line of s, which is how a multi-line
// payload (a tool result, a provider error) is summarised into one event.
func firstLine(s string) string {
	for _, line := range strings.Split(s, "\n") {
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			return trimmed
		}
	}
	return ""
}
