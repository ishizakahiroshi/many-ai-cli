package hub

import (
	"log/slog"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

// newTestServer は最小構成の Server を返す。UI WebSocket が 0 件なので
// broadcast は no-op となり、state machine 単体テストに使用できる。
func newTestServer() *Server {
	cfg := &config.Config{}
	cfg.Hub.Port = 47777
	cfg.Orchestration.MaxChildrenPerParent = 4
	cfg.Orchestration.MaxTotalSessions = 16
	// Most unit tests exercise a spawn's downstream behavior without a browser.
	// Production defaults are applied by config.Load; keep this bare test
	// fixture non-interactive unless a test opts into the confirmation gate.
	cfg.Orchestration.SpawnConfirmMode = config.SpawnConfirmOff
	return &Server{
		cfg:                 cfg,
		logger:              slog.Default(),
		sessions:            map[int]*session{},
		wrappers:            map[int]*wrapperConn{},
		uis:                 map[*websocket.Conn]*uiConn{},
		pendingInput:        map[int][]string{},
		slashCmdCache:       map[string]*slashCmdCacheEntry{},
		approvalRuleTargets: map[string]approvalRuleTarget{},
		usageLinkCache:      newUsageLinkCache(),
		installLinkCache:    newInstallLinkCache(),
		modelsCache:         &modelsCache{},
		modelsRemoteCache:   newModelsRemoteCache(),
		orchestration:       newOrchestrationManager(),
	}
}

// registerTestSession はテスト用セッションを Server に登録する。
func registerTestSession(s *Server, id int, provider string) *session {
	ses := &session{
		ID:       id,
		Provider: provider,
		State:    "running",
		inputMu:  new(sync.Mutex), // AUDIT-11: inputMu はポインタ。本番の session 生成と同様 allocate する
	}
	s.sessionsMu.Lock()
	s.sessions[id] = ses
	s.sessionsMu.Unlock()
	return ses
}

func registerTestUI(s *Server) *websocket.Conn {
	conn := &websocket.Conn{}
	s.sessionsMu.Lock()
	s.uis[conn] = newUIConn(conn)
	s.sessionsMu.Unlock()
	return conn
}

func TestResizeOwnershipRejectsNonControllingUI(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	owner := registerTestUI(s)
	observer := registerTestUI(s)

	s.sessionsMu.Lock()
	_, _, _, _ = s.claimResizeOwnershipLocked(owner, 1, 120, 40)
	s.sessionsMu.Unlock()
	s.handleResizeFromUI(observer, proto.Message{
		Type: "pty_resize", SessionID: 1, Cols: 80, Rows: 24,
	})

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.controllingUI != owner {
		t.Fatal("resize ownership unexpectedly changed")
	}
	if ses.lastCols != 120 || ses.lastRows != 40 {
		t.Fatalf("non-owner resize reached session state: got %dx%d", ses.lastCols, ses.lastRows)
	}
}

func TestResizeOwnershipTransfersWithActiveSession(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	first := registerTestUI(s)
	second := registerTestUI(s)

	s.sessionsMu.Lock()
	_, _, _, _ = s.claimResizeOwnershipLocked(first, 1, 120, 40)
	_, _, _, _ = s.claimResizeOwnershipLocked(second, 1, 90, 30)
	defer s.sessionsMu.Unlock()
	if ses.controllingUI != second {
		t.Fatal("active UI did not acquire resize ownership")
	}
	if ses.lastCols != 90 || ses.lastRows != 30 {
		t.Fatalf("new owner's latest size was not applied: got %dx%d", ses.lastCols, ses.lastRows)
	}
}

func TestResizeOwnershipClearsWhenUIIsRemoved(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	owner := registerTestUI(s)

	s.sessionsMu.Lock()
	_, _, _, _ = s.claimResizeOwnershipLocked(owner, 1, 120, 40)
	delete(s.uis, owner)
	s.releaseResizeOwnershipLocked(owner)
	s.sessionsMu.Unlock()

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.controllingUI != nil {
		t.Fatal("resize ownership was not cleared when UI disconnected")
	}
}

func TestResizeOwnershipFirstRemainingUIClaimsAfterOwnerDisconnect(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	owner := registerTestUI(s)
	remainingA := registerTestUI(s)
	remainingB := registerTestUI(s)

	s.sessionsMu.Lock()
	_, _, _, _ = s.claimResizeOwnershipLocked(owner, 1, 120, 40)
	delete(s.uis, owner)
	s.releaseResizeOwnershipLocked(owner)
	s.sessionsMu.Unlock()

	// The first resize after the old owner disconnects atomically acquires
	// ownership, even when the size itself is unchanged.
	s.handleResizeFromUI(remainingA, proto.Message{
		Type: "pty_resize", SessionID: 1, Cols: 120, Rows: 40,
	})
	// A later resize from another remaining UI must be rejected.
	s.handleResizeFromUI(remainingB, proto.Message{
		Type: "pty_resize", SessionID: 1, Cols: 80, Rows: 24,
	})

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.controllingUI != remainingA {
		t.Fatal("first remaining UI did not acquire resize ownership")
	}
	if ses.lastCols != 120 || ses.lastRows != 40 {
		t.Fatalf("non-owner resize reached session state: got %dx%d", ses.lastCols, ses.lastRows)
	}
}

func TestResizeOwnershipTwoUIsDoNotBounceWrapperSize(t *testing.T) {
	received := make(chan proto.Message, 8)
	wsServer := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		for {
			var message proto.Message
			if err := websocket.JSON.Receive(conn, &message); err != nil {
				return
			}
			received <- message
		}
	}))
	defer wsServer.Close()
	wsURL := "ws" + strings.TrimPrefix(wsServer.URL, "http")
	wrapperWS, err := websocket.Dial(wsURL, "", "http://127.0.0.1/")
	if err != nil {
		t.Fatalf("dial wrapper websocket: %v", err)
	}
	defer wrapperWS.Close()

	uiServer := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		for {
			var message proto.Message
			if err := websocket.JSON.Receive(conn, &message); err != nil {
				return
			}
		}
	}))
	defer uiServer.Close()
	uiURL := "ws" + strings.TrimPrefix(uiServer.URL, "http")
	dialUI := func() *websocket.Conn {
		t.Helper()
		conn, err := websocket.Dial(uiURL, "", "http://127.0.0.1/")
		if err != nil {
			t.Fatalf("dial UI websocket: %v", err)
		}
		return conn
	}
	first := dialUI()
	defer first.Close()
	second := dialUI()
	defer second.Close()

	s := newTestServer()
	registerTestSession(s, 1, "codex")
	s.sessionsMu.Lock()
	s.uis[first] = newUIConn(first)
	s.uis[second] = newUIConn(second)
	s.wrappers[1] = newWrapperConn(wrapperWS)
	s.sessionsMu.Unlock()

	assertResize := func(cols, rows int) {
		t.Helper()
		select {
		case got := <-received:
			if got.Type != "pty_resize" || got.Cols != cols || got.Rows != rows {
				t.Fatalf("wrapper resize = %+v, want %dx%d", got, cols, rows)
			}
		case <-time.After(2 * time.Second):
			t.Fatalf("timed out waiting for wrapper resize %dx%d", cols, rows)
		}
	}
	assertNoResize := func() {
		t.Helper()
		select {
		case got := <-received:
			t.Fatalf("unexpected wrapper resize from non-owner: %+v", got)
		case <-time.After(100 * time.Millisecond):
		}
	}

	s.handleResizeFromUI(first, proto.Message{Type: "pty_resize", SessionID: 1, Cols: 120, Rows: 40})
	assertResize(120, 40)

	for i := 0; i < 5; i++ {
		s.handleResizeFromUI(second, proto.Message{Type: "pty_resize", SessionID: 1, Cols: 80 + i, Rows: 24})
	}
	assertNoResize()

	s.claimResizeOwnership(second, 1, 84, 24)
	assertResize(84, 24)
	s.handleResizeFromUI(first, proto.Message{Type: "pty_resize", SessionID: 1, Cols: 121, Rows: 41})
	assertNoResize()
}

