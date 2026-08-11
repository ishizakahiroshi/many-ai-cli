package hub

import (
	"bytes"
	"testing"
)

// TestReattachReplayGap は「切断中に落ちたぶんだけ」を replay から切り出せること、
// および重複配信・過剰配信に倒れないことを確認する。
func TestReattachReplayGap(t *testing.T) {
	// wrapper が PTY から読んだ全ストリーム（累計 20 バイト）
	full := []byte("ABCDEFGHIJKLMNOPQRST")
	// wrapper の replay リング（直近 8 バイト = 末尾 "MNOPQRST"）
	replay := full[len(full)-8:]

	tests := []struct {
		name         string
		replay       []byte
		wrapperTotal int64
		hubSeen      int64
		want         []byte
	}{
		{
			// 取りこぼし 3 バイト（Hub は 17 バイトまで受信済み）→ 末尾 3 バイトだけ
			name: "落ちたぶんだけ切り出す", replay: replay,
			wrapperTotal: 20, hubSeen: 17, want: []byte("RST"),
		},
		{
			// 取りこぼしゼロ = 全部届いていた → 何も送らない（二重描画の防止）
			name: "欠落なしなら無配信", replay: replay,
			wrapperTotal: 20, hubSeen: 20, want: nil,
		},
		{
			// replay 窓（8）を超える欠落 → 手元にある最大限（replay 全量）
			name: "replay 窓を超える欠落は全量", replay: replay,
			wrapperTotal: 20, hubSeen: 5, want: replay,
		},
		{
			// 欠落がちょうど replay 窓と同じ → 全量
			name: "欠落が replay 窓ちょうど", replay: replay,
			wrapperTotal: 20, hubSeen: 12, want: replay,
		},
		{
			// 累計未申告の旧 wrapper → 当て推量せず無配信
			name: "累計未申告なら無配信", replay: replay,
			wrapperTotal: 0, hubSeen: 12, want: nil,
		},
		{
			// Hub の方が進んでいる（異常系）→ 無配信
			name: "Hub の累計が先行していたら無配信", replay: replay,
			wrapperTotal: 20, hubSeen: 25, want: nil,
		},
		{
			name: "replay が空なら無配信", replay: nil,
			wrapperTotal: 20, hubSeen: 5, want: nil,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := reattachReplayGap(tt.replay, tt.wrapperTotal, tt.hubSeen)
			if !bytes.Equal(got, tt.want) {
				t.Fatalf("reattachReplayGap = %q, want %q", got, tt.want)
			}
		})
	}
}

// TestReattachReplayGapReconstructsStream は「Hub が受信済みのぶん + gap」を
// つないだ結果が、wrapper の元ストリームと完全に一致する（欠落も重複もない）
// ことを確認する。これが崩れると xterm.js の描画座標がずれる。
func TestReattachReplayGapReconstructsStream(t *testing.T) {
	full := []byte("0123456789abcdefghijklmnopqrstuvwxyz")
	const replayWindow = 12

	for hubSeen := 0; hubSeen <= len(full); hubSeen++ {
		replay := full
		if len(full) > replayWindow {
			replay = full[len(full)-replayWindow:]
		}
		gap := reattachReplayGap(replay, int64(len(full)), int64(hubSeen))

		missing := len(full) - hubSeen
		if missing > replayWindow {
			// 復元不能な穴があるケース。gap は replay 全量になり、
			// 連結しても元ストリームには戻らない（想定内）。
			if !bytes.Equal(gap, replay) {
				t.Fatalf("hubSeen=%d: gap = %q, want full replay %q", hubSeen, gap, replay)
			}
			continue
		}
		got := append(append([]byte(nil), full[:hubSeen]...), gap...)
		if !bytes.Equal(got, full) {
			t.Fatalf("hubSeen=%d: 復元結果 = %q, want %q", hubSeen, got, full)
		}
	}
}
