package hub

import (
	"fmt"
	"strings"
	"testing"
)

func TestExtractApprovalMarkerBlockMultiline(t *testing.T) {
	lines := []string{
		"thinking...",
		"[MANY-AI-CLI]",
		"Q1 scope?",
		" 1. Model only (Recommended)",
		" 2. All selectors",
		" N. User specifies",
		"[/MANY-AI-CLI]",
	}

	got := extractApprovalMarkerBlock(lines)
	if got == nil {
		t.Fatal("extractApprovalMarkerBlock returned nil")
	}
	want := "[MANY-AI-CLI]\nQ1 scope?\n 1. Model only (Recommended)\n 2. All selectors\n N. User specifies\n[/MANY-AI-CLI]"
	if got.Block != want {
		t.Fatalf("block = %q, want %q", got.Block, want)
	}
	if got.Sig == "" {
		t.Fatal("sig is empty")
	}
}

func TestExtractApprovalMarkerBlockLastCompleteBlock(t *testing.T) {
	lines := []string{
		"[MANY-AI-CLI]",
		"first?",
		"1. Yes",
		"[/MANY-AI-CLI]",
		"noise",
		"[MANY-AI-CLI]",
		"second?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	}

	got := extractApprovalMarkerBlock(lines)
	if got == nil {
		t.Fatal("extractApprovalMarkerBlock returned nil")
	}
	if got.Block != "[MANY-AI-CLI]\nsecond?\n1. Yes\n2. No\n[/MANY-AI-CLI]" {
		t.Fatalf("block = %q", got.Block)
	}
}

func TestExtractApprovalMarkerBlockIgnoresIncomplete(t *testing.T) {
	lines := []string{
		"[MANY-AI-CLI]",
		"Q1 missing close?",
		"1. Yes",
	}

	if got := extractApprovalMarkerBlock(lines); got != nil {
		t.Fatalf("extractApprovalMarkerBlock = %+v, want nil", got)
	}
}

// TestApprovalMarkerNotLeakedAcrossResize は、resize をまたいだ再描画が marker_leak に
// 化けないことを検証する。
//
// 実測（2026-08-02 セッション #4 の 26 分を jsonl から再生）: Web の承認バー・告知バナーが
// 出入りするたび #terminal-area の高さが変わって pty_resize が飛び、rows が 10〜35 の間を
// 秒間数回変動していた。resize 前の世代が VT ミラーの scrollback に残ると、抽出ウィンドウ内で
// 開始マーカーが新旧 2 本並び、非貪欲マッチが OPEN … OPEN … CLOSE を 1 ブロックとして拾う。
// 26 分間で承認マーカー 16 件中 15 件が抑止され、うち 10 件がこの形だった。
func TestApprovalMarkerNotLeakedAcrossResize(t *testing.T) {
	vt := newVTBuffer(40, 10)
	// resize 前の世代の残骸。終了マーカーだけが画面外へ失われた状態を模す。
	vt.scrollback = []string{approvalMarkerOpen, "Q1 proceed?", " 1. Yes"}

	vt.Resize(40, 14) // SIGWINCH

	// TUI が全画面を描き直す。
	vt.Write([]byte("[MANY-AI-CLI]\r\nQ1 proceed?\r\n 1. Yes\r\n 2. No\r\n N. User specifies\r\n[/MANY-AI-CLI]\r\n"))

	marker := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil {
		t.Fatal("marker not extracted after resize redraw")
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		t.Fatalf("classify = %q, want ok (resize-straddling redraw must not look corrupt)", reason)
	}
}

// TestExtractApprovalMarkerBlockTakesLatestGeneration は、描き直しで同じ質問が
// 2 世代残っているとき、最新世代だけが抽出されることを検証する。
//
// TUI は画面をスクロールさせながら描き直すため、途中まで描かれた世代が scrollback 側に
// 確定して残る。「最初の OPEN から最初の CLOSE まで」を取ると新旧をまたいだブロックに
// なり、内側に OPEN を抱えたまま自分で marker_leak と判定して承認パネルを握り潰していた。
// 実測（2026-08-13 / approval-corrupt ダンプ 138 件の replay）: marker_leak 74 件のうち
// 64 件がこの抽出だけで正常なブロックへ戻る。
func TestExtractApprovalMarkerBlockTakesLatestGeneration(t *testing.T) {
	lines := []string{
		"  直前の出力",
		approvalMarkerOpen,
		"  前置きの 1 行目",
		"",
		// ここから描き直された世代。
		approvalMarkerOpen,
		"  前置きの 1 行目",
		"",
		"  Q1 進めますか?",
		"  1. はい (Recommended)",
		"  2. いいえ",
		"   N. User specifies",
		approvalMarkerClose,
	}

	marker := extractApprovalMarkerBlock(lines)
	if marker == nil {
		t.Fatal("extractApprovalMarkerBlock = nil, want the latest generation")
	}
	if n := strings.Count(marker.Block, approvalMarkerOpen); n != 1 {
		t.Fatalf("OPEN が %d 個, want 1 (古い世代を巻き込んでいる): %q", n, marker.Block)
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		t.Fatalf("classify = %q, want ok (最新世代は正常なブロック)", reason)
	}
}

func TestMaybeBroadcastApprovalMarkerDedupesSameBlock(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	marker := extractApprovalMarkerBlock([]string{
		"[MANY-AI-CLI]",
		"Q1 proceed?",
		"1. Yes",
		"[/MANY-AI-CLI]",
	})
	if marker == nil {
		t.Fatal("marker nil")
	}

	if !s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("first marker should be accepted")
	}
	if ses.approvalMarkerSig != marker.Sig {
		t.Fatalf("approvalMarkerSig = %q, want %q", ses.approvalMarkerSig, marker.Sig)
	}
	if s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("same marker should be deduped")
	}
}

