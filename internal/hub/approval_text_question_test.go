package hub

// 画面だけが拾っていた承認 4 種を Hub で検出するテスト（子 plan C1 の C4）。
//
// パーサの入力と期待値は、以前 web/src/app/approval-parser-fixtures.ts にあったものを写している
// （2026-09-23 にブラウザ側の検出を撤去し、こちらが正本になった）。fixture はすべて合成データ。

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func optionNums(options []proto.ApprovalOption) []int {
	out := make([]int, 0, len(options))
	for _, opt := range options {
		out = append(out, opt.Num)
	}
	return out
}

func sameInts(a, b []int) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// ---- パーサ（以前 approval-parser-fixtures.ts にあった入力と期待値）----

func TestPlainYesNoQuestionFixtures(t *testing.T) {
	for _, tc := range []struct {
		name  string
		lines []string
		want  bool
	}{
		{"半角", []string{"Do you want to apply this patch? (Y:1/N:0)"}, true},
		{"全角", []string{"A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？ （Y：1／N：0）"}, true},
		{"折り返し", []string{"A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？ (Y:1/", "N:0)"}, true},
		{"見本の質問", []string{"question? (Y:1/N:0)"}, false},
		// マーカーの中の Yes/No はこの関数単体では拾う。
		// 検出の経路は承認マーカーを先に見るので、マーカー無しの質問として開くことはない。
		{"疑問符が無い", []string{"This is not a question (Y:1/N:0)"}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := extractPlainYesNoQuestion(tc.lines)
			if (got != nil) != tc.want {
				t.Fatalf("extractPlainYesNoQuestion = %+v, want found=%v", got, tc.want)
			}
			if got == nil {
				return
			}
			if got.Kind != approvalKindPlainYesNo || !sameInts(optionNums(got.Options), []int{1, 0}) ||
				got.Options[0].Label != "Yes (1)" || got.Options[1].Label != "No (0)" {
				t.Fatalf("plain yes/no = %+v", got)
			}
			if got.Question == "" || strings.Contains(got.Question, "Y:1") {
				t.Fatalf("質問文 = %q, want (Y:1/N:0) より前の本文", got.Question)
			}
		})
	}
}

func TestSequentialChoiceFixtures(t *testing.T) {
	lines := []string{
		"Q1: Choose branch",
		"  1. main",
		"  2. develop",
		"  N. User specifies",
		"Q2: Run tests",
		"  1. Yes",
		"  2. No",
		"  N. User specifies",
	}
	prompts := extractSequentialChoicePrompts(lines)
	if len(prompts) != 2 || prompts[0].Key != "Q1" || prompts[1].Question != "Run tests" ||
		!sameInts(optionNums(prompts[0].Options), []int{1, 2}) {
		t.Fatalf("prompts = %+v", prompts)
	}
	// 記録の Block は同じパーサで読み戻せる（approval-parser-fixtures.ts の sequentialChoiceSig の往復と同じ確認）。
	q := extractSequentialChoiceQuestion(lines)
	if q == nil || q.Kind != approvalKindSequentialChoice {
		t.Fatalf("sequential question = %+v", q)
	}
	again := extractSequentialChoicePrompts(strings.Split(q.Block, "\n"))
	if len(again) != 2 || again[0].Key != "Q1" || again[0].Question != "Choose branch" || again[1].Options[1].Label != "No" {
		t.Fatalf("Block を読み戻した結果 = %+v（Block = %q）", again, q.Block)
	}
	// 1 問だけ・マーカーを含む画面は順次質問にしない。
	if got := extractSequentialChoicePrompts(lines[:4]); got != nil {
		t.Fatalf("1 問だけで順次質問になった: %+v", got)
	}
	if got := extractSequentialChoicePrompts(append([]string{approvalMarkerOpen}, lines...)); got != nil {
		t.Fatalf("マーカーを含む画面で順次質問になった: %+v", got)
	}
}