// TestHandleNativeApprovalDetection_NewApproval は新しい承認が検出されたとき
// ネイティブの記録が開くことを確認する。
func TestHandleNativeApprovalDetection_NewApproval(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "claude")

	approval := &nativeApproval{
		Sig:      "sig-abc",
		Kind:     "native",
		Question: "Allow bash?",
	}
	s.handleNativeApprovalDetection(1, approval)

	s.sessionsMu.Lock()
	got := nativeRecordSig(s.sessions[1])
	s.sessionsMu.Unlock()
	if got != "sig-abc" {
		t.Fatalf("native record sig = %q, want %q", got, "sig-abc")
	}
}

// TestHandleNativeApprovalDetection_SameSigNoRepeat は同じ候補の再検出で
// 記録が変わらず、開く approval_state を配信し直さないことを確認する。
func TestHandleNativeApprovalDetection_SameSigNoRepeat(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 2, "claude")

	approval := &nativeApproval{Sig: "sig-dup", Kind: "native"}
	s.handleNativeApprovalDetection(2, approval)
	s.sessionsMu.Lock()
	first := s.sessions[2].pendingApproval
	s.sessionsMu.Unlock()

	// 同じ候補で再度呼ぶ → broadcast されない（記録も変わらない）
	sent := captureUIBroadcasts(s)
	s.handleNativeApprovalDetection(2, &nativeApproval{Sig: "sig-dup", Kind: "native"})

	s.sessionsMu.Lock()
	got := s.sessions[2].pendingApproval
	s.sessionsMu.Unlock()
	if got != first || nativeRecordSig(s.sessions[2]) != "sig-dup" {
		t.Fatalf("native record changed on re-detection: %+v -> %+v", first, got)
	}
	if opens := approvalStateOpens(sent(), 2); len(opens) != 0 {
		t.Fatalf("same candidate was broadcast again: %+v", opens)
	}
}