// Grok 差分再描画で色 CSI だけが揺れた同一質問は、同一 sig として dedupe する。
func TestApprovalMarkerSignatureIgnoresAnsiFlicker(t *testing.T) {
	plain := "[MANY-AI-CLI]\nスターター取込の本線？\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	// 実測 s22: 同一 clean 本文でも \x1b[48;2;... の余白/色だけが変わる
	coloredA := "[MANY-AI-CLI]\n\x1b[39m\x1b[48;2;1;1;1mスターター取込の本線？\x1b[K\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	coloredB := "[MANY-AI-CLI]\n\x1b[39m\x1b[48;2;20;20;20mスターター取込の本線？\x1b[K\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	sigPlain := approvalMarkerSignature(plain)
	sigA := approvalMarkerSignature(coloredA)
	sigB := approvalMarkerSignature(coloredB)
	if sigPlain == "" || sigA == "" {
		t.Fatal("empty signature")
	}
	if sigPlain != sigA || sigA != sigB {
		t.Fatalf("ANSI flicker must share sig: plain=%q a=%q b=%q", sigPlain, sigA, sigB)
	}

	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	first := &approvalMarkerBlock{Block: coloredA, Sig: sigA}
	second := &approvalMarkerBlock{Block: coloredB, Sig: sigB}
	if !s.maybeBroadcastApprovalMarker(1, first, ses.lastOutputAt) {
		t.Fatal("first colored marker should broadcast")
	}
	if s.maybeBroadcastApprovalMarker(1, second, ses.lastOutputAt) {
		t.Fatal("ANSI-only variant must be deduped (no rebroadcast)")
	}
}

func TestApprovalMarkerSignatureDiffersForDifferentQuestions(t *testing.T) {
	a := approvalMarkerSignature("[MANY-AI-CLI]\nQ1 first?\n1. Yes\n[/MANY-AI-CLI]")
	b := approvalMarkerSignature("[MANY-AI-CLI]\nQ1 second?\n1. Yes\n[/MANY-AI-CLI]")
	if a == b {
		t.Fatalf("different questions must not share sig: %q", a)
	}
}

func TestExtractApprovalMarkerBlockSpansScrollback(t *testing.T) {
	// 画面高 4 行の VT に、開始〜閉じが画面をまたぐ長いマーカーブロックを流す。
	// TailLines（現在画面のみ）では不完全、TailLinesWithScrollback では完全ブロックが取れること。
	vt := newVTBuffer(80, 4)
	blockLines := []string{
		"[MANY-AI-CLI]",
		"Q1 first long question that should scroll out of the viewport?",
		" 1. Option A for Q1 (Recommended)",
		" 2. Option B for Q1",
		" N. User specifies",
		"Q2 second question also long enough?",
		" 3. Option C for Q2 (Recommended)",
		" 4. Option D for Q2",
		" N. User specifies",
		"[/MANY-AI-CLI]",
	}
	for _, line := range blockLines {
		vt.Write([]byte(line + "\r\n"))
	}
	// 現在画面だけでは完全ブロックが取れないこと（scrollback 経路の必要性を必須 assert）
	screenOnly := extractApprovalMarkerBlock(vt.TailLines(vtTailLinesForApproval))
	if screenOnly != nil && strings.Contains(screenOnly.Block, "Q1 first") && strings.Contains(screenOnly.Block, "Q2 second") {
		t.Fatalf("screen-only extract unexpectedly got full block (scrollback path not required): %q", screenOnly.Block)
	}
	// 画面 4 行 + 最終空行で、開始が scrollback 側に落ちている想定
	withSB := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if withSB == nil {
		t.Fatalf("scrollback extract returned nil; screen=%v sb=%v lines=%v", vt.Lines(), vt.scrollback, vt.TailLinesWithScrollback(50))
	}
	if !strings.Contains(withSB.Block, "[MANY-AI-CLI]") || !strings.Contains(withSB.Block, "[/MANY-AI-CLI]") {
		t.Fatalf("incomplete block: %q", withSB.Block)
	}
	if !strings.Contains(withSB.Block, "Q1 first") || !strings.Contains(withSB.Block, "Q2 second") {
		t.Fatalf("missing questions in block: %q", withSB.Block)
	}
}

// clear 後も scrollback に古い完全ブロックが残るが、同一 sig は再 broadcast されないこと
// （敵対レビュー P1: ゴースト承認の抑止）。
func TestMaybeBroadcastApprovalMarkerNoGhostAfterScreenClear(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	if ses.vt == nil {
		ses.vt = newVTBuffer(80, 4)
	}
	vt := ses.vt
	blockLines := []string{
		"[MANY-AI-CLI]",
		"Q1 ghost candidate after clear?",
		" 1. Yes (Recommended)",
		" 2. No",
		"[/MANY-AI-CLI]",
	}
	for _, line := range blockLines {
		vt.Write([]byte(line + "\r\n"))
	}
	// 押し出して scrollback に載せる
	for i := 0; i < 6; i++ {
		vt.Write([]byte("noise line that scrolls\r\n"))
	}
	marker := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil {
		t.Fatalf("expected marker in scrollback; sb=%v screen=%v", vt.scrollback, vt.Lines())
	}
	if !s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("first broadcast should succeed")
	}
	// 画面クリアしても scrollback は残る（仕様）
	vt.Write([]byte("\x1b[2J\x1b[H"))
	if len(vt.scrollback) == 0 {
		t.Fatal("scrollback should survive clear")
	}
	again := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if again == nil {
		t.Fatal("old complete block still extractable from scrollback after clear")
	}
	if again.Sig != marker.Sig {
		t.Fatalf("sig mismatch after clear: %q vs %q", again.Sig, marker.Sig)
	}
	if s.maybeBroadcastApprovalMarker(1, again, ses.lastOutputAt) {
		t.Fatal("same sig after clear must be deduped (no ghost approval_marker)")
	}
}

// writeInkFrame は Ink 風の全画面塗り直しを VT へ書き込む。
//
// 各行を絶対座標で置くだけで改行を使わない。画面からあふれた行は scrollback へ
// 押し出されるのではなく上書きで消える — Claude Code が代替画面バッファでやっている
// ことと同じで、下の openless テスト群が再現したい状態そのもの。
func writeInkFrame(vt *vtBuffer, rows []string) {
	var sb strings.Builder
	sb.WriteString("\x1b[2J")
	for i, line := range rows {
		fmt.Fprintf(&sb, "\x1b[%d;1H", i+1)
		sb.WriteString(line)
	}
	vt.Write([]byte(sb.String()))
}

// inkChrome は CLI が画面下端に固定表示する区切り線・入力欄・ステータス行。
var inkChrome = []string{
	"",
	strings.Repeat("─", 40),
	"❯",
	strings.Repeat("─", 40),
	"  $3.23  Opus 5 (1M context)",
	"  ⏵⏵ bypass permissions on",
}

// TestReconstructOpenlessMarkerBlockFromInkOverflow は、画面高を超える承認ブロックで
// 開始マーカーが画面外へ消えても承認が検出できることを検証する。
//
// 実測（2026-08-26 セッション #5・128x35・本文 26 行）: Ink の塗り直しでは
// あふれた行が newLine() を通らないため vtBuffer の scrollback は 0 行のままで、
// 開始マーカーはどこにも残らない。extractApprovalMarkerBlock は nil を返し、
// 承認は無音で検出されず standby の完了マーカー未検出フォールバックへ落ちていた。
func TestReconstructOpenlessMarkerBlockFromInkOverflow(t *testing.T) {
	vt := newVTBuffer(60, 20)
	rows := []string{
		"  2 二つ目の観察。書き込みの後で外部 API を呼んでいます。",
		"  失敗しても保存済みのレコードは戻されません。",
		"",
		"  3 三つ目の観察。判定はすべて外部側の行を起点にしています。",
		"",
		"  ここから先は実環境の実測が要ります。",
		"",
		"  Q1 read-only で確認してよいですか。",
		"  1. [実行する] 実行する (Recommended)",
		"  2. [検証系で] まず検証系で同じ確認をする",
		"      N. User specifies",
		"",
		approvalMarkerClose,
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	if len(vt.scrollback) != 0 {
		t.Fatalf("scrollback = %d 行, want 0 (Ink の塗り直しは押し出しではない)", len(vt.scrollback))
	}
	if plain := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker)); plain != nil {
		t.Fatalf("素の抽出が非 nil: %q (開始マーカーは画面外のはず)", plain.Block)
	}

	marker := extractApprovalMarkerBlockFromVT(vt)
	if marker == nil {
		t.Fatal("extractApprovalMarkerBlockFromVT = nil, want 再構成されたブロック")
	}
	if marker.Sig == "" {
		t.Fatal("sig が空")
	}
	if n := strings.Count(marker.Block, approvalMarkerOpen); n != 1 {
		t.Fatalf("OPEN が %d 個, want 1: %q", n, marker.Block)
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		t.Fatalf("classify = %q, want ok: %q", reason, marker.Block)
	}
	for _, want := range []string{"Q1 read-only", "1. [実行する]", "2. [検証系で]", "N. User specifies"} {
		if !strings.Contains(marker.Block, want) {
			t.Fatalf("再構成ブロックに %q が無い: %q", want, marker.Block)
		}
	}
}

// 選択肢まで画面外へ流れた場合は、承認バーを出さずに抑止の告知へ回すこと。
// 「1.」を欠いたまま配信すると 2 から始まる承認パネルが出て、ユーザーが意図しない
// 番号を送ってしまう（classifyApprovalMarkerBlock の option_start が本来の歯止め）。
func TestReconstructOpenlessMarkerBlockKeepsOptionStartGuard(t *testing.T) {
	vt := newVTBuffer(60, 20)
	rows := []string{
		"  ここから先は実環境の実測が要ります。",
		"",
		"  2. [検証系で] まず検証系で同じ確認をする",
		"  3. [やめる] いったん保留にする",
		"      N. User specifies",
		"",
		"  確認内容は次のとおりです。",
		"",
		"  step1 二つのテーブルを突き合わせる",
		"  step2 監査ログを引く",
		approvalMarkerClose,
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	marker := extractApprovalMarkerBlockFromVT(vt)
	if marker == nil {
		t.Fatal("再構成されないと抑止の告知も出ない（無音になる）")
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "option_start" {
		t.Fatalf("classify = %q, want option_start: %q", reason, marker.Block)
	}
}

// 終了マーカーが画面下端から離れている＝その下に別の出力が続いているときは
// 再構成しない。前ターンの回答済みブロックの末尾が画面上部に残っている状態で、
// 古い質問の承認バーを蘇らせないため。
func TestReconstructOpenlessMarkerBlockIgnoresCloseFarFromBottom(t *testing.T) {
	vt := newVTBuffer(60, 30)
	rows := []string{
		"  前ターンの回答の末尾",
		"  1. はい (Recommended)",
		"  2. いいえ",
		"      N. User specifies",
		"  確認内容は次のとおりです。",
		"  step1 …",
		"  step2 …",
		"  step3 …",
		approvalMarkerClose,
	}
	// 終了マーカーの下に新しい出力が 14 行続く（下端まで openlessCloseBottomWindow 超）。
	for i := 0; i < 14; i++ {
		rows = append(rows, fmt.Sprintf("  新しいターンの出力 %d", i))
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	if marker := extractApprovalMarkerBlockFromVT(vt); marker != nil {
		t.Fatalf("extractApprovalMarkerBlockFromVT = %q, want nil (下端から離れた終了マーカー)", marker.Block)
	}
}

// 終了マーカーより後ろに開始マーカーがあるなら、次のブロックを描き始めている。
// 手前の古い終了マーカーで再構成しない。
func TestReconstructOpenlessMarkerBlockIgnoresNewerOpen(t *testing.T) {
	vt := newVTBuffer(60, 20)
	rows := []string{
		"  前のブロックの本文",
		"  1. はい (Recommended)",
		"  2. いいえ",
		"      N. User specifies",
		"  step1 …",
		"  step2 …",
		"  step3 …",
		"  step4 …",
		approvalMarkerClose,
		approvalMarkerOpen,
		"  Q1 次の質問を書き始めたところ",
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	if marker := extractApprovalMarkerBlockFromVT(vt); marker != nil {
		t.Fatalf("extractApprovalMarkerBlockFromVT = %q, want nil (後ろに新しい OPEN)", marker.Block)
	}
}

// 数行の断片からは再構成しない（画面が低い端末での誤検出防止）。
func TestReconstructOpenlessMarkerBlockIgnoresShortFragment(t *testing.T) {
	vt := newVTBuffer(60, 12)
	rows := []string{
		"",
		"",
		"",
		"  1. はい (Recommended)",
		"  2. いいえ",
		approvalMarkerClose,
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	if marker := extractApprovalMarkerBlockFromVT(vt); marker != nil {
		t.Fatalf("extractApprovalMarkerBlockFromVT = %q, want nil (本文が短すぎる)", marker.Block)
	}
}

// 開始マーカーが画面に残っている通常のブロックでは、再構成へ落ちずに
// これまでどおり実ブロックがそのまま返ること。
func TestExtractApprovalMarkerBlockFromVTPrefersRealOpen(t *testing.T) {
	vt := newVTBuffer(60, 20)
	rows := []string{
		approvalMarkerOpen,
		"  Q1 進めますか?",
		"  1. はい (Recommended)",
		"  2. いいえ",
		"      N. User specifies",
		approvalMarkerClose,
	}
	writeInkFrame(vt, append(rows, inkChrome...))

	marker := extractApprovalMarkerBlockFromVT(vt)
	if marker == nil {
		t.Fatal("extractApprovalMarkerBlockFromVT = nil")
	}
	if !strings.HasPrefix(marker.Block, approvalMarkerOpen+"\n  Q1 進めますか?") {
		t.Fatalf("実ブロックがそのまま返っていない: %q", marker.Block)
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		t.Fatalf("classify = %q, want ok", reason)
	}
}
