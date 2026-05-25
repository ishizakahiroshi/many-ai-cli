package wrapper

import (
	"log/slog"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"any-ai-cli/internal/config"
	"golang.org/x/net/websocket"
)

// mockProcessSession はテスト用の processSession 実装。
// Close 呼び出しを記録する。
type mockProcessSession struct {
	closeCalled atomic.Bool
}

func (m *mockProcessSession) Read(p []byte) (int, error)    { return 0, nil }
func (m *mockProcessSession) Write(p []byte) (int, error)   { return len(p), nil }
func (m *mockProcessSession) Close() error                  { m.closeCalled.Store(true); return nil }
func (m *mockProcessSession) Wait() error                   { return nil }
func (m *mockProcessSession) Resize(cols, rows uint16) error { return nil }

// makeSupervisor はテスト用の reconnectSupervisor を組み立てるヘルパー。
func makeSupervisor(
	cfg *config.Config,
	ps *mockProcessSession,
	intentional bool,
	probeAlive bool,
	reconnectCh chan struct{},
	done chan struct{},
	closeDone func(),
) *reconnectSupervisor {
	var intentionalFlag atomic.Bool
	intentionalFlag.Store(intentional)

	ws := &wrapperSession{currentSID: 1}

	return &reconnectSupervisor{
		cfg:              cfg,
		logger:           slog.Default(),
		ws:               ws,
		ps:               ps,
		provider:         "claude",
		display:          "Claude",
		cwd:              "/tmp",
		label:            "",
		model:            "",
		startedAtText:    "",
		rawLogPath:       "",
		jsonlPath:        "",
		intentional:      &intentionalFlag,
		done:             done,
		closeDone:        closeDone,
		reconnectCh:      reconnectCh,
		startReceiveLoop: func(_ *websocket.Conn) {},
		snapshotReplay:   func() []byte { return nil },
		probeHub:         func(_ *config.Config) bool { return probeAlive },
	}
}

// TestReconnectSupervisor_IntentionalShutdown は hub_shutdown 受信後（intentional=true）、
// grace 期間が 0 以下のとき PTY を kill して done を閉じることを確認する。
func TestReconnectSupervisor_IntentionalShutdown(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.WrapperReconnectGraceSec = 0 // grace 無効

	ps := &mockProcessSession{}
	reconnectCh := make(chan struct{}, 1)
	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	sup := makeSupervisor(cfg, ps, true /* intentional */, false /* probeAlive */, reconnectCh, done, closeDone)

	reconnectCh <- struct{}{}

	go sup.run()

	select {
	case <-done:
		// 期待通り done が閉じた
	case <-time.After(2 * time.Second):
		t.Fatal("timeout: reconnect supervisor did not terminate PTY on intentional shutdown with grace=0")
	}

	if !ps.closeCalled.Load() {
		t.Error("ps.Close() should have been called")
	}
}

// TestReconnectSupervisor_AutoShutdown は intentional=false かつ hub が down、
// かつ auto_shutdown=true のとき PTY を即 kill することを確認する。
func TestReconnectSupervisor_AutoShutdown(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.AutoShutdown = true
	cfg.Hub.WrapperReconnectGraceSec = 30

	ps := &mockProcessSession{}
	reconnectCh := make(chan struct{}, 1)
	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	// probeAlive=false → hub down, auto_shutdown=true → terminate immediately
	sup := makeSupervisor(cfg, ps, false /* intentional */, false /* probeAlive */, reconnectCh, done, closeDone)

	reconnectCh <- struct{}{}

	go sup.run()

	select {
	case <-done:
		// 期待通り done が閉じた
	case <-time.After(2 * time.Second):
		t.Fatal("timeout: reconnect supervisor did not terminate PTY on auto_shutdown")
	}

	if !ps.closeCalled.Load() {
		t.Error("ps.Close() should have been called")
	}
}

// TestReconnectSupervisor_HubAlive は hub が生きている場合（intentional でない WS 切断）、
// PTY を kill することを確認する（dismiss / kill-all / idle-timeout による意図的切断）。
func TestReconnectSupervisor_HubAlive(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.WrapperReconnectGraceSec = 30

	ps := &mockProcessSession{}
	reconnectCh := make(chan struct{}, 1)
	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	// probeAlive=true → hub is alive → intentional disconnect path
	sup := makeSupervisor(cfg, ps, false /* intentional */, true /* probeAlive */, reconnectCh, done, closeDone)

	reconnectCh <- struct{}{}

	go sup.run()

	select {
	case <-done:
		// 期待通り done が閉じた
	case <-time.After(2 * time.Second):
		t.Fatal("timeout: reconnect supervisor did not terminate PTY when hub is alive")
	}

	if !ps.closeCalled.Load() {
		t.Error("ps.Close() should have been called")
	}
}