// TestHandleNativeApprovalDetection_ClearOnNil は承認が nil の状態が連続したとき
// 記録が閉じることを確認する。
func TestHandleNativeApprovalDetection_ClearOnNil(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 3, "claude")
	ses.pendingApproval = testNativeRecord("sig-to-clear", "", 0)

	for i := 0; i < nativeApprovalClearMissLimit; i++ {
		s.handleNativeApprovalDetection(3, nil)
	}

	s.sessionsMu.Lock()
	got := s.sessions[3].pendingApproval
	s.sessionsMu.Unlock()
	if got != nil {
		t.Fatalf("pending record = %+v after clear, want nil", *got)
	}
}

// TestHandleNativeApprovalDetection_TransientNilKeepsSig は Codex TUI の一時的な
// 再描画抜けで記録が即時に閉じないことを確認する。
func TestHandleNativeApprovalDetection_TransientNilKeepsSig(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 8, "codex")
	ses.pendingApproval = testNativeRecord("sig-stable", "", 0)

	for i := 0; i < nativeApprovalClearMissLimit-1; i++ {
		s.handleNativeApprovalDetection(8, nil)
	}

	s.sessionsMu.Lock()
	got := nativeRecordSig(s.sessions[8])
	misses := s.sessions[8].nativeApprovalClearMisses
	s.sessionsMu.Unlock()
	if got != "sig-stable" {
		t.Fatalf("native record sig = %q before clear threshold, want %q", got, "sig-stable")
	}
	if misses != nativeApprovalClearMissLimit-1 {
		t.Fatalf("nativeApprovalClearMisses = %d, want %d", misses, nativeApprovalClearMissLimit-1)
	}
}

// TestHandleNativeApprovalDetection_DetectionResetsClearMisses は再検出で clear miss が
// リセットされることを確認する。
func TestHandleNativeApprovalDetection_DetectionResetsClearMisses(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 9, "codex")
	ses.pendingApproval = testNativeRecord("sig-stable", "", 0)

	s.handleNativeApprovalDetection(9, nil)
	s.handleNativeApprovalDetection(9, &nativeApproval{Sig: "sig-stable", Kind: "native"})

	s.sessionsMu.Lock()
	misses := s.sessions[9].nativeApprovalClearMisses
	s.sessionsMu.Unlock()
	if misses != 0 {
		t.Fatalf("nativeApprovalClearMisses = %d, want 0", misses)
	}
}

