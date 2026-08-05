package wrapper

import "testing"

// Hub は未 ack の入力を「元の seq のまま」再送する。PTY 書き込み後・ack 到達前に
// WS が切れると同じ seq が再び届くため、wrapper は既に書いた分を握り潰さなければ
// 確定 \r が 2 回入り、後続プロンプトを誤承認する。
func TestInputSeqDuplicateDetection(t *testing.T) {
	ws := newWrapperSession(nil, 1, 0)

	if ws.inputSeqAlreadyProcessed(5) {
		t.Fatal("unprocessed seq was reported as duplicate")
	}

	ws.markInputSeqProcessed(5)

	if !ws.inputSeqAlreadyProcessed(5) {
		t.Fatal("resent seq was not detected as duplicate")
	}
	if !ws.inputSeqAlreadyProcessed(4) {
		t.Fatal("older seq was not detected as duplicate")
	}
	if ws.inputSeqAlreadyProcessed(6) {
		t.Fatal("newer seq was rejected as duplicate")
	}
}

// seq 未採番（0）は旧 Hub からのフレーム。追跡対象外なので常に書き込む。
func TestInputSeqZeroIsNeverDuplicate(t *testing.T) {
	ws := newWrapperSession(nil, 1, 0)
	ws.markInputSeqProcessed(9)

	if ws.inputSeqAlreadyProcessed(0) {
		t.Fatal("unsequenced frame was skipped as duplicate")
	}
	// 記録側も 0 を無視する（最大値を汚さない）。
	ws.markInputSeqProcessed(0)
	if ws.inputSeqAlreadyProcessed(10) {
		t.Fatal("marking seq 0 corrupted the processed watermark")
	}
}

// 記録は単調増加。順不同で届いても watermark は下がらない。
func TestInputSeqWatermarkNeverRegresses(t *testing.T) {
	ws := newWrapperSession(nil, 1, 0)
	ws.markInputSeqProcessed(12)
	ws.markInputSeqProcessed(3)

	if !ws.inputSeqAlreadyProcessed(12) {
		t.Fatal("watermark regressed after an out-of-order mark")
	}
	if ws.inputSeqAlreadyProcessed(13) {
		t.Fatal("watermark advanced past the highest processed seq")
	}
}