// TestReconnectSupervisor_GraceDisabled は grace 期間が 0 のとき、
// PTY を即 kill することを確認する。
func TestReconnectSupervisor_GraceDisabled(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.WrapperReconnectGraceSec = 0
	cfg.Hub.AutoShutdown = false

	ps := &mockProcessSession{}
	reconnectCh := make(chan struct{}, 1)
	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	// intentional=false, probeAlive=false, auto_shutdown=false, grace=0
	// → "reconnect grace disabled — terminating PTY"
	sup := makeSupervisor(cfg, ps, false /* intentional */, false /* probeAlive */, reconnectCh, done, closeDone)

	reconnectCh <- struct{}{}

	go sup.run()

	select {
	case <-done:
		// 期待通り done が閉じた
	case <-time.After(2 * time.Second):
		t.Fatal("timeout: reconnect supervisor did not terminate PTY when grace is disabled")
	}

	if !ps.closeCalled.Load() {
		t.Error("ps.Close() should have been called")
	}
}

// TestReconnectSupervisor_DoneSignal は done チャネルが閉じられたとき、
// supervisor が正常終了することを確認する（PTY kill は呼ばない）。
func TestReconnectSupervisor_DoneSignal(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.WrapperReconnectGraceSec = 30

	ps := &mockProcessSession{}
	reconnectCh := make(chan struct{}, 1)
	done := make(chan struct{})
	closeDone := func() {} // テストでは手動で close する

	sup := makeSupervisor(cfg, ps, false, false, reconnectCh, done, closeDone)

	exited := make(chan struct{})
	go func() {
		sup.run()
		close(exited)
	}()

	// done を閉じると supervisor は次のループで return する
	close(done)

	select {
	case <-exited:
		// 期待通り
	case <-time.After(2 * time.Second):
		t.Fatal("timeout: reconnect supervisor did not exit on done signal")
	}

	// done シグナルだけで ps.Close() は呼ばれない
	if ps.closeCalled.Load() {
		t.Error("ps.Close() should NOT have been called on done signal")
	}
}

// TestWriteWithTrailingEnter_SplitsTrailingCR は末尾 \r を遅延分割して書くことを確認する。
func TestWriteWithTrailingEnter_SplitsTrailingCR(t *testing.T) {
	var writes [][]byte
	mock := &writeTracker{writeFunc: func(p []byte) (int, error) {
		cp := make([]byte, len(p))
		copy(cp, p)
		writes = append(writes, cp)
		return len(p), nil
	}}
	data := []byte("hello\r")
	writeWithTrailingEnter(mock, data, 1*time.Millisecond)
	if len(writes) != 2 {
		t.Fatalf("expected 2 writes, got %d", len(writes))
	}
	if string(writes[0]) != "hello" {
		t.Errorf("first write = %q, want %q", writes[0], "hello")
	}
	if string(writes[1]) != "\r" {
		t.Errorf("second write = %q, want %q", writes[1], "\r")
	}
}

// TestWriteWithTrailingEnter_NoSplitForSingleByte は1バイトデータはそのまま書くことを確認する。
func TestWriteWithTrailingEnter_NoSplitForSingleByte(t *testing.T) {
	var writes [][]byte
	mock := &writeTracker{writeFunc: func(p []byte) (int, error) {
		cp := make([]byte, len(p))
		copy(cp, p)
		writes = append(writes, cp)
		return len(p), nil
	}}
	data := []byte("\r")
	writeWithTrailingEnter(mock, data, 1*time.Millisecond)
	if len(writes) != 1 {
		t.Fatalf("expected 1 write, got %d", len(writes))
	}
	if string(writes[0]) != "\r" {
		t.Errorf("write = %q, want %q", writes[0], "\r")
	}
}

// TestWriteWithTrailingEnter_NoSplitForNoTrailingCR は末尾が \r でないデータはそのまま書くことを確認する。
func TestWriteWithTrailingEnter_NoSplitForNoTrailingCR(t *testing.T) {
	var writes [][]byte
	mock := &writeTracker{writeFunc: func(p []byte) (int, error) {
		cp := make([]byte, len(p))
		copy(cp, p)
		writes = append(writes, cp)
		return len(p), nil
	}}
	data := []byte("hello")
	writeWithTrailingEnter(mock, data, 1*time.Millisecond)
	if len(writes) != 1 {
		t.Fatalf("expected 1 write, got %d", len(writes))
	}
	if string(writes[0]) != "hello" {
		t.Errorf("write = %q, want %q", writes[0], "hello")
	}
}

// writeTracker はテスト用の processSession 実装（Write 呼び出しを記録）。
type writeTracker struct {
	writeFunc func(p []byte) (int, error)
}

func (w *writeTracker) Write(p []byte) (int, error)   { return w.writeFunc(p) }
func (w *writeTracker) Read(p []byte) (int, error)    { return 0, nil }
func (w *writeTracker) Close() error                  { return nil }
func (w *writeTracker) Wait() error                   { return nil }
func (w *writeTracker) Resize(cols, rows uint16) error { return nil }