func TestResetNativeApprovalClearMisses(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 17, "codex")
	ses.nativeApprovalClearMisses = nativeApprovalClearMissLimit - 1

	s.resetNativeApprovalClearMisses(17)

	s.sessionsMu.Lock()
	misses := s.sessions[17].nativeApprovalClearMisses
	s.sessionsMu.Unlock()
	if misses != 0 {
		t.Fatalf("nativeApprovalClearMisses = %d, want 0", misses)
	}
}

func TestSplitBracketedPasteSubmit(t *testing.T) {
	tests := []struct {
		name        string
		input       string
		wantFirst   string
		wantDelayed string
	}{
		{
			name:        "bracketed paste submit",
			input:       "\x1b[200~line 1\nline 2\x1b[201~\r",
			wantFirst:   "\x1b[200~line 1\nline 2\x1b[201~",
			wantDelayed: "\r",
		},
		{
			name:        "plain submit stays together",
			input:       "hello\r",
			wantFirst:   "hello\r",
			wantDelayed: "",
		},
		{
			name:        "bracketed paste without submit stays together",
			input:       "\x1b[200~line 1\nline 2\x1b[201~",
			wantFirst:   "\x1b[200~line 1\nline 2\x1b[201~",
			wantDelayed: "",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			gotFirst, gotDelayed := splitBracketedPasteSubmit(tc.input)
			if gotFirst != tc.wantFirst || gotDelayed != tc.wantDelayed {
				t.Fatalf("splitBracketedPasteSubmit() = (%q, %q), want (%q, %q)", gotFirst, gotDelayed, tc.wantFirst, tc.wantDelayed)
			}
		})
	}
}

func TestShouldSuppressNativeApprovalClearMiss(t *testing.T) {
	tests := []struct {
		name     string
		provider string
		lines    []string
		want     bool
	}{
		{
			name:     "codex mostly blank redraw",
			provider: "codex",
			lines:    []string{"", " ", "•", "", " "},
			want:     true,
		},
		{
			name:     "copilot mostly blank redraw",
			provider: "copilot",
			lines:    []string{"", "status", ""},
			want:     true,
		},
		{
			name:     "claude numbered prompt uses normal clear misses",
			provider: "claude",
			lines:    []string{"", " "},
			want:     false,
		},
		{
			name:     "codex nonblank output after clear",
			provider: "codex",
			lines:    []string{"Running command", "line 2", "line 3"},
			want:     false,
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldSuppressNativeApprovalClearMiss(tc.provider, tc.lines); got != tc.want {
				t.Fatalf("shouldSuppressNativeApprovalClearMiss() = %v, want %v", got, tc.want)
			}
		})
	}
}

// TestHandleNativeApprovalDetection_ConsumedTTL は consumed TTL 内の同一 sig が
// 再度検出されないことを確認する。
func TestHandleNativeApprovalDetection_ConsumedTTL(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 4, "claude")
	ses.nativeApprovalConsumed = "sig-consumed"
	ses.nativeApprovalConsumedAt = time.Now() // TTL 内

	approval := &nativeApproval{Sig: "sig-consumed", Kind: "native"}
	s.handleNativeApprovalDetection(4, approval)

	s.sessionsMu.Lock()
	got := nativeRecordSig(s.sessions[4])
	s.sessionsMu.Unlock()
	// TTL 内なので記録が開いていないこと
	if got != "" {
		t.Fatalf("native record sig = %q, want empty (TTL suppression)", got)
	}
}

// TestHandleNativeApprovalDetection_ResizeDebounceSkip は resize debounce 中に
// 承認が検出されないことを確認する。
func TestHandleNativeApprovalDetection_ResizeDebounceSkip(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 5, "claude")
	ses.vtResizeDebounceUntil = time.Now().Add(5 * time.Second) // debounce 中

	approval := &nativeApproval{Sig: "sig-resize", Kind: "native"}
	s.handleNativeApprovalDetection(5, approval)

	s.sessionsMu.Lock()
	got := nativeRecordSig(s.sessions[5])
	s.sessionsMu.Unlock()
	if got != "" {
		t.Fatalf("native record sig = %q, want empty (debounce skip)", got)
	}
}