func TestHubChoiceFixtures(t *testing.T) {
	// マーカー移行前の旧形式は、質問→選択肢→自由入力行という明確な構造に限って救済する。
	legacy := extractHubChoiceQuestion([]string{
		"Q1 どちらで進めますか？",
		"1. 最小修正 (Recommended)",
		"2. 原因調査も行う",
		"N. User specifies",
	})
	if legacy == nil || !sameInts(optionNums(legacy.Options), []int{1, 2}) || !legacy.Options[0].IsCurrent {
		t.Fatalf("旧形式の選択 = %+v", legacy)
	}
	if legacy.Question != "Q1 どちらで進めますか？" || !strings.HasSuffix(legacy.Block, "N. User specifies") {
		t.Fatalf("質問文 / Block = %q / %q", legacy.Question, legacy.Block)
	}
	// 通常の手順説明に Q1 と「（推奨）」が混ざっても、確認待ちにはしない（質問が選択肢の後ろ）。
	if got := extractHubChoiceQuestion([]string{
		"1. metricId 一致のみでマージ判定する（推奨）— データ消失の責任範囲を除去する。",
		"2. 上記に加えて週利用量誤命名の根本原因も追加調査する。",
		"3. 結果、月間利用量は普通に追加し、ローリング利用量は表示名だけ直す。",
		"Q1 どちらで進めますか？",
	}); got != nil {
		t.Fatalf("手順説明が旧形式の選択になった: %+v", got)
	}
	if got := extractHubChoiceQuestion([]string{
		"Implementation notes:",
		"1. Read the config",
		"2. Update the renderer",
		"3. Add a test",
	}); got != nil {
		t.Fatalf("番号付きの箇条書きが旧形式の選択になった: %+v", got)
	}
}

func TestFallbackApprovalOptionFixtures(t *testing.T) {
	// 1 行に連結された「1. … 2. … 3. … N. User specifies」を 3 選択肢へ復元する。
	glued := extractFallbackApprovalOptions([]string{
		"1. 質問2=「PCも含め全画面で効かせる」を選択（質問1の初期値はON=既定で表示のまま）(Recommended)2. 質問1=「初期値OFF=既定で非表示」を選択（質問2の適用範囲はスマホのみのまま）3. 両方とも option 2（初期値OFF かつ 全画面で効かせる） N. User specifies",
	})
	if !sameInts(optionNums(glued.Options), []int{1, 2, 3}) ||
		strings.Contains(glued.Options[2].Label, "User specifies") || !strings.Contains(glued.Options[0].Label, "Recommended") {
		t.Fatalf("連結の復元 = %+v", glued.Options)
	}
	// option 1 が折り返され、継続行が行頭に空白を持たない。
	wrapped := extractFallbackApprovalOptions([]string{
		"どこまで進めるか確認します。",
		"1. C4(docs) + C3の「全アクセス失効」ボタンのみ（＝最小構成完成・C1を実際に使え",
		"る状態に。PINは見送り）  (Recommended)",
		"2. 上記に加えて C2+C3のPIN一式も実装（任意PIN・ロックアウト・SEC-C新規デバイス通知まで全部）",
		"3. C4(docs)だけ先に作る（ボタンUIは後回し）",
		"N. User specifies",
	})
	if !sameInts(optionNums(wrapped.Options), []int{1, 2, 3}) ||
		!strings.Contains(wrapped.Options[0].Label, "Recommended") || !strings.Contains(wrapped.Options[0].Label, "る状態に") {
		t.Fatalf("折り返しの結合 = %+v", wrapped.Options)
	}
	// 連番でない「1. … 3. …」は誤分割しない。
	if got := extractFallbackApprovalOptions([]string{"1. first 3. third"}); len(got.Options) > 1 {
		t.Fatalf("連番でない行を分割した: %+v", got.Options)
	}
	// AskUserQuestion のピッカーは Web ボタン化しない。
	if got := extractFallbackApprovalOptions([]string{
		"スキーマ差分の適用範囲は?",
		"❯ 1. 全差分を全環境へ適用",
		"  2. 必要なものだけ精査して適用",
		"  3. コードだけ先にデプロイ",
		"  4. 差分の中身を先に見たい",
		"  5. Type something.",
		"  6. Chat about this",
	}); len(got.Options) != 0 {
		t.Fatalf("AskUserQuestion の選択肢を返した: %+v", got.Options)
	}
	// 標準のツール許可プロンプトは抑止しない。
	if got := extractFallbackApprovalOptions([]string{
		"This command requires approval",
		"❯ 1. Yes",
		"  2. Yes, and don't ask again for this command",
		"  3. No",
	}); !sameInts(optionNums(got.Options), []int{1, 2, 3}) {
		t.Fatalf("標準の承認 = %+v", got.Options)
	}
	// Grok Build のラジオ印（番号の直後がピリオドではない）。
	grok := extractFallbackApprovalOptions([]string{
		"Check MANY_AI_CLI hub session env",
		"$env:MANY_AI_CLI",
		"1 (•) Yes, and don't ask again for anything (always-approve mode)",
		"2 (○) Yes, proceed",
		"3 (○) No, reject (type to add feedback)",
		"1/3:select | Tab:next option | Ctrl+o:always-approve | Ctrl+c:cancel | Esc:scrollback",
	})
	if !sameInts(optionNums(grok.Options), []int{1, 2, 3}) || grok.Options[1].Label != "Yes, proceed" ||
		!grok.Options[0].IsCurrent || grok.Options[1].IsCurrent {
		t.Fatalf("Grok のラジオ印 = %+v", grok.Options)
	}
}

