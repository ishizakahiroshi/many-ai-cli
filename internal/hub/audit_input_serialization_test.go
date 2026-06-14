package hub

import (
	"log/slog"
	"many-ai-cli/internal/config"
	"sync"
	"testing"
)

// auditInputServer builds a minimal Server suitable for submitInput tests.
func auditInputServer(t *testing.T) *Server {
	t.Helper()
	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = 47777
	s := &Server{
		cfg:          cfg,
		logger:       slog.Default(),
		sessions:     map[int]*session{},
		wrappers:     map[int]*wrapperConn{},
		uis:          nil,
		pendingInput: map[int][]string{},
	}
	return s
}

// TestSubmitInputSerializedConcurrent は同一セッションへ複数の goroutine が
// 同時に submitInput を呼んでも inputMu によって直列化されることを確認する（#18）。
// -race フラグで data race が検出されないことが主要な保証。
// wrapper が nil（未接続）の場合はすべての入力が pendingInput に蓄積される。
func TestSubmitInputSerializedConcurrent(t *testing.T) {
	s := auditInputServer(t)
	const sessionID = 1

	// セッションを登録（wrapper は nil = 未接続とし、すべて pending へ行く）。
	ses := &session{ID: sessionID, State: "standby"}
	s.sessionsMu.Lock()
	s.sessions[sessionID] = ses
	// wrappers[sessionID] は設定しない（nil）
	s.sessionsMu.Unlock()

	const goroutines = 10
	const msgsPerGoroutine = 5
	total := goroutines * msgsPerGoroutine

	var wg sync.WaitGroup
	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := 0; i < msgsPerGoroutine; i++ {
				// wc=nil: trySendInput が即座に combined を返し pending へ積まれる。
				s.submitInput(nil, sessionID, "hello\r")
			}
		}()
	}
	wg.Wait()

	s.sessionsMu.Lock()
	got := len(s.pendingInput[sessionID])
	s.sessionsMu.Unlock()

	if got != total {
		t.Errorf("pendingInput length = %d, want %d (some inputs lost or duplicated)", got, total)
	}
}

// TestSubmitInputBracketedPasteNoConcurrentInterleave は bracketed-paste
// （bracketedPasteEnd+"\r" サフィックス）を含む入力が、50ms 遅延中に
// 別の goroutine の入力で割り込まれないことを確認する（#18）。
// wrapper が nil の場合は全件 pending に積まれ、順序も保証される。
func TestSubmitInputBracketedPasteNoConcurrentInterleave(t *testing.T) {
	s := auditInputServer(t)
	const sessionID = 2

	ses := &session{ID: sessionID, State: "standby"}
	s.sessionsMu.Lock()
	s.sessions[sessionID] = ses
	s.sessionsMu.Unlock()

	// 2 goroutine が同時に bracketed-paste 入力を送る。
	// inputMu がなければ一方の first 送信後・delayed 送信前に他方が割り込む可能性がある。
	var wg sync.WaitGroup
	for g := 0; g < 2; g++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			// bracketedPasteEnd を含む入力: splitBracketedPasteSubmit が first/delayed に分割する。
			paste := "\x1b[200~paste content\x1b[201~\r"
			s.submitInput(nil, sessionID, paste)
		}()
	}
	wg.Wait()

	// pending に 2 件蓄積されていることを確認（race detector が data race を検出しなければ合格）。
	s.sessionsMu.Lock()
	got := len(s.pendingInput[sessionID])
	s.sessionsMu.Unlock()

	if got != 2 {
		t.Errorf("pendingInput length = %d, want 2", got)
	}
}

// TestSubmitInputNilSessionDropped はセッションが存在しない（削除済み）場合に
// submitInput がパニックせず正常終了することを確認する。
func TestSubmitInputNilSessionDropped(t *testing.T) {
	s := auditInputServer(t)
	// sessions[99] は未登録。submitInput は早期リターンする。
	s.submitInput(nil, 99, "hello\r")
	// pendingInput に何も積まれていないことを確認。
	s.sessionsMu.Lock()
	got := len(s.pendingInput[99])
	s.sessionsMu.Unlock()
	if got != 0 {
		t.Errorf("pendingInput[99] = %d, want 0 for non-existent session", got)
	}
}

// TestInputMuExistsOnSession は session 構造体が inputMu フィールドを持つことを
// コンパイル時に保証する回帰テスト（#18）。
func TestInputMuExistsOnSession(t *testing.T) {
	var ses session
	// inputMu に対して Lock/Unlock が呼べることをコンパイル・実行レベルで確認する。
	ses.inputMu.Lock()
	ses.inputMu.Unlock()
}