// TestMarkNativeApprovalConsumed は consumed マークが正しくセットされ、
// 記録を閉じる条件（sig 一致）が満たされることを確認する。
func TestMarkNativeApprovalConsumed(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 6, "claude")
	ses.pendingApproval = testNativeRecord("sig-to-consume", "", 0)

	s.markNativeApprovalConsumed(proto.Message{
		SessionID:   6,
		ApprovalSig: "sig-to-consume",
	})

	s.sessionsMu.Lock()
	consumed := s.sessions[6].nativeApprovalConsumed
	sig := nativeRecordSig(s.sessions[6])
	s.sessionsMu.Unlock()
	if consumed != "sig-to-consume" {
		t.Fatalf("nativeApprovalConsumed = %q, want %q", consumed, "sig-to-consume")
	}
	// 記録が閉じていること
	if sig != "" {
		t.Fatalf("native record sig = %q, want empty after consume", sig)
	}
}

// TestMarkNativeApprovalConsumed_SigMismatch は sig 不一致の場合に
// ネイティブの記録が閉じないことを確認する。
func TestMarkNativeApprovalConsumed_SigMismatch(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 7, "claude")
	ses.pendingApproval = testNativeRecord("sig-A", "", 0)

	s.markNativeApprovalConsumed(proto.Message{
		SessionID:   7,
		ApprovalSig: "sig-B", // 別の sig
	})

	s.sessionsMu.Lock()
	sig := nativeRecordSig(s.sessions[7])
	s.sessionsMu.Unlock()
	if sig != "sig-A" {
		t.Fatalf("native record sig = %q, want %q (mismatch should not clear)", sig, "sig-A")
	}
}

// TestEvaluateIdle_RunningToWaiting は running セッションが idleAfter 経過後に
// 保留中の承認の記録があれば waiting に遷移することを確認する。
func TestEvaluateIdle_RunningToWaiting(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 10, "claude")
	ses.pendingApproval = testNativeRecord("sig-waiting", "", 0)
	ses.lastOutputAt = time.Now().Add(-(idleAfter + time.Millisecond))

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[10].State
	s.sessionsMu.Unlock()
	if state != "waiting" {
		t.Fatalf("state = %q, want %q", state, "waiting")
	}
}

// TestEvaluateIdle_RunningToStandby は running セッションが idleAfter 経過後に
// 保留中の承認の記録が無ければ standby に遷移することを確認する。
func TestEvaluateIdle_RunningToStandby(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 11, "claude")
	ses.lastOutputAt = time.Now().Add(-(idleAfter + time.Millisecond))

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[11].State
	s.sessionsMu.Unlock()
	if state != "standby" {
		t.Fatalf("state = %q, want %q", state, "standby")
	}
}

// TestEvaluateIdle_WaitingToStandby は waiting セッションで保留中の承認の記録が
// 閉じていれば standby に遷移することを確認する。
func TestEvaluateIdle_WaitingToStandby(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 12, "claude")
	ses.State = "waiting"

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[12].State
	s.sessionsMu.Unlock()
	if state != "standby" {
		t.Fatalf("state = %q, want %q", state, "standby")
	}
}

// TestEvaluateIdle_RecordKeepsWaitingWithoutUI は、画面を 1 つも開いていなくても
// 記録が開いている間は waiting が続くことを確認する。以前の 15 秒のリース
// （画面の再申告が止まると保留中を下ろす）より長く出力が止まっていても下ろさない。
func TestEvaluateIdle_RecordKeepsWaitingWithoutUI(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 16, "claude")
	ses.State = "waiting"
	ses.pendingApproval = testNativeRecord("native-sig", "", 0)
	ses.lastOutputAt = time.Now().Add(-time.Minute)

	for i := 0; i < 3; i++ {
		s.evaluateIdle()
	}

	s.sessionsMu.Lock()
	state := s.sessions[16].State
	activity := s.sessions[16].Activity
	s.sessionsMu.Unlock()
	if state != "waiting" || !activity.AwaitingApproval || !activity.AwaitingUser {
		t.Fatalf("state = %q activity = %+v, want waiting with the approval flags", state, activity)
	}
}

// TestEvaluateIdle_NoChangeWhenRunningRecent は lastOutputAt が直近なら
// running のまま変化しないことを確認する。
func TestEvaluateIdle_NoChangeWhenRunningRecent(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 13, "claude")
	ses.lastOutputAt = time.Now() // 直近

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[13].State
	s.sessionsMu.Unlock()
	if state != "running" {
		t.Fatalf("state = %q, want %q (no change expected)", state, "running")
	}
}