func TestUngluedApprovalLinesFixtures(t *testing.T) {
	result := ungluedApprovalLines([]string{
		"  どの方向でいきますか？",
		"1. [A] 説明A (Recommended)2. [B] 説明B3. [C] 説明C",
	})
	for _, prefix := range []string{"1.", "2.", "3."} {
		found := false
		for _, line := range result {
			if strings.HasPrefix(line, prefix) {
				found = true
			}
		}
		if !found {
			t.Fatalf("%s の行が無い: %q", prefix, result)
		}
	}
	proper := ungluedApprovalLines([]string{
		"  Q1 進め方",
		"  1. [A] 説明A (Recommended)",
		"  2. [B] 説明B",
		"   N. User specifies",
	})
	ones, twos := 0, 0
	for _, line := range proper {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "1.") {
			ones++
		}
		if strings.HasPrefix(trimmed, "2.") {
			twos++
		}
	}
	if ones != 1 || twos != 1 {
		t.Fatalf("改行済みの入力を再分割した: %q", proper)
	}
}

func TestIsMultiQuestionPrompt(t *testing.T) {
	for _, tc := range []struct {
		line string
		want bool
	}{
		{"←  ☐ Scope  ☐ Timing  ✔ Submit  →", true},
		{"← Scope  Timing  Submit →", true},
		{"Review your answers", true},
		{"Ready to submit your answers?", true},
		{"Press → to continue", false},
		{"Submit the form when ready", false},
	} {
		if got := isMultiQuestionPrompt([]string{tc.line}); got != tc.want {
			t.Fatalf("isMultiQuestionPrompt(%q) = %v, want %v", tc.line, got, tc.want)
		}
	}
}

// ---- AskUserQuestion の告知 ----

func askUserQuestionScreen(cursorOn int) []string {
	options := []string{"全差分を全環境へ適用", "必要なものだけ精査して適用", "Type something.", "Chat about this"}
	lines := []string{"スキーマ差分の適用範囲は?"}
	for i, label := range options {
		prefix := "  "
		if i == cursorOn {
			prefix = "❯ "
		}
		lines = append(lines, prefix+string(rune('1'+i))+". "+label)
	}
	return append(lines, "Enter to select · ↑↓ to navigate · Esc to cancel")
}

func TestAskUserQuestionNoticeDetection(t *testing.T) {
	// カーソルが動いても同じ告知（同一性に本文を入れない）。
	first := detectNativeApproval("claude", askUserQuestionScreen(0))
	moved := detectNativeApproval("claude", askUserQuestionScreen(1))
	if first == nil || moved == nil || first.Kind != approvalKindAskUserQuestion || first.Sig != moved.Sig {
		t.Fatalf("告知 = %+v / %+v, want カーソル位置によらず同じ告知", first, moved)
	}
	// 複数質問のタブ行は、選択肢より先に告知として判定する。
	multi := detectNativeApproval("claude", []string{
		"←  ☐ Scope  ☐ Timing  ✔ Submit  →",
		"",
		"How wide should the change be?",
		"❯ 1. Only this file",
		"  2. The whole package",
		"  3. Type something.",
		"Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
	})
	if multi == nil || multi.Kind != approvalKindAskUserQuestion || multi.Question != "" || len(multi.Options) != 0 {
		t.Fatalf("複数質問の告知 = %+v", multi)
	}
	// Review ページの Submit / Cancel を普通の承認として出さない。
	review := detectNativeApproval("claude", []string{
		"Review your answers",
		"",
		"Ready to submit your answers?",
		"❯ 1. Submit answers",
		"  2. Cancel",
		"Enter to select · Esc to cancel",
	})
	if review == nil || review.Kind != approvalKindAskUserQuestion || review.Sig != multi.Sig {
		t.Fatalf("Review ページ = %+v, want タブの告知と同じ 1 件", review)
	}
}

