package hub

import (
	"log/slog"
	"os"
	"path/filepath"
	"testing"
	"time"

	"any-ai-cli/internal/config"
	"any-ai-cli/internal/proto"
	"golang.org/x/net/websocket"
)

// newTestServer は最小構成の Server を返す。UI WebSocket が 0 件なので
// broadcast は no-op となり、state machine 単体テストに使用できる。
func newTestServer() *Server {
	cfg := &config.Config{}
	cfg.Hub.Port = 47777
	return &Server{
		cfg:                 cfg,
		logger:              slog.Default(),
		sessions:            map[int]*session{},
		wrappers:            map[int]*websocket.Conn{},
		uis:                 map[*websocket.Conn]*uiConn{},
		slashCmdCache:       map[string]*slashCmdCacheEntry{},
		approvalRuleTargets: map[string]approvalRuleTarget{},
		usageLinkCache:      newUsageLinkCache(),
		modelsCache:         &modelsCache{},
		modelsRemoteCache:   newModelsRemoteCache(),
	}
}

// registerTestSession はテスト用セッションを Server に登録する。
func registerTestSession(s *Server, id int, provider string) *session {
	ses := &session{
		ID:       id,
		Provider: provider,
		State:    "running",
	}
	s.sessionsMu.Lock()
	s.sessions[id] = ses
	s.sessionsMu.Unlock()
	return ses
}

// TestHandleNativeApprovalDetection_NewApproval は新しい承認が検出されたとき
// nativeApprovalSig がセットされることを確認する。
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
	got := s.sessions[1].nativeApprovalSig
	s.sessionsMu.Unlock()
	if got != "sig-abc" {
		t.Fatalf("nativeApprovalSig = %q, want %q", got, "sig-abc")
	}
}

// TestHandleNativeApprovalDetection_SameSigNoRepeat は同一 sig の重複送信で
// nativeApprovalSig が変わらないことを確認する。
func TestHandleNativeApprovalDetection_SameSigNoRepeat(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 2, "claude")
	ses.nativeApprovalSig = "sig-dup"

	// 同じ sig で再度呼ぶ → broadcast されない（sig 変わらず）
	approval := &nativeApproval{Sig: "sig-dup", Kind: "native"}
	s.handleNativeApprovalDetection(2, approval)

	s.sessionsMu.Lock()
	got := s.sessions[2].nativeApprovalSig
	s.sessionsMu.Unlock()
	if got != "sig-dup" {
		t.Fatalf("nativeApprovalSig = %q, want %q", got, "sig-dup")
	}
}

// TestHandleNativeApprovalDetection_ClearOnNil は承認が nil のとき sig がクリアされることを確認する。
func TestHandleNativeApprovalDetection_ClearOnNil(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 3, "claude")
	ses.nativeApprovalSig = "sig-to-clear"

	s.handleNativeApprovalDetection(3, nil)

	s.sessionsMu.Lock()
	got := s.sessions[3].nativeApprovalSig
	s.sessionsMu.Unlock()
	if got != "" {
		t.Fatalf("nativeApprovalSig = %q after clear, want empty", got)
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
	got := s.sessions[4].nativeApprovalSig
	s.sessionsMu.Unlock()
	// TTL 内なので sig がセットされていないこと
	if got != "" {
		t.Fatalf("nativeApprovalSig = %q, want empty (TTL suppression)", got)
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
	got := s.sessions[5].nativeApprovalSig
	s.sessionsMu.Unlock()
	if got != "" {
		t.Fatalf("nativeApprovalSig = %q, want empty (debounce skip)", got)
	}
}

// TestMarkNativeApprovalConsumed は consumed マークが正しくセットされ、
// approval_cleared broadcast 条件（sig 一致）が満たされることを確認する。
func TestMarkNativeApprovalConsumed(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 6, "claude")
	ses.nativeApprovalSig = "sig-to-consume"

	s.markNativeApprovalConsumed(proto.Message{
		SessionID:   6,
		ApprovalSig: "sig-to-consume",
	})

	s.sessionsMu.Lock()
	consumed := s.sessions[6].nativeApprovalConsumed
	sig := s.sessions[6].nativeApprovalSig
	s.sessionsMu.Unlock()
	if consumed != "sig-to-consume" {
		t.Fatalf("nativeApprovalConsumed = %q, want %q", consumed, "sig-to-consume")
	}
	// sig がクリアされていること
	if sig != "" {
		t.Fatalf("nativeApprovalSig = %q, want empty after consume", sig)
	}
}

// TestMarkNativeApprovalConsumed_SigMismatch は sig 不一致の場合に
// nativeApprovalSig がクリアされないことを確認する。
func TestMarkNativeApprovalConsumed_SigMismatch(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 7, "claude")
	ses.nativeApprovalSig = "sig-A"

	s.markNativeApprovalConsumed(proto.Message{
		SessionID:   7,
		ApprovalSig: "sig-B", // 別の sig
	})

	s.sessionsMu.Lock()
	sig := s.sessions[7].nativeApprovalSig
	s.sessionsMu.Unlock()
	if sig != "sig-A" {
		t.Fatalf("nativeApprovalSig = %q, want %q (mismatch should not clear)", sig, "sig-A")
	}
}

// TestEvaluateIdle_RunningToWaiting は running セッションが idleAfter 経過後に
// approvalVisible=true なら waiting に遷移することを確認する。
func TestEvaluateIdle_RunningToWaiting(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 10, "claude")
	ses.approvalVisible = true
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
// approvalVisible=false なら standby に遷移することを確認する。
func TestEvaluateIdle_RunningToStandby(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 11, "claude")
	ses.approvalVisible = false
	ses.lastOutputAt = time.Now().Add(-(idleAfter + time.Millisecond))

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[11].State
	s.sessionsMu.Unlock()
	if state != "standby" {
		t.Fatalf("state = %q, want %q", state, "standby")
	}
}

// TestEvaluateIdle_WaitingToStandby は waiting セッションで approvalVisible=false になると
// standby に遷移することを確認する（session_hint フリップ追従）。
func TestEvaluateIdle_WaitingToStandby(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 12, "claude")
	ses.State = "waiting"
	ses.approvalVisible = false // UI が approval を非表示にした

	s.evaluateIdle()

	s.sessionsMu.Lock()
	state := s.sessions[12].State
	s.sessionsMu.Unlock()
	if state != "standby" {
		t.Fatalf("state = %q, want %q", state, "standby")
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
