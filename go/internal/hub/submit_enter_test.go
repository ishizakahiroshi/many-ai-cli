package hub

// submit_enter_test.go: 確定 \r の送出タイミング（C1）と、確定できたかの検知・再送（C2）。
// plan_hub-submit-enter-not-confirmed.md
//
// 固定 50ms 遅延は「どれだけ待てば十分か」に答えを持たないため、出力静止待ちへ置き換えた。
// 時間で待つ設計に答えが無いことは変わらないので、最後の保証は「送った結果どうなったか」を
// 見る C2 側にある。ここではその両方を実時間に依存しない短い timing で固定する。

import (
	"io"
	"log/slog"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

func submitEnterTestTiming() submitEnterTiming {
	return submitEnterTiming{
		idleSettle:    10 * time.Millisecond,
		minWait:       10 * time.Millisecond,
		slowMinWait:   40 * time.Millisecond,
		maxWait:       200 * time.Millisecond,
		poll:          2 * time.Millisecond,
		confirmWindow: 30 * time.Millisecond,
	}
}

func submitEnterServer(t *testing.T, sessionID int, provider string) *Server {
	t.Helper()
	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = 47777
	s := &Server{
		cfg:          cfg,
		logger:       slog.New(slog.NewTextHandler(io.Discard, nil)),
		sessions:     map[int]*session{},
		wrappers:     map[int]*wrapperConn{},
		pendingInput: map[int][]string{},
		submitEnter:  submitEnterTestTiming(),
	}
	s.sessionsMu.Lock()
	s.sessions[sessionID] = &session{ID: sessionID, Provider: provider, State: "standby", inputMu: new(sync.Mutex)}
	s.sessionsMu.Unlock()
	return s
}

// setTestLastOutput は PTY 出力を 1 回受けたのと同じ状態にする（markRunning の
// 副作用である broadcast / sessionStore 更新を持ち込まずに lastOutputAt だけ動かす）。
func (s *Server) setTestLastOutput(sessionID int, at time.Time) {
	s.sessionsMu.Lock()
	if ses := s.sessions[sessionID]; ses != nil {
		ses.lastOutputAt = at
	}
	s.sessionsMu.Unlock()
}

// streamTestOutput は stop が閉じるまで PTY 出力が続いている状態を作る。
// 戻り値を呼ぶと goroutine の終了まで待つ。
func streamTestOutput(s *Server, sessionID int, stop <-chan struct{}) func() {
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-stop:
				return
			default:
			}
			s.setTestLastOutput(sessionID, time.Now())
			time.Sleep(2 * time.Millisecond)
		}
	}()
	return func() { <-done }
}

type capturedFrames struct {
	mu     sync.Mutex
	frames []string
}

func (c *capturedFrames) add(data string) {
	c.mu.Lock()
	c.frames = append(c.frames, data)
	c.mu.Unlock()
}

func (c *capturedFrames) snapshot() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]string, len(c.frames))
	copy(out, c.frames)
	return out
}

func (c *capturedFrames) count(data string) int {
	n := 0
	for _, f := range c.snapshot() {
		if f == data {
			n++
		}
	}
	return n
}

// TestSubmitEnterMinWaitIsLongerForSlowPasteProviders は provider 別の最低待機を固定する。
// codex / opencode は大きいペーストをプレースホルダへほぼ無出力で畳み込むため、出力静止が
// 瞬時に成立して早撃ちになる。web/src/app/deferred-enter.ts の実測値 700ms に合わせる。
func TestSubmitEnterMinWaitIsLongerForSlowPasteProviders(t *testing.T) {
	timing := submitEnterTiming{}.resolved()
	for _, provider := range []string{"codex", "opencode"} {
		if got := timing.minWaitFor(provider); got != submitEnterSlowMinWait {
			t.Errorf("minWaitFor(%q) = %v, want %v", provider, got, submitEnterSlowMinWait)
		}
	}
	for _, provider := range []string{"claude", "grok", "copilot", ""} {
		if got := timing.minWaitFor(provider); got != submitEnterMinWait {
			t.Errorf("minWaitFor(%q) = %v, want %v", provider, got, submitEnterMinWait)
		}
	}
	if submitEnterSlowMinWait < 700*time.Millisecond {
		t.Errorf("submitEnterSlowMinWait = %v, want >= 700ms (web 側の実測値)", submitEnterSlowMinWait)
	}
}