// 告知は記録を開き、「保留中」と通知を出す。消えたら閉じる。一括承認の対象にしない。
func TestAskUserQuestionNoticeRecord(t *testing.T) {
	s, count := countingWebhook(t)
	ses := registerTestSession(s, 1, "claude")
	ses.lastOutputAt = time.Now().Add(-time.Minute)
	sent := captureUIBroadcasts(s)

	s.handleNativeApprovalDetection(1, detectNativeApproval("claude", askUserQuestionScreen(0)))
	record := ses.pendingApproval
	if !record.isNative() || record.Kind != approvalKindAskUserQuestion || record.hasOptions() {
		t.Fatalf("告知の記録 = %+v", record)
	}
	s.handleNativeApprovalDetection(1, detectNativeApproval("claude", askUserQuestionScreen(2)))
	if ses.pendingApproval != record {
		t.Fatal("カーソルを動かしただけで記録が開き直した")
	}
	if n := len(approvalStateOpens(sent(), 1)); n != 1 {
		t.Fatalf("開く approval_state = %d 件, want 1", n)
	}
	if !ses.Activity.AwaitingApproval || ses.State != "waiting" {
		t.Fatalf("告知で「保留中」が立たない: state=%q activity=%+v", ses.State, ses.Activity)
	}
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte(strings.Join(askUserQuestionScreen(0), "\r\n")))
	if items := s.pendingNativeApprovals(); len(items) != 0 {
		t.Fatalf("一括承認の対象に告知が入った: %d 件", len(items))
	}

	for i := 0; i < nativeApprovalClearMissLimit; i++ {
		s.handleNativeApprovalDetection(1, nil)
	}
	assertSingleClose(t, sent, 1, record.Sig, approvalCloseVanished)
	assertNotificationCount(t, count, 1)
}

// ---- 手がかり語 ----

func TestScreenApprovalUsesTriggerPhrasesAndUserSpecifies(t *testing.T) {
	codex := []string{
		"Apply the migration now. Continue?",
		"› 1. Yes",
		"  2. No",
	}
	if got := detectNativeApproval("codex", codex); got != nil {
		t.Fatalf("手がかり語が無いのに検出した: %+v", got)
	}
	if got := detectNativeApprovalWith("codex", codex, []string{"continue?"}); got == nil || !sameInts(optionNums(got.Options), []int{1, 2}) {
		t.Fatalf("承認パターンの語で検出しない: %+v", got)
	}
	// ブラウザの推測検出と同じく、承認語の選択肢に「N. User specifies」が添えてあれば承認とみなす。
	userSpecifies := []string{
		"How should we handle this?",
		"❯ 1. Yes, apply",
		"  2. No, skip",
		"N. User specifies",
	}
	if got := detectNativeApproval("claude", userSpecifies); got == nil {
		t.Fatal("N. User specifies を手がかりにしない")
	}
}

func TestApprovalTriggerPhrasesReloadFromPatternFiles(t *testing.T) {
	home := withApprovalTestHome(t)
	dir := filepath.Join(home, ".many-ai-cli", "approval-patterns")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "codex.json"), []byte(`["Continue?"]`), 0o600); err != nil {
		t.Fatal(err)
	}
	s := newTestServer()
	lines := []string{"Apply the migration now. Continue?", "› 1. Yes", "  2. No"}
	if got := s.detectScreenApproval("codex", lines); got != nil {
		t.Fatalf("読み込む前に検出した: %+v", got)
	}
	s.reloadApprovalTriggerPhrases()
	if got := s.detectScreenApproval("codex", lines); got == nil {
		t.Fatal("承認パターンのファイルを読んでも検出しない")
	}
	if !s.chunkMayShowScreenApproval("codex", []byte("please CONTINUE? now")) {
		t.Fatal("承認パターンの語で画面検出の入口が開かない")
	}
	if s.chunkMayShowScreenApproval("codex", []byte("plain output")) {
		t.Fatal("手がかりの無いチャンクで画面検出の入口が開いた")
	}
}

