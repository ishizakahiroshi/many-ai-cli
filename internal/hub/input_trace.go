package hub

// input_trace.go: 「Web から送信した文字列が CLI 入力欄に滞留し、もう一度 Enter を
// 押さないと送信されない」症状（2026-08-04 調査）の観測専用コード。
//
// 現行のチャット送信は 2 通の pty_input に分かれる（web/src/app.ts buildBodySubmitPart）:
//   1 通目 = \x1b[200~ 本文 \x1b[201~（確定 \r を含まない）
//   2 通目 = 単独の "\r"（UI 側が PTY 出力の静止を見てから 1 回だけ撃つ）
//
// hub.log は正常な pty_input を一切記録しないため、症状発生時に「確定 \r が Hub へ
// 届いたのかどうか」すら履歴から判定できなかった（保留経路に落ちたときだけ WARN が
// 出る）。ここで input_trace を刻むことで、UI が撃ったか → Hub が受けたか →
// Hub が wrapper へ送ったか を再現 1 回で切り分ける。
//
// 調査完了後に撤去する前提の一時計測コードであり、恒久機能ではない。

import (
	"encoding/hex"
	"strings"
)

// inputShape は pty_input のペイロード形状を分類する。滞留症状では
// paste_body だけが届いて bare_cr が来ない、という並びになるはず。
func inputShape(text string) string {
	switch {
	case text == "":
		return "empty"
	case text == "\r":
		return "bare_cr"
	case strings.HasSuffix(text, bracketedPasteEnd):
		return "paste_body"
	case strings.HasSuffix(text, bracketedPasteEnd+"\r"):
		return "paste_body_cr"
	case strings.HasSuffix(text, "\r"):
		return "body_cr"
	default:
		return "other"
	}
}

// inputTailHex は末尾 n バイトを hex で返す。ペースト終端（1b5b3230317e）と
// 確定 \r（0d）を目視で区別するために使う。
func inputTailHex(text string, n int) string {
	b := []byte(text)
	if len(b) > n {
		b = b[len(b)-n:]
	}
	return hex.EncodeToString(b)
}