// TestSubmitEnterSettleWaitsForOutputToStop は「固定時間ではなく出力静止を待つ」ことを固定する。
func TestSubmitEnterSettleWaitsForOutputToStop(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	stop := make(chan struct{})
	wait := streamTestOutput(s, sessionID, stop)
	time.AfterFunc(60*time.Millisecond, func() { close(stop) })

	waited := s.waitForSubmitEnterSettle(sessionID)
	wait()

	if waited < 60*time.Millisecond {
		t.Fatalf("settle returned after %v; 出力が続いている間は撃ってはいけない", waited)
	}
	if waited >= submitEnterTestTiming().maxWait {
		t.Fatalf("settle waited %v = maxWait まで粘った; 静止検知が働いていない", waited)
	}
}

// TestSubmitEnterSettleHonorsProviderMinWait は、出力が一度も無く静止が即成立する
// セッションでも provider 別の最低待機を守ることを固定する（早撃ち防止の本体）。
func TestSubmitEnterSettleHonorsProviderMinWait(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "codex")
	waited := s.waitForSubmitEnterSettle(sessionID)
	if min := submitEnterTestTiming().slowMinWait; waited < min {
		t.Fatalf("settle waited %v, want >= %v (codex の最低待機)", waited, min)
	}
}

// TestSubmitEnterSettleGivesUpAtMaxWait は、出力が永久に止まらない病的ケースでも
// 必ず送出することを固定する（保険が無いと確定 \r が一生撃たれない）。
//
// 「止まらない出力」は goroutine で lastOutputAt を刻み続ける形にしない。2ms 周期の
// goroutine が idleSettle（10ms）以上スケジュールから外れると静止判定が先に成立し、
// maxWait に届く前に返ってしまう（2026-09-07 CI macOS で 24.8ms、ローカルでも 50 回中
// 1 回）。outputQuietFor は time.Since(lastOutputAt) >= quiet で判定するので、
// lastOutputAt を未来に置けば経過時間は負のまま静止にならず、maxWait で諦める経路
// だけを決定的に踏む。
func TestSubmitEnterSettleGivesUpAtMaxWait(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	s.setTestLastOutput(sessionID, time.Now().Add(time.Hour))

	waited := s.waitForSubmitEnterSettle(sessionID)

	maxWait := submitEnterTestTiming().maxWait
	if waited < maxWait {
		t.Fatalf("settle waited %v, want >= maxWait %v", waited, maxWait)
	}
	if waited > 3*maxWait {
		t.Fatalf("settle waited %v, want to give up near maxWait %v", waited, maxWait)
	}
}

// TestSubmitEnterResendsOnceWhenNotConfirmed は、確定 \r の後に PTY 出力が
// 1 バイトも来なければ未確定とみなして再送すること、かつ再送は 1 回だけであることを固定する。
// 2 回以上撃つと二重確定になり、後続プロンプトを誤承認する。
func TestSubmitEnterResendsOnceWhenNotConfirmed(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	frames := &capturedFrames{}
	wc := &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			frames.add(string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Lock()
	s.wrappers[sessionID] = wc
	s.sessionsMu.Unlock()

	body := bracketedPasteStart + "hello" + bracketedPasteEnd
	s.submitInput(sessionID, body+"\r")

	if got := frames.count("\r"); got != 2 {
		t.Fatalf("確定 \\r を %d 回送った、want 2（初回 + 再送 1 回だけ）: %q", got, frames.snapshot())
	}
	if got := frames.count(body); got != 1 {
		t.Fatalf("本文を %d 回送った、want 1（再送は \\r だけ）: %q", got, frames.snapshot())
	}
}

// TestSubmitEnterDoesNotResendWhenOutputFollows は、確定 \r の後に PTY 出力が
// 続いた（= CLI が動き出した）場合は再送しないことを固定する。
func TestSubmitEnterDoesNotResendWhenOutputFollows(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	frames := &capturedFrames{}
	wc := &wrapperConn{sendFunc: func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok || msg.Type != "pty_input" {
			return nil
		}
		data := string(msg.Data)
		frames.add(data)
		if data == "\r" {
			// CLI が確定を受け取って描画を始めた状態を模す。
			go func() {
				time.Sleep(5 * time.Millisecond)
				s.setTestLastOutput(sessionID, time.Now())
			}()
		}
		return nil
	}}
	s.sessionsMu.Lock()
	s.wrappers[sessionID] = wc
	s.sessionsMu.Unlock()

	s.submitInput(sessionID, bracketedPasteStart+"hello"+bracketedPasteEnd+"\r")

	if got := frames.count("\r"); got != 1 {
		t.Fatalf("確定 \\r を %d 回送った、want 1（確定できているので再送不要）: %q", got, frames.snapshot())
	}
}