// ---- 文章の質問の記録 ----

// 端末ミラーのマーカー無し Yes/No: 記録を開き「保留中」と通知を出す。
// 確定したユーザーターンで閉じ、画面に残っている間は次のターンでも開き直さない。
func TestPlainYesNoRecordFromTerminal(t *testing.T) {
	s, count := countingWebhook(t)
	ses := registerTestSession(s, 1, "grok")
	ses.lastOutputAt = time.Now().Add(-time.Minute)
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Deploy to staging finished.\r\nProceed with the production deploy? (Y:1/N:0)\r\n"))
	sent := captureUIBroadcasts(s)
	screen := func() *textQuestion { return detectTextQuestion(ses.vt.TailLines(vtTailLinesForApproval)) }

	if !s.openTextQuestion(1, screen(), time.Now(), approvalSourceGoVT) {
		t.Fatal("マーカー無しの Yes/No が記録にならない")
	}
	record := ses.pendingApproval
	if !record.isMarker() || record.Kind != approvalKindPlainYesNo || record.Question != "Proceed with the production deploy?" ||
		!sameInts(optionNums(record.Options), []int{1, 0}) {
		t.Fatalf("記録 = %+v", record)
	}
	if !ses.Activity.AwaitingApproval {
		t.Fatal("「保留中」が立たない")
	}
	if s.openTextQuestion(1, screen(), time.Now(), approvalSourceGoVT) {
		t.Fatal("同じ質問で記録を開き直した")
	}

	s.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
	assertSingleClose(t, sent, 1, record.Sig, approvalCloseAnsweredTerminal)
	if s.openTextQuestion(1, screen(), time.Now(), approvalSourceGoVT) {
		t.Fatal("回答済みの質問を開き直した")
	}
	// 次のターンでも、画面に残っている回答済みの質問は持ち越す（approval_identity.go の持ち越し）。
	s.handleInput(proto.Message{SessionID: 1, Text: "次の作業へ進んでください\r"})
	if s.openTextQuestion(1, screen(), time.Now(), approvalSourceGoVT) {
		t.Fatal("次のターンで回答済みの質問を開き直した")
	}
	assertNotificationCount(t, count, 1)
}

// 端末ミラーの文章の質問は、画面に出ているネイティブの承認を上書きしない（マーカーと同じ例外）。
func TestTerminalTextQuestionDoesNotReplaceVisibleNativeRecord(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	native := syntheticNativeApproval("Run git status?")
	s.handleNativeApprovalDetection(1, native)
	question := detectTextQuestion([]string{"Proceed? (Y:1/N:0)"})
	if s.openTextQuestion(1, question, time.Now(), approvalSourceGoVT) || nativeRecordSig(ses) != native.Sig {
		t.Fatalf("文章の質問がネイティブの記録を上書きした: %+v", ses.pendingApproval)
	}
}

// トランスクリプトの順次質問: assistant メッセージを出力が落ち着いた時点で開き、次の user メッセージで閉じる。
func TestSequentialChoiceRecordFromTranscript(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.agentChatPath = "transcript.jsonl"
	sent := captureUIBroadcasts(s)
	text := strings.Join([]string{
		"進める前に 2 点確認させてください。",
		"Q1: Choose branch",
		"  1. main",
		"  2. develop",
		"Q2: Run tests",
		"  1. Yes",
		"  2. No",
	}, "\n")
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: text},
	}, false, time.Now())
	if ses.pendingApproval != nil {
		t.Fatalf("出力が落ち着く前に開いた: %+v", ses.pendingApproval)
	}
	s.evaluateIdle()
	record := ses.pendingApproval
	if !record.isMarker() || record.Kind != approvalKindSequentialChoice || record.Source != approvalSourceTranscript ||
		!strings.Contains(record.Block, "Q2: Run tests") {
		t.Fatalf("記録 = %+v", record)
	}
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "user", Kind: "text", Text: "Q1: 1\nQ2: 2"},
	}, false, time.Now())
	assertSingleClose(t, sent, 1, record.Sig, approvalCloseAnsweredTerminal)
}

