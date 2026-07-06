package wrapper

import (
	"bytes"
	"testing"
)

// cursor-agent の TUI は「制御文字＋本文」の同一チャンク入力を丸ごと捨てるため、
// 先頭 Ctrl+U(0x15) だけを別書き込みへ分離する（2026-07-03 実測・v2026.07.01）。
func TestSplitLeadingClearControl(t *testing.T) {
	tests := []struct {
		name     string
		provider string
		data     []byte
		wantLead []byte
		wantRest []byte
	}{
		{
			name:     "cursor-agent の Ctrl+U 前置入力は分離する",
			provider: "cursor-agent",
			data:     []byte("\x15今日は何日 \r"),
			wantLead: []byte{0x15},
			wantRest: []byte("今日は何日 \r"),
		},
		{
			name:     "cursor-agent でも Ctrl+U 単独は分離しない（既存経路のまま）",
			provider: "cursor-agent",
			data:     []byte{0x15},
			wantLead: nil,
			wantRest: []byte{0x15},
		},
		{
			name:     "cursor-agent でも Ctrl+U 前置なしは分離しない",
			provider: "cursor-agent",
			data:     []byte("hello\r"),
			wantLead: nil,
			wantRest: []byte("hello\r"),
		},
		{
			name:     "claude は Ctrl+U 前置でも分離しない（挙動不変）",
			provider: "claude",
			data:     []byte("\x15hello\r"),
			wantLead: nil,
			wantRest: []byte("\x15hello\r"),
		},
		{
			name:     "codex は Ctrl+U 前置でも分離しない（挙動不変）",
			provider: "codex",
			data:     []byte("\x15hello\r"),
			wantLead: nil,
			wantRest: []byte("\x15hello\r"),
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			lead, rest := splitLeadingClearControl(tt.provider, tt.data)
			if !bytes.Equal(lead, tt.wantLead) {
				t.Errorf("lead = %q, want %q", lead, tt.wantLead)
			}
			if !bytes.Equal(rest, tt.wantRest) {
				t.Errorf("rest = %q, want %q", rest, tt.wantRest)
			}
		})
	}
}
