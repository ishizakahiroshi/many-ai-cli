package hub

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func TestUsageProbeManagerRejectsDuplicateAndCancels(t *testing.T) {
	manager := newUsageProbeManager()
	ctx, cancel := context.WithCancel(context.Background())
	state := &usageProbeState{cancel: cancel}
	key := usageProbeKey("claude", "max-a")
	if !manager.begin(key, state) {
		t.Fatal("first usage probe was not registered")
	}
	if manager.begin(key, &usageProbeState{}) {
		t.Fatal("duplicate usage probe was accepted")
	}
	if !manager.cancel(key) {
		t.Fatal("running usage probe was not cancellable")
	}
	if ctx.Err() != context.Canceled {
		t.Fatalf("cancel did not reach probe context: %v", ctx.Err())
	}
	manager.finish(key)
	if manager.isRunning("claude", "max-a") {
		t.Fatal("finished usage probe still reported as running")
	}
}

func TestCleanupUsageProbeTranscriptsKeepsNewestAndProtectsOtherProjects(t *testing.T) {
	root := t.TempDir()
	profileDir := filepath.Join(root, "profile")
	probeCWD := filepath.Join(root, "usage-probe")
	target := filepath.Join(profileDir, "projects", claudeProjectDirName(probeCWD))
	other := filepath.Join(profileDir, "projects", claudeProjectDirName(filepath.Join(root, "usage-probe-extra")))
	if err := os.MkdirAll(target, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(other, 0o700); err != nil {
		t.Fatal(err)
	}
	base := time.Now().Add(-10 * time.Minute)
	for i := 0; i < 5; i++ {
		path := filepath.Join(target, "probe-"+string(rune('a'+i))+".jsonl")
		if err := os.WriteFile(path, []byte("synthetic"), 0o600); err != nil {
			t.Fatal(err)
		}
		stamp := base.Add(time.Duration(i) * time.Minute)
		if err := os.Chtimes(path, stamp, stamp); err != nil {
			t.Fatal(err)
		}
	}
	otherPath := filepath.Join(other, "user-transcript.jsonl")
	if err := os.WriteFile(otherPath, []byte("must remain"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "keep.txt"), []byte("not a transcript"), 0o600); err != nil {
		t.Fatal(err)
	}

	removed, err := cleanupUsageProbeTranscripts(profileDir, probeCWD)
	if err != nil {
		t.Fatal(err)
	}
	if removed != 2 {
		t.Fatalf("removed=%d, want 2", removed)
	}
	entries, err := os.ReadDir(target)
	if err != nil {
		t.Fatal(err)
	}
	jsonlCount := 0
	for _, entry := range entries {
		if filepath.Ext(entry.Name()) == ".jsonl" {
			jsonlCount++
		}
	}
	if jsonlCount != usageProbeTranscriptKeep {
		t.Fatalf("probe transcript count=%d, want %d", jsonlCount, usageProbeTranscriptKeep)
	}
	if _, err := os.Stat(otherPath); err != nil {
		t.Fatalf("other project transcript was removed: %v", err)
	}
	if _, err := os.Stat(filepath.Join(target, "keep.txt")); err != nil {
		t.Fatalf("non-transcript file was removed: %v", err)
	}
}

func TestUsageProbeConfirmDialogMatchesStartupPickers(t *testing.T) {
	cases := []struct {
		name   string
		screen string
		want   bool
	}{
		{
			name: "folder trust",
			screen: "Quick safety check\n" +
				"Is this a project you created or one you trust?\n" +
				"1. Yes, I trust this folder\n" +
				"2. No, exit\n" +
				"Enter to confirm",
			want: true,
		},
		{
			name: "external imports",
			screen: "Allow external CLAUDE.md file imports?\n" +
				"This project's CLAUDE.md imports files outside the current working directory.\n" +
				"1. Yes, allow external imports\n" +
				"2. No, disable external imports",
			want: true,
		},
		{
			name:   "ready prompt",
			screen: "Try something like:\n> write a test\n",
			want:   false,
		},
		{
			name: "tool approval",
			screen: "Bash command\nls\nDo you want to proceed?\n" +
				"1. Yes\n2. No\nEnter to confirm",
			want: false,
		},
		{name: "empty", screen: "", want: false},
	}
	for _, tc := range cases {
		if got := usageProbeConfirmDialog(tc.screen); got != tc.want {
			t.Fatalf("%s: got %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestUsageProbeScreenIsConfirmDialogReadsVT(t *testing.T) {
	s := &Server{sessions: map[int]*session{}}
	vt := newVTBuffer(80, 12)
	vt.Write([]byte("Is this a project you created or one you trust?\r\nYes, I trust this folder\r\n"))
	s.sessions[3] = &session{ID: 3, vt: vt}
	if !s.usageProbeScreenIsConfirmDialog(3) {
		t.Fatal("folder-trust screen was not detected")
	}
	if s.usageProbeScreenIsConfirmDialog(99) {
		t.Fatal("missing session looked like a confirm dialog")
	}
}

// usageProbePicker は、選択肢の並びとカーソル位置から picker の画面行を組み立てる。
// cursor は options の添字。
func usageProbePicker(title string, options []string, cursor int) []string {
	lines := []string{title, ""}
	for i, opt := range options {
		prefix := "  "
		if i == cursor {
			prefix = "❯ "
		}
		lines = append(lines, prefix+opt)
	}
	return append(lines, "", "Enter to confirm · Esc to cancel")
}

// usageProbeKeysUntilEnter は usageProbeDialogKey の出したキーを picker に当てていき、
// Enter が出るまでのキー列と、Enter の時点でカーソルが指していた選択肢を返す。
func usageProbeKeysUntilEnter(t *testing.T, title string, options []string, cursor int) ([]string, string) {
	t.Helper()
	var keys []string
	for step := 0; step <= usageProbeMaxCursorMoves; step++ {
		key, ok := usageProbeDialogKey(usageProbePicker(title, options, cursor))
		if !ok {
			t.Fatalf("no key decided for cursor=%d options=%q", cursor, options)
		}
		keys = append(keys, key)
		switch key {
		case "\r":
			return keys, options[cursor]
		case "\x1b[B":
			cursor++
		case "\x1b[A":
			cursor--
		}
		if cursor < 0 || cursor >= len(options) {
			t.Fatalf("cursor left the picker: keys=%q", keys)
		}
	}
	t.Fatalf("no Enter within %d moves: keys=%q", usageProbeMaxCursorMoves, keys)
	return nil, ""
}

// TestUsageProbeDialogKeyChoosesOptionByText は、Claude Code の版によって選択肢の並びが
// 変わっても、プローブが「信頼する」「読み込みを許可する」側を文字で選ぶことを確認する。
// 2.1.281（2026-09-24 観測）はフォルダ信頼の既定が先頭の「No, exit」で、以前の Enter
// だけの実装はこれを選んでプローブ用の Claude を終了させていた。
func TestUsageProbeDialogKeyChoosesOptionByText(t *testing.T) {
	const trustTitle = "Quick safety check: Is this a project you created or one you trust?"
	cases := []struct {
		name     string
		title    string
		options  []string
		cursor   int
		wantKeys []string
		wantPick string
	}{
		{"2.1.281 no-first", trustTitle, []string{"No, exit", "Yes, I trust this folder"}, 0, []string{"\x1b[B", "\r"}, "Yes, I trust this folder"},
		{"2026-08-20 yes-first", trustTitle, []string{"1. Yes, I trust this folder", "2. No, exit"}, 0, []string{"\r"}, "1. Yes, I trust this folder"},
		{"cursor below target", trustTitle, []string{"1. Yes, I trust this folder", "2. No, exit"}, 1, []string{"\x1b[A", "\r"}, "1. Yes, I trust this folder"},
		{"external imports", "Allow external CLAUDE.md file imports?", []string{"1. Yes, allow external imports", "2. No, disable external imports"}, 0, []string{"\r"}, "1. Yes, allow external imports"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			keys, picked := usageProbeKeysUntilEnter(t, tc.title, tc.options, tc.cursor)
			if strings.Join(keys, "|") != strings.Join(tc.wantKeys, "|") {
				t.Fatalf("keys = %q, want %q", keys, tc.wantKeys)
			}
			if picked != tc.wantPick {
				t.Fatalf("confirmed %q, want %q", picked, tc.wantPick)
			}
		})
	}
}

// TestUsageProbeDialogKeyRefusesToGuess は、選ぶ選択肢かカーソルが画面に見つからないとき、
// キーを決めない（＝何も送らない）ことを確認する。
func TestUsageProbeDialogKeyRefusesToGuess(t *testing.T) {
	if _, ok := usageProbeDialogKey(usageProbePicker("Is this a project you created or one you trust?", []string{"Leave", "Trust"}, 0)); ok {
		t.Fatal("decided a key although no known option is on screen")
	}
	if _, ok := usageProbeDialogKey([]string{"Is this a project you created or one you trust?", "  No, exit", "  Yes, I trust this folder"}); ok {
		t.Fatal("decided a key although no cursor is on screen")
	}
}

// usageProbePickerSessionID は使用量の記録（パッケージ全体で共有）が他のテストと混ざらない ID。
const usageProbePickerSessionID = 4242

// usageProbePickerServer は、プローブが送るキーに合わせて picker を描き直す偽 wrapper を
// 付けた claude セッション（ID usageProbePickerSessionID）を用意する。Enter でカーソルの選択肢が確定し、送られた
// pty_input は順に frames へ記録される。
func usageProbePickerServer(options []string, cursor int) (*Server, *[]string, *sync.Mutex, *string) {
	s := newTestServer()
	ses := registerTestSession(s, usageProbePickerSessionID, "claude")
	const title = "Quick safety check: Is this a project you created or one you trust?"
	ses.vt = newVTBuffer(100, 30)
	ses.vt.Write([]byte(strings.Join(usageProbePicker(title, options, cursor), "\r\n")))
	// 実機の CLI は picker を描いてから静止する。waitForInputReady は出力が一度も無い
	// セッションを上限まで待つので、描いた時刻を入れておく。
	ses.lastOutputAt = time.Now().Add(-time.Second)
	var mu sync.Mutex
	frames := []string{}
	confirmed := ""
	s.wrappers[usageProbePickerSessionID] = &wrapperConn{sendFunc: func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok || msg.Type != "pty_input" {
			return nil
		}
		data := string(msg.Data)
		mu.Lock()
		frames = append(frames, data)
		switch data {
		case "\x1b[B":
			cursor++
		case "\x1b[A":
			cursor--
		case "\r":
			if confirmed == "" && cursor >= 0 && cursor < len(options) {
				confirmed = options[cursor]
			}
		}
		cur := cursor
		done := confirmed != ""
		mu.Unlock()
		s.sessionsMu.Lock()
		if live := s.sessions[usageProbePickerSessionID]; live != nil {
			live.lastOutputAt = time.Now()
			live.vt = newVTBuffer(100, 30)
			if done {
				live.vt.Write([]byte("Welcome back\r\n"))
			} else {
				live.vt.Write([]byte(strings.Join(usageProbePicker(title, options, cur), "\r\n")))
			}
		}
		s.sessionsMu.Unlock()
		return nil
	}}
	return s, &frames, &mu, &confirmed
}

// TestDriveUsageProbeSessionMovesToTrustBeforeEnter は、2.1.281 の並び（先頭が No, exit）で
// プローブが ↓ を送ってから Enter を押し、「Yes, I trust this folder」を確定させることを
// 通しで確認する。
func TestDriveUsageProbeSessionMovesToTrustBeforeEnter(t *testing.T) {
	s, frames, mu, confirmed := usageProbePickerServer([]string{"No, exit", "Yes, I trust this folder"}, 0)
	ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
	defer cancel()
	_ = s.driveUsageProbeSession(ctx, usageProbePickerSessionID)

	mu.Lock()
	defer mu.Unlock()
	if *confirmed != "Yes, I trust this folder" {
		t.Fatalf("confirmed %q, want the trust option; frames=%q", *confirmed, *frames)
	}
	if len(*frames) < 2 || (*frames)[0] != "\x1b[B" || (*frames)[1] != "\r" {
		t.Fatalf("frames = %q, want ↓ then Enter first", *frames)
	}
}

// TestDriveUsageProbeSessionSendsNothingOnUnknownPicker は、知らない文言の picker では
// キーを 1 つも送らずに errUsageProbeUnknownDialog で終わることを確認する。
func TestDriveUsageProbeSessionSendsNothingOnUnknownPicker(t *testing.T) {
	s, frames, mu, _ := usageProbePickerServer([]string{"Leave", "Trust"}, 0)
	ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
	defer cancel()
	err := s.driveUsageProbeSession(ctx, usageProbePickerSessionID)

	if !errors.Is(err, errUsageProbeUnknownDialog) {
		t.Fatalf("err = %v, want errUsageProbeUnknownDialog", err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(*frames) != 0 {
		t.Fatalf("frames = %q, want nothing sent", *frames)
	}
}
