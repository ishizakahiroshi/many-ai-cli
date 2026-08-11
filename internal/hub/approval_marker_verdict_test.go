package hub

import (
	"strings"
	"testing"
)

// テスト内でもマーカーの実文字列を連続して書かない（approval_marker_verdict.go の
// 定数コメント参照）。ブロック組み立ては必ずこのヘルパを通す。
func markerBlock(inner ...string) string {
	return approvalMarkerOpen + "\n" + strings.Join(inner, "\n") + "\n" + approvalMarkerClose
}

func TestClassifyApprovalMarkerBlockAcceptsValidForms(t *testing.T) {
	cases := []struct {
		name  string
		block string
	}{
		{"single question", markerBlock(
			"path-exists の挙動をどうしますか？",
			"1. 実在判定可にする (Recommended)",
			"2. 許可リストを維持",
			"N. User specifies",
		)},
		{"batch with block-through numbering", markerBlock(
			"前置きの説明です。",
			"Q1 最初の質問ですか？",
			" 1. 案 A (Recommended)",
			" 2. 案 B",
			" N. User specifies",
			"Q2 次の質問ですか？",
			" 3. 案 C (Recommended)",
			" 4. 案 D",
			" N. User specifies",
		)},
		{"batch with per-question renumbering", markerBlock(
			"Q1 最初の質問ですか？",
			" 1. 案 A (Recommended)",
			" 2. 案 B",
			" N. User specifies",
			"Q2 次の質問ですか？",
			" 1. 案 C (Recommended)",
			" 2. 案 D",
			" N. User specifies",
		)},
		{"yes no form", markerBlock("Proceed with this change? (Y:1/N:0)")},
		{"yes no full width", markerBlock("対象機能を無効化しますか？ （Y：1／N：0）")},
		{"multi select", markerBlock(
			"#multi どの機能を有効にしますか？",
			"1. 機能 A",
			"2. 機能 B",
			"3. 機能 C",
		)},
		{"no numbered options at all", markerBlock(
			"これは選択肢を持たない説明だけのブロックです。",
		)},
		{"uneven per-question groups", markerBlock(
			"Q1 最初の質問ですか？",
			" 1. 案 A",
			"Q2 次の質問ですか？",
			" 1. 案 B",
			" 2. 案 C",
			" 3. 案 D",
		)},
		// approval-rules.md が禁じるのは「同一質問内の重複」だけ。質問をまたいだ
		// 番号の再利用は許容されている（実履歴で 3 件確認済み）。
		{"cross-question overlapping numbers", markerBlock(
			"Q1 最初の質問ですか？",
			" 1. 案 A",
			" 2. 案 B",
			" 3. 案 C",
			"Q2 次の質問ですか？",
			" 3. 案 C を維持",
			" 4. 案 D",
			" 5. 案 E",
		)},
		// 前置きの地の文に行頭数字が来ても弾かない。
		// 実例: 前置き中の IP アドレスや版数を擬似選択肢として拾ってしまう事故があった。
		{"preamble line starting with a number", markerBlock(
			"3. 案 C は前回採用済みです。",
			"どの方式で進めますか？",
			"1. 案 A (Recommended)",
			"2. 案 B",
			"N. User specifies",
		)},
		// 罫線 1 文字（範囲表記など）は正常扱い。しきい値 3 未満で誤爆させない。
		{"label with a single box drawing dash", markerBlock(
			"どの範囲を対象にしますか？",
			"1. 10─20 件だけ (Recommended)",
			"2. 全件",
			"N. User specifies",
		)},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if reason := classifyApprovalMarkerBlock(tc.block); reason != "" {
				t.Fatalf("valid block rejected: reason=%q", reason)
			}
		})
	}
}