// トランスクリプトの旧形式の選択: 質問と選択肢を記録に持ち、既定の選択は推奨の選択肢。
// 出力がすでに落ち着いていれば、読んだその場で開く。
func TestHubChoiceRecordFromTranscript(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.Activity.OutputIdle = true
	s.scanTranscriptApprovalMarkers(1, "codex", "rollout.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: "原因は 2 つ考えられます。\nどちらで進めますか？\n1. 最小修正\n2. 原因調査も行う (Recommended)\nN. User specifies"},
	}, false, time.Now())
	record := ses.pendingApproval
	if !record.isMarker() || record.Kind != approvalKindHubChoice || record.Question != "どちらで進めますか？" ||
		!sameInts(optionNums(record.Options), []int{1, 2}) || !record.Options[1].IsCurrent {
		t.Fatalf("記録 = %+v", record)
	}
}

// ---- 文章の質問は出力が落ち着いてから開く（C5 の敵対レビュー指摘 2） ----

// transcriptSequentialQuestion は、作業の途中に書いても順次質問と同じ形になる箇条書き。
func transcriptSequentialQuestion() string {
	return strings.Join([]string{
		"C1: Pick the parser",
		"  1. Keep the current one",
		"  2. Replace it",
		"C2: Pick the test runner",
		"  1. go test",
		"  2. bun test",
	}, "\n")
}

// トランスクリプト: ターンの途中の assistant メッセージが質問の形をしていても、その後に
// assistant のメッセージ（ツール・本文）が続いたら開かない。Push も「保留中」も出ない。
func TestTranscriptTextQuestionMidTurnDoesNotOpen(t *testing.T) {
	s, count := countingWebhook(t)
	ses := registerTestSession(s, 1, "claude")
	ses.agentChatPath = "transcript.jsonl"
	sent := captureUIBroadcasts(s)

	// 同じバッチの中で続いた場合
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion()},
		{Role: "assistant", Kind: "tool", Tools: []agentChatTool{{Name: "Bash", Input: "go test ./..."}}},
	}, false, time.Now())
	s.evaluateIdle()
	if ses.pendingApproval != nil || ses.Activity.AwaitingApproval {
		t.Fatalf("途中の箇条書きを質問として開いた: %+v", ses.pendingApproval)
	}

	// 次の poll で続いた場合（出力はまだ落ち着いていない）
	ses.Activity.OutputIdle = false
	ses.lastOutputAt = time.Now()
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion()},
	}, false, time.Now())
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "thinking", Thinking: []string{"run the tests"}},
	}, false, time.Now())
	ses.lastOutputAt = time.Now().Add(-time.Minute)
	s.evaluateIdle()
	if ses.pendingApproval != nil {
		t.Fatalf("続いた後の候補を開いた: %+v", ses.pendingApproval)
	}

	// 本文とツールを 1 通に持つメッセージは、ターンの途中なので質問として見ない
	ses.Activity.OutputIdle = true
	s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion(), Tools: []agentChatTool{{Name: "Read"}}},
	}, false, time.Now())
	if ses.pendingApproval != nil {
		t.Fatalf("ツールを呼んだメッセージを質問として開いた: %+v", ses.pendingApproval)
	}

	if opens := approvalStateOpens(sent(), 1); len(opens) != 0 {
		t.Fatalf("開く approval_state = %d 件, want 0", len(opens))
	}
	assertNotificationCount(t, count, 0)
}