// TestExtractBannerModel は起動バナーのレンダリング済み行からの
// モデル名 / effort 抽出（Claude / Codex / 非対象 provider）を確認する。
func TestExtractBannerModel(t *testing.T) {
	tests := []struct {
		name       string
		provider   string
		cwd        string
		lines      []string
		want       string
		wantEffort string
	}{
		{
			name:     "claude: effort・プラン付きバナー",
			provider: "claude",
			lines: []string{
				"▐▛███▜▌ Claude Code v2.1.162",
				"▝▜█████▛▘  Opus 4.8 (1M context) with medium effort · Claude Max",
				"  ▘▘ ▝▝  C:\\dev\\many-ai-cli",
			},
			want:       "Opus 4.8 (1M context)",
			wantEffort: "medium",
		},
		{
			// v2.1.246 でロゴのアスキーアートが変わった（▝▜█████▛▘ → ▝▜██████▀）。
			// ロゴ文字列を足場にしていた実装は、この形で検出できなくなった。
			name:     "claude: 新ロゴ（v2.1.246）",
			provider: "claude",
			lines: []string{
				" ▐▛███▛█   Claude Code v2.1.246",
				"▝▜██████▀  Opus 5 with high effort · Claude Pro",
			},
			want:       "Opus 5",
			wantEffort: "high",
		},
		{
			name:     "claude: effort なしバナー",
			provider: "claude",
			lines: []string{
				"▐▛███▜▌ Claude Code v2.1.162",
				"▝▜█████▛▘  Sonnet 4.6 · Claude Pro",
			},
			want:       "Sonnet 4.6",
			wantEffort: "",
		},
		{
			name:     "claude: バージョン行なし",
			provider: "claude",
			lines:    []string{"❯ Try \"edit <filepath> to...\""},
			want:     "",
		},
		{
			name:     "codex: 通常バナー",
			provider: "codex",
			lines: []string{
				"│ >_ OpenAI Codex (v0.136.0)              │",
				"│ model:       gpt-5.5 xhigh   /model to change │",
			},
			want: "gpt-5.5 xhigh",
		},
		{
			name:     "codex: loading は除外",
			provider: "codex",
			lines:    []string{"│ model:       loading   /model to change │"},
			want:     "",
		},
		{
			name:     "copilot: ステータス行右端（effort なし）",
			provider: "copilot",
			lines: []string{
				"❯",
				" ● Working esc canceltions, 1 skill, 1 MCP server                          Claude Haiku 4.5",
			},
			want: "Claude Haiku 4.5",
		},
		{
			name:       "copilot: effort サフィックス付き",
			provider:   "copilot",
			lines:      []string{"● Loading: 3 instructions, 1 skill                              GPT-5 mini · low"},
			want:       "GPT-5 mini",
			wantEffort: "low",
		},
		{
			name:     "copilot: Auto は許可",
			provider: "copilot",
			lines:    []string{"● Working                                  Auto"},
			want:     "Auto",
		},
		{
			name:     "copilot: モデル名らしくない右端は除外",
			provider: "copilot",
			lines:    []string{"↑/↓ to navigate · tab switch tab · enter to select · esc to cancel"},
			want:     "",
		},
		{
			name:     "cursor-agent: cwd·branch 行直上",
			provider: "cursor-agent",
			cwd:      `D:\dev\many-ai-cli`,
			lines: []string{
				"  → Plan, search, build anything",
				"   Auto",
				`  D:\dev\many-ai-cli · develop`,
			},
			want: "Auto",
		},
		{
			name:     "cursor-agent: 使用率サフィックスを除去",
			provider: "cursor-agent",
			cwd:      `D:\dev\many-ai-cli`,
			lines: []string{
				"  Auto · 7.4%",
				`  D:\dev\many-ai-cli · develop`,
			},
			want: "Auto",
		},
		{
			name:     "cursor-agent: 直上がプロンプト残骸なら除外",
			provider: "cursor-agent",
			cwd:      `D:\dev\many-ai-cli`,
			lines: []string{
				"  → 今日の日時は？",
				`  D:\dev\many-ai-cli · develop`,
			},
			want: "",
		},
		{
			name:     "非対象 provider",
			provider: "ollama",
			lines:    []string{"▝▜█████▛▘  Opus 4.8 · Claude Max"},
			want:     "",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, gotEffort := extractBannerModel(tt.provider, tt.cwd, tt.lines)
			if got != tt.want {
				t.Fatalf("extractBannerModel(%q) model = %q, want %q", tt.provider, got, tt.want)
			}
			if gotEffort != tt.wantEffort {
				t.Fatalf("extractBannerModel(%q) effort = %q, want %q", tt.provider, gotEffort, tt.wantEffort)
			}
		})
	}
}