// TestSubmitEnterConfirmDetectsOutputAfterSend は検知そのものの境界を固定する。
// since より後の出力だけを「動き出した」とみなす（送出前の残り描画で誤判定しない）。
func TestSubmitEnterConfirmDetectsOutputAfterSend(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	timing := submitEnterTestTiming()

	// 送出前の出力しか無い場合は「動き出していない」。
	s.setTestLastOutput(sessionID, time.Now())
	since := time.Now()
	if s.waitForOutputAfter(sessionID, since, timing.confirmWindow, timing.poll) {
		t.Fatalf("送出前の出力を確定の証拠として扱っている")
	}

	// 送出後に出力が来たら「動き出した」。
	since = time.Now()
	time.AfterFunc(5*time.Millisecond, func() { s.setTestLastOutput(sessionID, time.Now()) })
	if !s.waitForOutputAfter(sessionID, since, timing.confirmWindow, timing.poll) {
		t.Fatalf("送出後の出力を検知できていない")
	}
}

// TestSubmitEnterSplitAcrossMessagesStillSettles は、UI が本文と確定 CR を
// 別々の pty_input で送ってきた場合も、1 通で来た場合と同じ扱い（出力静止待ち →
// 送出 → 確認・再送）になることを固定する。
//
// これが無いと確定 CR は素通しになり、本文の配送が遅れた分だけ CR が本文へ密着して
// 内側 CLI に吸収される。実運用の UI は常にこの 2 通形式で送るため、1 通形式だけを
// 守っていても本番では一度も効かない。
func TestSubmitEnterSplitAcrossMessagesStillSettles(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	frames := &capturedFrames{}
	wc := &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			frames.add(string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Lock()
	s.wrappers[sessionID] = wc
	s.sessionsMu.Unlock()

	body := bracketedPasteStart + "hello" + bracketedPasteEnd
	s.submitInput(sessionID, body)
	// 本文の直後は出力が続いている（内側 CLI が取り込み・再描画中）状態を作る。
	stop := make(chan struct{})
	wait := streamTestOutput(s, sessionID, stop)

	go func() {
		time.Sleep(25 * time.Millisecond)
		close(stop)
	}()
	start := time.Now()
	s.submitInput(sessionID, "\r")
	elapsed := time.Since(start)
	wait()

	if elapsed < 20*time.Millisecond {
		t.Fatalf("確定 CR を %v で撃った、want 出力が止まるまで待つ（>=20ms）", elapsed)
	}
	if got := frames.count(body); got != 1 {
		t.Fatalf("本文を %d 回送った、want 1: %q", got, frames.snapshot())
	}
	if got := frames.count("\r"); got != 2 {
		t.Fatalf("確定 CR を %d 回送った、want 2（初回 + 確認できず再送 1 回）: %q", got, frames.snapshot())
	}
}

// TestSubmitEnterSplitMarkIsConsumedByOtherInput は、本文と確定 CR の間に別の入力が
// 挟まった場合に、後から来た無関係な CR を確定 CR として扱わないことを固定する。
func TestSubmitEnterSplitMarkIsConsumedByOtherInput(t *testing.T) {
	const sessionID = 1
	s := submitEnterServer(t, sessionID, "claude")
	frames := &capturedFrames{}
	wc := &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			frames.add(string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Lock()
	s.wrappers[sessionID] = wc
	s.sessionsMu.Unlock()

	s.submitInput(sessionID, bracketedPasteStart+"hello"+bracketedPasteEnd)
	s.submitInput(sessionID, "\x1b") // 本文以外の入力が印を消費する
	s.submitInput(sessionID, "\r")

	if got := frames.count("\r"); got != 1 {
		t.Fatalf("CR を %d 回送った、want 1（確定 CR 扱いしないので再送しない）: %q", got, frames.snapshot())
	}
}
