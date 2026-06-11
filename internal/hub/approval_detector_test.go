package hub

import (
	"bytes"
	"fmt"
	"os"
	"strings"
	"testing"

	"any-ai-cli/internal/proto"
)

func TestDetectNativeApprovalClaude(t *testing.T) {
	lines := []string{
		"",
		"Allow tool: Bash",
		"",
		"❯ 1. Yes, allow once",
		"  2. Yes, allow for this session",
		"  3. No",
	}
	got := detectNativeApproval("claude", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 3 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	if !got.Options[0].IsCurrent || got.Options[0].Label != "Yes, allow once" {
		t.Fatalf("option 1 = %+v", got.Options[0])
	}
	if got.Sig == "" {
		t.Fatal("sig is empty")
	}
}

func TestDetectNativeApprovalCodexShortcut(t *testing.T) {
	lines := []string{
		"Command requires approval",
		"Run: rm -rf ./tmp",
		"",
		"❯ Yes (y)",
		"  Yes, and don't ask again for this command (p)",
		"  No (n)",
		"  Cancel (esc)",
	}
	got := detectNativeApproval("codex", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native_codex_shortcut" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 4 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	if got.Options[0].SendText != "y" {
		t.Fatalf("option 1 send_text = %q", got.Options[0].SendText)
	}
	if got.Options[3].SendText != "\x1b" {
		t.Fatalf("option 4 send_text = %q", got.Options[3].SendText)
	}
}

func TestDetectNativeApprovalCopilotShortcut(t *testing.T) {
	lines := []string{
		"Permission required",
		"Run: git status",
		"",
		"❯ Allow once (y)",
		"  Deny once (n)",
		"  Allow all similar for this session (!)",
		"  Deny all similar for this session (#)",
		"  Show details (?)",
	}
	got := detectNativeApproval("copilot", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native_copilot_shortcut" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 5 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	wantSend := []string{"y", "n", "!", "#", "?"}
	for i, want := range wantSend {
		if got.Options[i].SendText != want {
			t.Fatalf("option %d send_text = %q, want %q (%+v)", i+1, got.Options[i].SendText, want, got.Options)
		}
	}
	if got.Options[3].Num != 4 || got.Options[4].Num != 5 {
		t.Fatalf("copilot option nums = %+v", got.Options)
	}
}

func TestDetectNativeApprovalCursorAgentShortcut(t *testing.T) {
	// 実機 UI（スクリーンショットより）。番号付きではなくキー表記のみのメニュー。
	lines := []string{
		"Run this command?",
		`Not in allowlist: Get-Date -Format "yyyy-MM-dd HH:mm:ss (dddd)"`,
		" - Run (once) (y)",
		"    Add Shell(Get-Date) to allowlist? (tab)",
		"    Auto-run everything (shift+tab)",
		"    Skip (esc or n)",
	}
	got := detectNativeApproval("cursor-agent", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native_cursor_agent_shortcut" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 4 {
		t.Fatalf("options len = %d (%+v)", len(got.Options), got.Options)
	}
	wantSend := []string{"y", "\t", "\x1b[Z", "\x1b"}
	for i, want := range wantSend {
		if got.Options[i].SendText != want {
			t.Fatalf("option %d send_text = %q, want %q (%+v)", i+1, got.Options[i].SendText, want, got.Options)
		}
	}
	if !got.Options[0].IsCurrent {
		t.Fatalf("option 1 should be current (selected): %+v", got.Options[0])
	}
	if got.Options[0].Label != "Run (once) (y)" || got.Options[1].Label != "Add Shell(Get-Date) to allowlist? (tab)" {
		t.Fatalf("cursor-agent labels = %q / %q", got.Options[0].Label, got.Options[1].Label)
	}
}

func TestDetectNativeApprovalClaudeModelSelector(t *testing.T) {
	// Claude Code の /model セレクタ（実機ログ claude_2026-06-11_051610_mer_s1 より）。
	// 選択肢ラベルに承認語（yes/no/allow 等）を含まないセレクタ型ダイアログ。
	lines := []string{
		"Select model",
		"Switch between Claude models. Your pick becomes the default for new sessions.",
		"",
		"  1. Default (recommended)  Opus 4.8 with 1M context · Best for everyday, complex tasks",
		"❯ 2. Fable ✔  Fable 5 · Most capable for your hardest and longest-running tasks",
		"  3. Sonnet  Sonnet 4.6 · Efficient for routine tasks",
		"  4. Haiku  Haiku 4.5 · Fastest for quick answers",
		"",
		"◐ Medium effort  ←/→ to adjust  Enter to set as default · s to use this session only · Esc to cancel",
	}
	got := detectNativeApproval("claude", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 4 {
		t.Fatalf("options len = %d (%+v)", len(got.Options), got.Options)
	}
	if !got.Options[1].IsCurrent {
		t.Fatalf("option 2 should be current: %+v", got.Options[1])
	}
}

func TestDetectNativeApprovalSuppressesAskUserQuestion(t *testing.T) {
	// Claude の AskUserQuestion ピッカー（末尾に "Type something" / "Chat about this"
	// の自由入力肢を持つ arrow 駆動 UI）は webify しない。再描画される VT のスクレイプで
	// 選択肢番号が Web ボタンとズレ誤選択を招くため、キーヒントが揃っていても nil を返し
	// ターミナル直操作へフォールバックする（approval-rules.md version 10 でマーカー誘導済み）。
	lines := []string{
		"スキーマ差分の適用範囲は?",
		"❯ 1. 全差分を全環境へ適用",
		"  2. 必要なものだけ精査して適用",
		"  3. コードだけ先にデプロイ",
		"  4. 差分の中身を先に見たい",
		"  5. Type something.",
		"  6. Chat about this",
		"Enter to select · ↑↓ to navigate · Esc to cancel",
	}
	if got := detectNativeApproval("claude", lines); got != nil {
		t.Fatalf("detectNativeApproval = %+v, want nil (AskUserQuestion should be suppressed)", got)
	}
}

func TestDetectNativeApprovalSelectorRequiresKeyHints(t *testing.T) {
	// セレクタ許容はキー操作ヒント行（Enter to ... + Esc to cancel）が揃う場合のみ。
	// ヒントなしのカーソル付き番号リスト（AI 応答の箇条書き等）は引き続き拒否する。
	lines := []string{
		"Pick one of the following:",
		"❯ 1. First plan",
		"  2. Second plan",
		"  3. Third plan",
	}
	if got := detectNativeApproval("claude", lines); got != nil {
		t.Fatalf("detectNativeApproval = %+v, want nil", got)
	}
}

func TestDetectNativeApprovalFalsePositiveNumberedList(t *testing.T) {
	lines := []string{
		"Implementation plan:",
		"❯ 1. Read the files",
		"  2. Edit the code",
		"  3. Run tests",
	}
	if got := detectNativeApproval("claude", lines); got != nil {
		t.Fatalf("detectNativeApproval = %+v, want nil", got)
	}
}

func TestDetectNativeApprovalFromANSIGolden(t *testing.T) {
	raw, err := os.ReadFile("testdata/approval_codex_shortcut_ansi.ansi")
	if err != nil {
		t.Fatal(err)
	}
	raw = bytes.ReplaceAll(raw, []byte(`\x1b`), []byte{0x1b})
	raw = bytes.ReplaceAll(raw, []byte("\n"), []byte("\r\n"))

	vt := newVTBuffer(120, 30)
	vt.Write(raw)
	got := detectNativeApproval("codex", vt.TailLines(vtTailLinesForApproval))
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Question != "Run: rm -rf ./tmp" {
		t.Fatalf("question = %q", got.Question)
	}
	if got.Kind != "native_codex_shortcut" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 4 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	if got.Options[0].SendText != "y" || !got.Options[0].IsCurrent {
		t.Fatalf("first option = %+v", got.Options[0])
	}
	if got.Options[3].SendText != "\x1b" {
		t.Fatalf("cancel send_text = %q", got.Options[3].SendText)
	}
}

func TestExtractNativeApprovalOptionsBranches(t *testing.T) {
	t.Run("chooses longest cluster after gap", func(t *testing.T) {
		lines := []string{
			"❯ 1. Old allow",
			"  2. Old deny",
			"",
			"",
			"",
			"",
			"",
			"  1. Allow once",
			"❯ 2. Allow for this session",
			"  3. No",
		}
		opts, start, end := extractNativeApprovalOptions("claude", lines)
		if len(opts) != 3 || start != 7 || end != 9 {
			t.Fatalf("opts/start/end = %d/%d/%d, want 3/7/9 (%+v)", len(opts), start, end, opts)
		}
	})

	t.Run("rejects menu without cursor or send text", func(t *testing.T) {
		lines := []string{
			"  1. Yes, allow once",
			"  2. No",
		}
		opts, _, _ := extractNativeApprovalOptions("claude", lines)
		if opts != nil {
			t.Fatalf("opts = %+v, want nil", opts)
		}
	})

	t.Run("accepts codex shortcut send text without cursor", func(t *testing.T) {
		lines := []string{
			"  Yes (y)",
			"  Yes, and don't ask again for this command (p)",
			"  No (n)",
		}
		opts, _, _ := extractNativeApprovalOptions("codex", lines)
		if len(opts) != 3 {
			t.Fatalf("opts len = %d, want 3 (%+v)", len(opts), opts)
		}
		if opts[0].SendText != "y" || opts[1].SendText != "p" || opts[2].SendText != "n" {
			t.Fatalf("send_text values = %+v", opts)
		}
	})

	t.Run("accepts copilot shortcut send text without cursor", func(t *testing.T) {
		lines := []string{
			"  Allow once (y)",
			"  Deny once (n)",
			"  Allow all similar for this session (!)",
		}
		opts, _, _ := extractNativeApprovalOptions("copilot", lines)
		if len(opts) != 3 {
			t.Fatalf("opts len = %d, want 3 (%+v)", len(opts), opts)
		}
		if opts[0].SendText != "y" || opts[1].SendText != "n" || opts[2].SendText != "!" {
			t.Fatalf("send_text values = %+v", opts)
		}
	})

	t.Run("rejects option count above cap", func(t *testing.T) {
		lines := make([]string, 0, approvalMaxOptions+1)
		for i := 1; i <= approvalMaxOptions+1; i++ {
			prefix := "  "
			if i == 1 {
				prefix = "❯ "
			}
			lines = append(lines, fmt.Sprintf("%s%d. Allow", prefix, i))
		}
		opts, _, _ := extractNativeApprovalOptions("claude", lines)
		if opts != nil {
			t.Fatalf("opts = %+v, want nil", opts)
		}
	})

	t.Run("trims box drawing around option labels", func(t *testing.T) {
		lines := []string{
			"│ ❯ 1. Yes, allow once │",
			"│   2. No │",
		}
		opts, _, _ := extractNativeApprovalOptions("claude", lines)
		if len(opts) != 2 {
			t.Fatalf("opts len = %d, want 2 (%+v)", len(opts), opts)
		}
		if opts[0].Label != "Yes, allow once" || opts[1].Label != "No" {
			t.Fatalf("labels = %q / %q", opts[0].Label, opts[1].Label)
		}
	})
}

func TestNativeApprovalLooksValid(t *testing.T) {
	claudeOpts := []proto.ApprovalOption{
		{Label: "Yes, allow once", IsCurrent: true},
		{Label: "No"},
	}
	if !nativeApprovalLooksValid("claude", []string{"Allow tool: Bash"}, claudeOpts) {
		t.Fatal("claude approval with hint and approval labels should be valid")
	}
	if nativeApprovalLooksValid("claude", []string{"Implementation plan:"}, claudeOpts) {
		t.Fatal("claude approval without hint should be invalid")
	}

	codexOpts := []proto.ApprovalOption{
		{Label: "Run command (y)", SendText: "y"},
		{Label: "Cancel (esc)", SendText: "\x1b"},
	}
	if !nativeApprovalLooksValid("codex", []string{"This command requires approval"}, codexOpts) {
		t.Fatal("codex shortcut with native hint should be valid")
	}
	if nativeApprovalLooksValid("codex", []string{"Choose a branch"}, codexOpts) {
		t.Fatal("codex shortcut without hint should be invalid")
	}

	copilotOpts := []proto.ApprovalOption{
		{Label: "Allow once (y)", SendText: "y"},
		{Label: "Deny once (n)", SendText: "n"},
	}
	if !nativeApprovalLooksValid("copilot", []string{"Permission required"}, copilotOpts) {
		t.Fatal("copilot shortcut with native hint should be valid")
	}
	if nativeApprovalLooksValid("copilot", []string{"Choose a branch"}, copilotOpts) {
		t.Fatal("copilot shortcut without hint should be invalid")
	}

	cursorAgentOpts := []proto.ApprovalOption{
		{Label: "Allow once (y)", SendText: "y"},
		{Label: "Deny once (n)", SendText: "n"},
	}
	if !nativeApprovalLooksValid("cursor-agent", []string{"Permission required"}, cursorAgentOpts) {
		t.Fatal("cursor-agent shortcut with native hint should be valid")
	}
	if nativeApprovalLooksValid("cursor-agent", []string{"Choose a branch"}, cursorAgentOpts) {
		t.Fatal("cursor-agent shortcut without hint should be invalid")
	}
}

func TestDetectNativeApprovalUsesRecentLineLimit(t *testing.T) {
	lines := []string{
		"Allow tool: Bash",
		"❯ 1. Yes, allow once",
		"  2. No",
	}
	for len(lines) <= approvalRecentLines {
		lines = append(lines, "filler")
	}
	if got := detectNativeApproval("claude", lines); got != nil {
		t.Fatalf("old approval outside recent limit should be ignored: %+v", got)
	}
}

func TestCleanNativeApprovalLabelCompactsWhitespace(t *testing.T) {
	got := cleanNativeApprovalLabel(" │  Yes,\t allow   once  │ ")
	if got != "Yes, allow once" {
		t.Fatalf("clean label = %q", got)
	}
	if strings.TrimSpace(got) != got {
		t.Fatalf("clean label has outer whitespace: %q", got)
	}
}