// TestApplyDetectedModel_OnlyIfEmpty は onlyIfEmpty=true のとき既存 Model を
// 上書きしないこと、空なら設定することを確認する。
func TestApplyDetectedModel_OnlyIfEmpty(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 14, "claude")

	// 空 → 設定される
	s.applyDetectedModel(14, "claude", "Opus 4.8 (1M context)", "medium", true)
	if ses.Model != "Opus 4.8 (1M context)" {
		t.Fatalf("Model = %q, want %q", ses.Model, "Opus 4.8 (1M context)")
	}
	if ses.Effort != "medium" {
		t.Fatalf("Effort = %q, want %q", ses.Effort, "medium")
	}
	if !ses.initialModelScanDone {
		t.Fatalf("initialModelScanDone = false, want true")
	}

	// 既存値あり + onlyIfEmpty=true → 上書きしない
	s.applyDetectedModel(14, "claude", "Haiku 4.5", "low", true)
	if ses.Model != "Opus 4.8 (1M context)" {
		t.Fatalf("Model = %q, onlyIfEmpty で上書きされてはならない", ses.Model)
	}
	if ses.Effort != "medium" {
		t.Fatalf("Effort = %q, onlyIfEmpty で上書きされてはならない", ses.Effort)
	}

	// 既存値あり + onlyIfEmpty=false（/model 変更経路）→ 上書きする
	s.applyDetectedModel(14, "claude", "Haiku 4.5", "", false)
	if ses.Model != "Haiku 4.5" {
		t.Fatalf("Model = %q, want %q", ses.Model, "Haiku 4.5")
	}
	// effort は空文字で消さない（検出できなかっただけかもしれないため）
	if ses.Effort != "medium" {
		t.Fatalf("Effort = %q, 空文字で消してはならない", ses.Effort)
	}

	// モデルが同じでも effort だけ変われば反映する
	s.applyDetectedModel(14, "claude", "Haiku 4.5", "high", false)
	if ses.Effort != "high" {
		t.Fatalf("Effort = %q, want %q", ses.Effort, "high")
	}
}

// TestFinalizeTranscript_EmptyPath は jsonlPath が空のとき何もしないことを確認する。
func TestFinalizeTranscript_EmptyPath(t *testing.T) {
	s := newTestServer()
	// パニックや error が起きないことだけ確認
	s.finalizeTranscript(1, "")
}

// TestFinalizeTranscript_CreatesTranscript は有効な JSONL から
// transcript ファイルが生成されることを確認する。
func TestFinalizeTranscript_CreatesTranscript(t *testing.T) {
	tmp := t.TempDir()
	// 最小限の JSONL（session_end イベントを含む）
	jsonlPath := filepath.Join(tmp, "session.jsonl")
	content := `{"ts":"2026-01-01T00:00:00Z","type":"session_end","session_id":1,"state":"completed","exit_code":0}` + "\n"
	if err := os.WriteFile(jsonlPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.finalizeTranscript(1, jsonlPath)

	// transcript が生成されていること（WriteTranscriptFile が決めるパスを確認）
	// sessionlog.TranscriptPath の実装に依存するが、*.txt が生成されるはず
	entries, err := os.ReadDir(tmp)
	if err != nil {
		t.Fatal(err)
	}
	var found bool
	for _, e := range entries {
		if filepath.Ext(e.Name()) == ".txt" {
			found = true
		}
	}
	if !found {
		t.Error("transcript .txt file not found in tmp dir")
	}
}
