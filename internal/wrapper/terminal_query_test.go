package wrapper

import (
	"bytes"
	"testing"
)

func TestProcessTerminalQueriesDSRCursorPosition(t *testing.T) {
	state := newTerminalQueryState()
	forward, replies, next := processTerminalQueries(state, []byte("hello\x1b[6nworld"))
	if string(forward) != "helloworld" {
		t.Fatalf("forward = %q, want helloworld", forward)
	}
	if string(replies) != "\x1b[1;6R" {
		// "hello" advances col to 6 (1-based after 5 chars → col=6)
		t.Fatalf("replies = %q, want CPR at col after hello", replies)
	}
	if next.row != 1 || next.col != 11 { // helloworld = 10 chars → col 11
		t.Fatalf("cursor = %d;%d, want 1;11", next.row, next.col)
	}
	if len(next.carry) != 0 {
		t.Fatalf("carry = %q, want empty", next.carry)
	}
}

func TestProcessTerminalQueriesDSRAfterCUP(t *testing.T) {
	state := newTerminalQueryState()
	forward, replies, next := processTerminalQueries(state, []byte("\x1b[12;4H\x1b[6n"))
	if string(forward) != "\x1b[12;4H" {
		t.Fatalf("forward = %q", forward)
	}
	if string(replies) != "\x1b[12;4R" {
		t.Fatalf("replies = %q, want CPR reflecting CUP", replies)
	}
	if next.row != 12 || next.col != 4 {
		t.Fatalf("cursor = %d;%d, want 12;4", next.row, next.col)
	}
}

func TestProcessTerminalQueriesPrimaryDA(t *testing.T) {
	cases := []struct {
		name string
		in   string
	}{
		{name: "bare", in: "\x1b[c"},
		{name: "zero", in: "\x1b[0c"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, replies, _ := processTerminalQueries(newTerminalQueryState(), []byte(tc.in))
			if string(replies) != "\x1b[?1;2c" {
				t.Fatalf("replies = %q, want primary DA", replies)
			}
		})
	}
}

func TestProcessTerminalQueriesIgnoresSecondaryDA(t *testing.T) {
	in := []byte("\x1b[>c")
	forward, replies, _ := processTerminalQueries(newTerminalQueryState(), in)
	if string(forward) != string(in) {
		t.Fatalf("secondary DA should pass through, got %q", forward)
	}
	if len(replies) != 0 {
		t.Fatalf("replies = %q, want none", replies)
	}
}

func TestProcessTerminalQueriesDSRStatus(t *testing.T) {
	_, replies, _ := processTerminalQueries(newTerminalQueryState(), []byte("\x1b[5n"))
	if string(replies) != "\x1b[0n" {
		t.Fatalf("replies = %q, want CSI 0n", replies)
	}
}

func TestProcessTerminalQueriesSplitAcrossChunks(t *testing.T) {
	state := newTerminalQueryState()
	forward1, replies1, state := processTerminalQueries(state, []byte("pre\x1b[6"))
	if string(forward1) != "pre" {
		t.Fatalf("forward1 = %q", forward1)
	}
	if len(replies1) != 0 {
		t.Fatalf("replies1 = %q", replies1)
	}
	if !bytes.Equal(state.carry, []byte("\x1b[6")) {
		t.Fatalf("carry = %q", state.carry)
	}
	forward2, replies2, state := processTerminalQueries(state, []byte("npost"))
	if string(forward2) != "post" {
		t.Fatalf("forward2 = %q", forward2)
	}
	if string(replies2) != "\x1b[1;4R" { // "pre" → col 4
		t.Fatalf("replies2 = %q", replies2)
	}
	if len(state.carry) != 0 {
		t.Fatalf("carry after complete = %q", state.carry)
	}
}

func TestProcessTerminalQueriesMultipleQueries(t *testing.T) {
	in := []byte("\x1b[6n\x1b[c\x1b[6n")
	forward, replies, _ := processTerminalQueries(newTerminalQueryState(), in)
	if len(forward) != 0 {
		t.Fatalf("forward = %q, want empty", forward)
	}
	want := "\x1b[1;1R\x1b[?1;2c\x1b[1;1R"
	if string(replies) != want {
		t.Fatalf("replies = %q, want %q", replies, want)
	}
}

func TestProcessTerminalQueriesPlainTextUnchanged(t *testing.T) {
	in := []byte("Standby…\nready")
	forward, replies, next := processTerminalQueries(newTerminalQueryState(), in)
	if string(forward) != string(in) {
		t.Fatalf("forward mutated: %q", forward)
	}
	if len(replies) != 0 {
		t.Fatalf("unexpected replies %q", replies)
	}
	if next.row != 2 {
		t.Fatalf("row = %d after LF, want 2", next.row)
	}
}