// トランスクリプト: 開いた文章の質問の後に AI が答えを待たずに話し続けたら、vanished で閉じる。
// 同じ質問がもう一度書かれただけなら閉じない。承認マーカーの記録は、この規則では閉じない。
func TestTranscriptTextQuestionClosesWhenAssistantContinues(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.agentChatPath = "transcript.jsonl"
	ses.Activity.OutputIdle = true
	sent := captureUIBroadcasts(s)
	scan := func(messages ...agentChatMessage) {
		s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", messages, false, time.Now())
	}

	scan(agentChatMessage{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion()})
	record := ses.pendingApproval
	if !record.isTranscriptTextQuestion() {
		t.Fatalf("記録 = %+v", record)
	}
	scan(agentChatMessage{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion()})
	if ses.pendingApproval != record || len(approvalStateCloses(sent(), 1)) != 0 {
		t.Fatal("同じ質問が続いただけで閉じた")
	}

	scan(agentChatMessage{Role: "assistant", Kind: "tool", Tools: []agentChatTool{{Name: "Bash"}}})
	assertSingleClose(t, sent, 1, record.Sig, approvalCloseVanished)
	if ses.pendingApproval != nil || ses.Activity.AwaitingApproval {
		t.Fatal("話し続けた後も「保留中」が残っている")
	}

	// 承認マーカーは AI が答えを待つと書いた合図なので、続けて話しても閉じない（閉じるのはユーザーターン）。
	scan(agentChatMessage{Role: "assistant", Kind: "text", Text: transcriptAssistantText()})
	marker := ses.pendingApproval
	if !marker.isMarker() || marker.Kind != approvalKindMarker {
		t.Fatalf("マーカーの記録 = %+v", marker)
	}
	scan(agentChatMessage{Role: "assistant", Kind: "tool", Tools: []agentChatTool{{Name: "Bash"}}})
	if ses.pendingApproval != marker {
		t.Fatal("続けて話しただけでマーカーの記録を閉じた")
	}
}

// トランスクリプト: 候補のまま答えられた（user メッセージ・確定したユーザーターン）ら、落ち着いても開かない。
func TestTranscriptTextQuestionCandidateDroppedByUserTurn(t *testing.T) {
	for _, tc := range []struct {
		name   string
		answer func(s *Server)
	}{
		{"transcript user message", func(s *Server) {
			s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
				{Role: "user", Kind: "text", Text: "C1: 2"},
			}, false, time.Now())
		}},
		{"submitted user turn", func(s *Server) {
			s.handleInput(proto.Message{SessionID: 1, Text: "C1: 2\r"})
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newTestServer()
			ses := registerTestSession(s, 1, "claude")
			ses.agentChatPath = "transcript.jsonl"
			s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
				{Role: "assistant", Kind: "text", Text: transcriptSequentialQuestion()},
			}, false, time.Now())
			tc.answer(s)
			s.evaluateIdle()
			if ses.pendingApproval != nil {
				t.Fatalf("答えた後の候補を開いた: %+v", ses.pendingApproval)
			}
		})
	}
}

// 端末ミラー: 出力が続いている間は文章の質問を開かず、落ち着いた時点で 1 回見て開く。
// 承認マーカーが読めるときは文章の質問を探さない。
func TestTerminalTextQuestionOpensOnlyWhenOutputIdle(t *testing.T) {
	s, count := countingWebhook(t)
	ses := registerTestSession(s, 1, "grok")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Deploy to staging finished.\r\nProceed with the production deploy? (Y:1/N:0)\r\n"))

	ses.lastOutputAt = time.Now()
	s.evaluateIdle()
	if ses.pendingApproval != nil {
		t.Fatalf("出力が続いている間に開いた: %+v", ses.pendingApproval)
	}
	s.evaluateReplayApproval(1)
	if ses.pendingApproval != nil {
		t.Fatalf("出力が続いている間に replay の評価で開いた: %+v", ses.pendingApproval)
	}

	ses.lastOutputAt = time.Now().Add(-time.Minute)
	s.evaluateIdle()
	record := ses.pendingApproval
	if !record.isMarker() || record.Kind != approvalKindPlainYesNo || record.Source != approvalSourceGoVT {
		t.Fatalf("記録 = %+v", record)
	}
	if ses.State != "waiting" || !ses.Activity.AwaitingApproval {
		t.Fatalf("落ち着いた同じ評価で「保留中」にならない: state=%q activity=%+v", ses.State, ses.Activity)
	}
	s.evaluateIdle()
	assertNotificationCount(t, count, 1)

	// 承認マーカーがあるときは、そちらが優先で文章の質問を探さない。
	marked := registerTestSession(s, 2, "grok")
	marked.vt = newVTBuffer(80, 24)
	marked.vt.Write([]byte(strings.ReplaceAll(strings.Join(transcriptMarkerBody, "\n")+"\nProceed? (Y:1/N:0)", "\n", "\r\n")))
	marked.lastOutputAt = time.Now().Add(-time.Minute)
	s.evaluateIdle()
	if marked.pendingApproval.isMarker() && marked.pendingApproval.Kind == approvalKindPlainYesNo {
		t.Fatalf("マーカーがあるのに文章の質問を開いた: %+v", marked.pendingApproval)
	}
}