func TestClassifyApprovalMarkerBlockRejectsCorruptForms(t *testing.T) {
	cases := []struct {
		name       string
		block      string
		wantReason string
	}{
		{
			// 2026-07-31 の実障害そのもの。Q1 の選択肢 3 行が VT ミラーで失われ、
			// Q2 の 3./4. だけが残って「選択肢が 3 から始まるパネル」になった。
			name: "leading question options lost",
			block: markerBlock(
				"計画上、判断が必要な項目があります。",
				"Q1 最初の質問ですか？",
				"Q2 次の質問ですか？",
				" 3. 案 C (Recommended)",
				" 4. 案 D",
				" N. User specifies",
			),
			wantReason: "option_start",
		},
		{
			name: "single surviving option",
			block: markerBlock(
				"計画上、判断が必要な項目があります。",
				" 4. 案 D",
				" N. User specifies",
			),
			wantReason: "option_start",
		},
		{
			// 見出しが失われて 2 問分の選択肢が 1 グループへ融合した形。
			name: "options of two questions fused without headings",
			block: markerBlock(
				"判断が必要な項目があります。",
				" 1. 案 A",
				" 1. 案 B",
				" 2. 案 C",
				" 3. 案 D",
			),
			wantReason: "duplicate_option",
		},
		{
			name: "duplicate without renumbering",
			block: markerBlock(
				"Q1 質問ですか？",
				" 1. 案 A",
				" 2. 案 B",
				" 2. 案 B の再描画残骸",
				" 3. 案 C",
			),
			wantReason: "duplicate_option",
		},
		{
			name: "fused blocks leak an open marker",
			block: approvalMarkerOpen + "\n" +
				"Q1 前の世代の質問ですか？\n" +
				" 1. 案 A\n" +
				approvalMarkerOpen + "\n" +
				"Q1 今の世代の質問ですか？\n" +
				" 1. 案 A\n" +
				approvalMarkerClose,
			wantReason: "marker_leak",
		},
		{
			// 2026-08-01 の実障害。resize 直後の reflow で TUI コンポーザの枠線が
			// 選択肢ラベルへ重なった。マーカーの対も番号構造も無傷なので番号検査では
			// 捕まらず、Web 側の consumed sig / blockSig が一致しなくなって
			// 回答済みの一括承認バーが復活した。
			name: "option label overwritten by composer rule",
			block: markerBlock(
				"Q1 最初の質問ですか？",
				" 1. 案 A の説明が途中で "+strings.Repeat("─", 26),
				" 2. 案 B",
				" N. User specifies",
			),
			wantReason: "box_rule",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := classifyApprovalMarkerBlock(tc.block)
			if got != tc.wantReason {
				t.Fatalf("reason = %q, want %q", got, tc.wantReason)
			}
		})
	}
}

// 抑止しても approvalMarkerSig を書き換えないこと。書き換えると、破損ブロックの直後に
// 届いた正常なブロックが dedupe で潰れて承認が二度と出なくなる。
func TestMaybeBroadcastApprovalMarkerSuppressesCorruptWithoutConsumingSig(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")

	corrupt := &approvalMarkerBlock{Block: markerBlock(
		"Q2 次の質問ですか？",
		" 3. 案 C",
		" 4. 案 D",
	)}
	corrupt.Sig = approvalMarkerSignature(corrupt.Block)

	if s.maybeBroadcastApprovalMarker(1, corrupt, ses.lastOutputAt) {
		t.Fatal("corrupt marker was broadcast")
	}
	if ses.approvalMarkerSig != "" {
		t.Fatalf("approvalMarkerSig was consumed by a corrupt block: %q", ses.approvalMarkerSig)
	}

	healthy := &approvalMarkerBlock{Block: markerBlock(
		"Q1 最初の質問ですか？",
		" 1. 案 A",
		" 2. 案 B",
	)}
	healthy.Sig = approvalMarkerSignature(healthy.Block)

	if !s.maybeBroadcastApprovalMarker(1, healthy, ses.lastOutputAt) {
		t.Fatal("healthy marker was not broadcast after a corrupt one")
	}
	if ses.approvalMarkerSig != healthy.Sig {
		t.Fatalf("approvalMarkerSig = %q, want %q", ses.approvalMarkerSig, healthy.Sig)
	}
}
