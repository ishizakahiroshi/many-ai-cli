package wrapper

import (
	"bytes"
	"encoding/base64"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"golang.org/x/net/websocket"
	"golang.org/x/term"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/hubruntime"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/subscription"
)

const (
	reconnectDialInterval          = 2 * time.Second
	replayBufferLimit              = 64 * 1024 // bytes of recent PTY output to replay on reconnect
	hubProbeTimeout                = 1 * time.Second
	hubStartupTimeout              = 10 * time.Second
	hubStartupPoll                 = 100 * time.Millisecond
	processCloseGrace              = 2 * time.Second
	ptyInputChunkBytes             = 1024
	ptyInputChunkDelay             = 3 * time.Millisecond
	defaultWrapperSendWriteTimeout = 5 * time.Second
	// 実測では Codex の PTY 出力が 100ms 窓で最大 582 チャンクに達し、
	// 64 チャンクのキューを日常的に超えていた。ピークの約 1.76 倍を確保する。
	ptyOutputQueueCapacity = 1024
	// pty_data 1 通が無制限に膨らまないよう、待ち行列の結合上限を設ける。
	ptyOutputCoalesceMaxBytes = 256 * 1024
	// postReattachGuard は reattach 成功直後の猶予窓。この間に再度 WS が切断
	// しても、probe(hub) が生きているというだけで「意図的切断」と即断せず、
	// transport fault と同様に reconnect grace へ回す。理由は
	// bugfix_codex-reattach-eof-misclassified-intentional-kill_2026-07-20.md
	// を参照: reattach 直後の素の EOF は transportBroken を経由しないため、
	// この窓が無いと queue-full 由来の transport fault を修正しても即死経路が残る。
	postReattachGuard = 10 * time.Second
)

// reconnectGrace returns the wrapper's grace period before giving up after Hub crash.
// 0 disables reconnect entirely (legacy "kill immediately" behavior).
func reconnectGrace(cfg *config.Config) time.Duration {
	sec := cfg.Hub.WrapperReconnectGraceSec
	if sec < 0 {
		sec = 0
	}
	return time.Duration(sec) * time.Second
}

func wrapperSendWriteTimeout(cfg *config.Config) time.Duration {
	if cfg == nil || cfg.Hub.WrapperSendWriteTimeoutSec <= 0 {
		return defaultWrapperSendWriteTimeout
	}
	return time.Duration(cfg.Hub.WrapperSendWriteTimeoutSec) * time.Second
}

// wrapperSession は接続状態（現在の WS conn / session ID）を管理する。
// reconnect 時に conn と sid が入れ替わるため、stateMu で保護する。
type wrapperSession struct {
	stateMu          sync.Mutex
	sendMu           sync.Mutex // websocket.JSON.Send の直列化
	currentConn      *websocket.Conn
	currentSID       int
	sendWriteTimeout time.Duration
	// maxProcessedInputSeq は PTY へ書き終えた pty_input の最大シーケンス番号。
	// Hub は未 ack の入力を「元の seq のまま」再送するため、ここ以下の seq が
	// 来たら「既に書いた分の再送」と判定して PTY へ二重に書かず ack だけ返す。
	// これが無いと、PTY 書き込み後・ack 到達前に WS が切れた場合（abortCurrentConn は
	// sendMu を待たずに閉じるため ack は失われうる）、再送で確定 \r が 2 回入り
	// 後続プロンプトを誤承認する。接続を跨いで保持する必要があるため conn ではなく
	// セッションに持つ。
	maxProcessedInputSeq int64
}

// inputSeqAlreadyProcessed は seq が「PTY へ書き終えた分の再送」かを返す。
// Hub の採番は単調増加で、再送時も元の seq を保つため、最大値との比較で足りる。
// 限界: 書き込みに失敗した seq を飛ばして後続が成功すると、その失敗分の再送も
// 「処理済み」と見なす。PTY 書き込みが失敗するのは PTY 自体が壊れている場合に
// 限られ、そのときはセッションごと終了するため実害を許容する。
func (ws *wrapperSession) inputSeqAlreadyProcessed(seq int64) bool {
	if seq <= 0 {
		return false
	}
	ws.stateMu.Lock()
	defer ws.stateMu.Unlock()
	return seq <= ws.maxProcessedInputSeq
}

// markInputSeqProcessed は PTY へ書き終えた seq を記録する。書き込みが成功した
// 場合にだけ呼ぶこと（失敗分を記録すると Hub の再送が握り潰される）。
func (ws *wrapperSession) markInputSeqProcessed(seq int64) {
	if seq <= 0 {
		return
	}
	ws.stateMu.Lock()
	defer ws.stateMu.Unlock()
	if seq > ws.maxProcessedInputSeq {
		ws.maxProcessedInputSeq = seq
	}
}

func newWrapperSession(conn *websocket.Conn, sid int, sendWriteTimeout time.Duration) *wrapperSession {
	if sendWriteTimeout <= 0 {
		sendWriteTimeout = defaultWrapperSendWriteTimeout
	}
	return &wrapperSession{
		currentConn:      conn,
		currentSID:       sid,
		sendWriteTimeout: sendWriteTimeout,
	}
}

func (ws *wrapperSession) getSID() int {
	ws.stateMu.Lock()
	defer ws.stateMu.Unlock()
	return ws.currentSID
}

// sendMsg は現在の conn にメッセージを送信する。
// 複数 goroutine から呼んでも sendMu で直列化される。
// 書き込みが詰まった場合に PTY 読み出し側まで巻き込まないよう、
// 呼び出し側はエラーを transport fault として扱う。
func (ws *wrapperSession) sendMsg(m proto.Message) error {
	ws.sendMu.Lock()
	defer ws.sendMu.Unlock()

	ws.stateMu.Lock()
	c := ws.currentConn
	ws.stateMu.Unlock()
	if c == nil {
		return fmt.Errorf("wrapper websocket is not connected")
	}
	timeout := ws.sendWriteTimeout
	if timeout <= 0 {
		timeout = defaultWrapperSendWriteTimeout
	}
	if err := c.SetWriteDeadline(time.Now().Add(timeout)); err != nil {
		return fmt.Errorf("set wrapper websocket write deadline: %w", err)
	}
	return websocket.JSON.Send(c, m)
}

func (ws *wrapperSession) swapConn(c *websocket.Conn, sid int) {
	ws.sendMu.Lock()
	defer ws.sendMu.Unlock()

	var old *websocket.Conn
	ws.stateMu.Lock()
	if ws.currentConn != nil && ws.currentConn != c {
		old = ws.currentConn
	}
	ws.currentConn = c
	if sid > 0 {
		ws.currentSID = sid
	}
	ws.stateMu.Unlock()
	if old != nil {
		_ = old.Close()
	}
}

func (ws *wrapperSession) clearConn(broken *websocket.Conn) {
	ws.closeCurrentConn(broken)
}

func (ws *wrapperSession) isCurrentConn(c *websocket.Conn) bool {
	ws.stateMu.Lock()
	defer ws.stateMu.Unlock()
	return c != nil && ws.currentConn == c
}

// abortCurrentConn closes the current connection without waiting for sendMu.
// It is used by the bounded PTY output queue when a writer may be blocked in
// websocket I/O. Closing the underlying connection interrupts that write;
// the normal receive loop then observes the close and clears the state.
func (ws *wrapperSession) abortCurrentConn() {
	ws.stateMu.Lock()
	c := ws.currentConn
	ws.currentConn = nil
	ws.stateMu.Unlock()
	if c != nil {
		_ = c.Close()
	}
}

func (ws *wrapperSession) closeCurrentConn(c *websocket.Conn) {
	if c == nil {
		return
	}
	ws.sendMu.Lock()
	defer ws.sendMu.Unlock()

	var closeConn *websocket.Conn
	ws.stateMu.Lock()
	if ws.currentConn == c {
		closeConn = ws.currentConn
		ws.currentConn = nil
	}
	ws.stateMu.Unlock()
	if closeConn != nil {
		_ = closeConn.Close()
	}
}

func (ws *wrapperSession) closeAnyConn() {
	ws.sendMu.Lock()
	defer ws.sendMu.Unlock()

	var old *websocket.Conn
	ws.stateMu.Lock()
	old = ws.currentConn
	ws.currentConn = nil
	ws.stateMu.Unlock()
	if old != nil {
		_ = old.Close()
	}
}

// reconnectSupervisor は Hub WS 切断後の再接続ロジックを担う。
// intentional / auto_shutdown / grace の 3 分岐を管理する。
type reconnectSupervisor struct {
	cfg              *config.Config
	logger           *slog.Logger
	ws               *wrapperSession
	ps               processSession
	provider         string
	display          string
	cwd              string
	label            string
	model            string
	agentSessionID   string
	startedAtText    string
	rawLogPath       string
	jsonlPath        string
	intentional      *atomic.Bool
	done             <-chan struct{}
	closeDone        func()
	reconnectCh      <-chan struct{}
	startReceiveLoop func(c *websocket.Conn)
	// snapshotReplay は replay バッファと PTY 読み出し累計バイト数を返す。
	snapshotReplay  func() ([]byte, int64)
	transportBroken *atomic.Bool
	resumeOutput    func()
	// lastReattachAt は直近の reattach 成功時刻（UnixNano）。0 は未 reattach。
	// postReattachGuard 以内の再切断は即死させず reconnect grace へ回す。
	lastReattachAt *atomic.Int64
	// probeHub はテスト時に差し替えられる。nil の場合は probeHubAlive を使う。
	probeHub func(cfg *config.Config) bool
}

// run は reconnect supervisor のメインループ。goroutine として起動する。
func (r *reconnectSupervisor) run() {
	probe := r.probeHub
	if probe == nil {
		probe = probeHubAlive
	}
	for {
		select {
		case <-r.done:
			return
		case <-r.reconnectCh:
		}
		intentional := r.intentional.Load()
		transportBroken := r.transportBroken != nil && r.transportBroken.Load()
		recentlyReattached := false
		if r.lastReattachAt != nil {
			if last := r.lastReattachAt.Load(); last != 0 {
				recentlyReattached = time.Since(time.Unix(0, last)) < postReattachGuard
			}
		}
		if !intentional && !transportBroken && !recentlyReattached && probe(r.cfg) {
			r.logger.Info("hub still alive after WS close — treating as intentional disconnect", "session_id", r.ws.getSID())
			_ = r.ps.Close()
			r.closeDone()
			return
		}
		// A transport fault is local to the wrapper's PTY output path. It does
		// not prove that the Hub stopped, so it must use the reconnect grace
		// path even when auto_shutdown is enabled. Otherwise a transient full
		// output queue or write deadline kills the wrapped CLI and all of its
		// child agents while the Hub is still running.
		//
		// A disconnect within postReattachGuard of a successful reattach gets
		// the same benefit of the doubt: a plain WS EOF here does not go
		// through transportFailure() (it never touched the PTY output queue),
		// but observed behavior shows it is the same kind of transient
		// flakiness, not a fresh dismiss/kill-all click. See
		// bugfix_codex-reattach-eof-misclassified-intentional-kill_2026-07-20.md.
		if !intentional && !transportBroken && !recentlyReattached && r.cfg.Hub.AutoShutdown {
			r.logger.Info("hub is down and auto_shutdown is enabled; terminating PTY", "session_id", r.ws.getSID())
			_ = r.ps.Close()
			r.closeDone()
			return
		}
		grace := reconnectGrace(r.cfg)
		if grace <= 0 {
			r.logger.Info("reconnect grace disabled — terminating PTY", "session_id", r.ws.getSID())
			_ = r.ps.Close()
			r.closeDone()
			return
		}
		if intentional {
			r.logger.Info("intentional hub shutdown — waiting for manual hub restart within grace period", "session_id", r.ws.getSID(), "grace_sec", int(grace/time.Second))
		} else if transportBroken {
			r.logger.Info("PTY output transport fault — entering reconnect grace period", "session_id", r.ws.getSID(), "grace_sec", int(grace/time.Second))
		} else if recentlyReattached {
			r.logger.Info("WS closed shortly after reattach — entering reconnect grace period", "session_id", r.ws.getSID(), "grace_sec", int(grace/time.Second))
		} else {
			r.logger.Info("hub appears down — entering reconnect grace period", "session_id", r.ws.getSID(), "grace_sec", int(grace/time.Second))
		}
		deadline := time.Now().Add(grace)
		reconnected := false
		for !reconnected {
			if time.Now().After(deadline) {
				r.logger.Info("reconnect grace expired — terminating PTY", "session_id", r.ws.getSID())
				_ = r.ps.Close()
				r.closeDone()
				return
			}
			select {
			case <-r.done:
				return
			case <-time.After(reconnectDialInterval):
			}
			cols, rows := 0, 0
			if w, h, err := term.GetSize(int(os.Stdin.Fd())); err == nil && w > 0 && h > 0 {
				cols, rows = w, h
			}
			replay, ptyBytes := r.snapshotReplay()
			newConn, newSID, err := dialAndReattach(r.cfg, r.ws.getSID(), r.provider, r.display, r.cwd, r.label, r.model, r.agentSessionID, r.startedAtText, r.rawLogPath, r.jsonlPath, cols, rows, replay, ptyBytes)
			if err != nil {
				r.logger.Debug("reconnect attempt failed", "err", err)
				continue
			}
			if newSID == 0 {
				r.logger.Warn("hub rejected reattach — terminating PTY", "session_id", r.ws.getSID())
				_ = r.ps.Close()
				r.closeDone()
				return
			}
			r.logger.Info("wrapper reconnected to hub", "old_sid", r.ws.getSID(), "new_sid", newSID)
			if r.lastReattachAt != nil {
				r.lastReattachAt.Store(time.Now().UnixNano())
			}
			r.ws.swapConn(newConn, newSID)
			// A receive loop for the old connection may have reported the same
			// close while the reattach handshake was in flight. Do not let that
			// stale signal trigger a second supervisor iteration after reattach.
			for {
				select {
				case <-r.reconnectCh:
				default:
					goto reconnectSignalsDrained
				}
			}
		reconnectSignalsDrained:
			if r.transportBroken != nil {
				r.transportBroken.Store(false)
			}
			if r.resumeOutput != nil {
				r.resumeOutput()
			}
			r.startReceiveLoop(newConn)
			r.intentional.Store(false)
			reconnected = true
		}
	}
}

// ptyOutputWriter は PTY 出力を Hub WS へ非同期送信する。
// queue が満杯になった場合はデータを捨てず、transport fault として
// 接続を切断する。切断中の出力は wrapperSession の replay buffer に残り、
// 再接続時に Hub へ再送される。
type ptyOutputWriter struct {
	ws                     *wrapperSession
	outCh                  chan []byte
	mu                     sync.Mutex
	accepting              bool
	closed                 bool
	pendingChunk           []byte
	done                   chan struct{}
	notifyTransportFailure func(ptyOutputTransportFault)
}

type ptyOutputTransportFault struct {
	Kind          string
	QueueCapacity int
	ChunkBytes    int
	Err           error
}

func newPTYOutputWriter(ws *wrapperSession, capacity int, notifyTransportFailure func(ptyOutputTransportFault)) *ptyOutputWriter {
	if capacity <= 0 {
		capacity = ptyOutputQueueCapacity
	}
	return &ptyOutputWriter{
		ws:                     ws,
		outCh:                  make(chan []byte, capacity),
		accepting:              true,
		done:                   make(chan struct{}),
		notifyTransportFailure: notifyTransportFailure,
	}
}

func (w *ptyOutputWriter) enqueue(chunk []byte) {
	if len(chunk) == 0 {
		return
	}
	w.mu.Lock()
	if !w.accepting || w.closed {
		w.mu.Unlock()
		return
	}
	select {
	case w.outCh <- chunk:
		w.mu.Unlock()
		return
	default:
		// Keep PTY reads non-blocking. Terminal byte streams cannot safely
		// lose an arbitrary middle chunk, so disconnect and let replay/reconnect
		// restore a coherent recent view.
		w.accepting = false
		w.mu.Unlock()
		w.triggerTransportFailure(ptyOutputTransportFault{
			Kind:          "pty_output_queue_full",
			QueueCapacity: cap(w.outCh),
			ChunkBytes:    len(chunk),
		})
	}
}

func (w *ptyOutputWriter) isAccepting() bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.accepting && !w.closed
}

func (w *ptyOutputWriter) failTransport(fault ptyOutputTransportFault) {
	w.mu.Lock()
	if !w.accepting || w.closed {
		w.mu.Unlock()
		return
	}
	w.accepting = false
	w.mu.Unlock()
	w.triggerTransportFailure(fault)
}

func (w *ptyOutputWriter) triggerTransportFailure(fault ptyOutputTransportFault) {
	if w.notifyTransportFailure != nil {
		go w.notifyTransportFailure(fault)
	}
}

func (w *ptyOutputWriter) run() {
	defer close(w.done)
	for {
		chunk, ok := w.nextChunk()
		if !ok {
			return
		}
		if !w.isAccepting() {
			continue
		}
		chunk = w.coalesce(chunk)
		if !w.isAccepting() {
			continue
		}
		if err := w.ws.sendMsg(proto.Message{
			Type:      "pty_data",
			SessionID: w.ws.getSID(),
			Data:      chunk,
		}); err != nil {
			w.failTransport(ptyOutputTransportFault{
				Kind:          "pty_output_send_failed",
				QueueCapacity: cap(w.outCh),
				ChunkBytes:    len(chunk),
				Err:           err,
			})
		}
	}
}

// nextChunk returns a chunk saved by coalesce before reading the channel.
// pendingChunk is protected because resume may discard it concurrently with
// the writer goroutine after a transport fault.
func (w *ptyOutputWriter) nextChunk() ([]byte, bool) {
	w.mu.Lock()
	if w.pendingChunk != nil {
		chunk := w.pendingChunk
		w.pendingChunk = nil
		w.mu.Unlock()
		return chunk, true
	}
	w.mu.Unlock()
	chunk, ok := <-w.outCh
	return chunk, ok
}

// coalesce drains chunks already waiting in outCh without waiting for a new
// chunk. The first chunk is returned as-is when no second chunk is available;
// allocation starts only when a second chunk is actually merged.
func (w *ptyOutputWriter) coalesce(chunk []byte) []byte {
	if len(chunk) == 0 {
		return chunk
	}
	if len(chunk) > ptyOutputCoalesceMaxBytes {
		w.savePendingChunk(chunk[ptyOutputCoalesceMaxBytes:])
		return chunk[:ptyOutputCoalesceMaxBytes]
	}

	merged := chunk
	for {
		select {
		case next, ok := <-w.outCh:
			if !ok {
				return merged
			}
			if len(next) == 0 {
				continue
			}
			if len(merged)+len(next) > ptyOutputCoalesceMaxBytes {
				w.savePendingChunk(next)
				return merged
			}
			if len(merged) == len(chunk) {
				merged = make([]byte, 0, len(chunk)+len(next))
				merged = append(merged, chunk...)
			}
			merged = append(merged, next...)
			if len(merged) == ptyOutputCoalesceMaxBytes {
				return merged
			}
		default:
			return merged
		}
	}
}

func (w *ptyOutputWriter) savePendingChunk(chunk []byte) {
	w.mu.Lock()
	// transport fault 後（accepting=false）は保存しない。resume が pendingChunk を
	// nil にした「後」に coalesce がここへ到達すると、fault 前の古いチャンクが
	// reattach 後に送られ、replay と二重描画になる。resume の意図
	// （stale queued chunks must not be sent a second time）をここでも守る。
	if w.accepting && !w.closed {
		w.pendingChunk = chunk
	}
	w.mu.Unlock()
}

// resume discards any chunks queued before the transport fault and resumes
// accepting fresh PTY output after a successful reattach. The replay buffer
// carries the recent output across the gap, so stale queued chunks must not be
// sent a second time after reattach.
func (w *ptyOutputWriter) resume() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.closed {
		return
	}
	w.pendingChunk = nil
	for {
		select {
		case <-w.outCh:
		default:
			w.accepting = true
			return
		}
	}
}

func (w *ptyOutputWriter) finish() {
	w.mu.Lock()
	if !w.closed {
		w.closed = true
		w.accepting = false
		close(w.outCh)
	}
	w.mu.Unlock()
	<-w.done
}

// ptyPump は PTY 出力を読み出し、enqueue へ渡し続ける。
// ストリーム終端まで読み切ったら closeDone を呼んで done を閉じる。
// 戻り値は ps.Wait() で使う waitErr。
func ptyPump(
	ps processSession,
	lf *os.File,
	rawMaxBytes int64,
	enqueue func([]byte),
	finishOutput func(),
	appendReplay func([]byte),
	closeDone func(),
) {
	var rawWritten int64
	var carry []byte
	buf := make([]byte, 4096)
	for {
		n, readErr := ps.Read(buf)
		if n > 0 {
			raw := make([]byte, n)
			copy(raw, buf[:n])
			var out []byte
			out, carry = pumpChunk(carry, raw)
			if len(out) > 0 {
				out = repairMojibakeUTF8(out)
				if lf != nil && (rawMaxBytes <= 0 || rawWritten < rawMaxBytes) {
					_, _ = lf.Write(out)
					rawWritten += int64(len(out))
				}
				appendReplay(out)
				enqueue(out)
			}
		}
		if readErr != nil {
			break
		}
	}
	// Flush any remaining carry bytes (e.g. incomplete sequence at end of stream).
	if len(carry) > 0 {
		flushed := repairMojibakeUTF8(carry)
		if lf != nil && (rawMaxBytes <= 0 || rawWritten < rawMaxBytes) {
			_, _ = lf.Write(flushed)
		}
		appendReplay(flushed)
		enqueue(flushed)
	}
	finishOutput()
	closeDone()
}

// writeWithTrailingEnter は data を PTY へ書き込む。
// ConPTY の制約として、text+\r を1チャンクで書くと \r が Enter でなく改行として
// 処理される場合があるため、末尾の \r を delay 分だけ遅延させて別チャンクで送る。
// data が空、または末尾が \r でない場合はそのまま書き込む。
func logPTYWriteError(logger *slog.Logger, sessionID int, op string, err error) {
	if err != nil && logger != nil {
		logger.Warn("pty write failed", "session_id", sessionID, "op", op, "err", err)
	}
}

func writePTY(ps processSession, data []byte) error {
	if len(data) == 0 {
		return nil
	}
	for len(data) > 0 {
		n, err := ps.Write(data)
		if err != nil {
			return err
		}
		if n <= 0 {
			return io.ErrShortWrite
		}
		data = data[n:]
	}
	return nil
}

func writePTYChunked(ps processSession, data []byte) error {
	if len(data) <= ptyInputChunkBytes {
		return writePTY(ps, data)
	}
	for len(data) > 0 {
		n := nextPTYInputChunkLen(data, ptyInputChunkBytes)
		if err := writePTY(ps, data[:n]); err != nil {
			return err
		}
		data = data[n:]
		if len(data) > 0 {
			time.Sleep(ptyInputChunkDelay)
		}
	}
	return nil
}

func nextPTYInputChunkLen(data []byte, limit int) int {
	if len(data) <= limit {
		return len(data)
	}
	n := limit
	for n > 0 && !utf8.RuneStart(data[n]) {
		n--
	}
	if n == 0 {
		return limit
	}
	return n
}

// clearPrefixSplitDelay は先頭 Ctrl+U を単独書き込みした後、本文を書くまでの待ち時間。
// 別チャンク化さえされていれば間隔ほぼ 0ms でも取り込まれることを実測済みだが、
// パイプ内での結合を避けるため trailingEnterDelay の既定と同じ 20ms を置く。
const clearPrefixSplitDelay = 20 * time.Millisecond

// splitLeadingClearControl は入力先頭の行クリア用 Ctrl+U(0x15) を本文と別書き込みに
// 分離すべきケースを判定し、分離する場合は (先頭 1 バイト, 残り) を返す（不要なら lead=nil）。
// cursor-agent の TUI（v2026.07.01 で実測）は「制御文字＋本文」が同一チャンクで届くと
// チャンク全体を入力として捨てるため、UI が前置する Ctrl+U を単独チャンクで先行送信する。
// claude / codex 等は同一チャンクでも取り込めるため対象外（挙動不変）。
func splitLeadingClearControl(provider string, data []byte) (lead []byte, rest []byte) {
	if provider == "cursor-agent" && len(data) > 1 && data[0] == 0x15 {
		return data[:1], data[1:]
	}
	return nil, data
}

func writeWithTrailingEnter(ps processSession, data []byte, delay time.Duration) error {
	if len(data) > 0 && data[len(data)-1] == '\r' {
		if err := writePTYChunked(ps, data[:len(data)-1]); err != nil {
			return err
		}
		time.Sleep(delay)
		return writePTY(ps, data[len(data)-1:])
	}
	return writePTYChunked(ps, data)
}

// shouldSplitTrailingEnter は通常の入力経路を変えずに、末尾 Enter を分離する
// 必要がある入力だけを判定する。OpenCode の承認 1 番は \r 単独で届くため、
// text+Enter と同じ ConPTY 再描画待ちへ入れる。その他の単独キー入力は従来どおり
// 即時書き込みし、複数バイトの末尾 Enter は既存の分離挙動を維持する。
func shouldSplitTrailingEnter(provider string, data []byte) bool {
	if len(data) == 0 || data[len(data)-1] != '\r' {
		return false
	}
	return len(data) > 1 || provider == "opencode"
}

func trailingEnterDelay(provider string) time.Duration {
	switch provider {
	case "codex", "opencode":
		return 180 * time.Millisecond
	default:
		return 20 * time.Millisecond
	}
}

func openCodePermissionArgs(permissionMode string) []string {
	if permissionMode == "bypassPermissions" {
		return []string{"--auto"}
	}
	return nil
}

func Run(cfg *config.Config, logger *slog.Logger, provider string, args []string) error {
	fs := flag.NewFlagSet("wrap", flag.ContinueOnError)
	label := fs.String("label", "", "session label shown in UI card")
	model := fs.String("model", "", "model override")
	permissionMode := fs.String("permission-mode", "", "permission mode (claude/grok: passed through; copilot/cursor-agent/opencode: bypassPermissions maps to each CLI's full-allow option)")
	sandbox := fs.String("sandbox", "", "codex sandbox mode")
	askForApproval := fs.String("ask-for-approval", "", "codex ask-for-approval")
	codexOSS := fs.Bool("codex-oss", false, "codex: use --oss to route via local Ollama daemon")
	utf8Session := fs.Bool("utf8", false, "set UTF-8 console encoding for this session (Windows only)")
	subscriptionLogin := fs.Bool("subscription-login", false, "run the provider CLI's own login flow instead of an interactive session")
	_ = fs.Parse(args)
	providerArgs := fs.Args()

	// Subscription login セッションは「公式 CLI のログインサブコマンドを PTY で
	// 走らせるだけ」の使い捨てセッション。model / permission / settings 等の
	// 通常フラグは付けない（login サブコマンドは受け付けない）。認証そのものは
	// 公式 CLI が行い、many-ai-cli は token を一切見ない。
	loginMode := false
	if *subscriptionLogin {
		adapter, ok := subscription.AdapterFor(provider)
		if !ok {
			return fmt.Errorf("provider %q does not support subscription profiles", provider)
		}
		loginMode = true
		providerArgs = adapter.LoginArgs()
	}

	// Reconstruct provider-specific flags from wrapper-parsed flags
	var extra []string
	if !loginMode {
		if *model != "" {
			extra = append(extra, "--model", *model)
		}
		switch provider {
		case "claude", "grok":
			// grok CLI は claude 互換の --permission-mode（bypassPermissions 含む）をネイティブサポートする
			if *permissionMode != "" && *permissionMode != "default" {
				extra = append(extra, "--permission-mode", *permissionMode)
			}
		case "codex":
			if *codexOSS {
				extra = append(extra, "--oss")
			}
			if *sandbox != "" {
				extra = append(extra, "--sandbox", *sandbox)
			}
			if *askForApproval != "" {
				extra = append(extra, "--ask-for-approval", *askForApproval)
			}
		case "copilot":
			// --allow-all = --allow-all-tools --allow-all-paths --allow-all-urls の一括指定
			if *permissionMode == "bypassPermissions" {
				extra = append(extra, "--allow-all")
			}
		case "cursor-agent":
			// --force: Force allow commands unless explicitly denied
			if *permissionMode == "bypassPermissions" {
				extra = append(extra, "--force")
			}
		case "opencode":
			// --auto: auto-approve permissions that are not explicitly denied
			extra = append(extra, openCodePermissionArgs(*permissionMode)...)
		}
	}
	providerArgs = append(extra, providerArgs...)
	var agentSessionID string
	if !loginMode {
		var err error
		providerArgs, agentSessionID, err = prepareClaudeSessionArgs(provider, providerArgs)
		if err != nil {
			return err
		}
	}

	// Hub にスポーンされた場合、起動中の Hub ポートが MANY_AI_CLI_HUB_PORT で渡される。
	// config.yaml のポートより優先して使い、wrapper が別 Hub を勝手に起動するのを防ぐ。
	if portStr := os.Getenv("MANY_AI_CLI_HUB_PORT"); portStr != "" {
		if port, err2 := strconv.Atoi(portStr); err2 == nil && port > 0 {
			cfg.Hub.Port = port
		}
	}

	if err := ensureHub(cfg, logger); err != nil {
		return err
	}
	cwd, _ := os.Getwd()
	display := map[string]string{"claude": "Claude", "codex": "Codex", "copilot": "GitHub Copilot", "cursor-agent": "Cursor Agent", "opencode": "OpenCode", "grok": "Grok Build", "shell": "Shell"}[provider]
	termCols, termRows := 0, 0
	if w, h, err := term.GetSize(int(os.Stdin.Fd())); err == nil && w > 0 && h > 0 {
		termCols, termRows = w, h
	}

	conn, reg, err := dialAndRegister(cfg, provider, display, cwd, *label, *model, agentSessionID, termCols, termRows)
	if err != nil {
		return err
	}
	sessionID := reg.SessionID
	setConsoleTitle(fmt.Sprintf("many-ai-cli [#%d:%s]", sessionID, provider))
	setConsoleIcon()
	initCols, initRows := reg.Cols, reg.Rows
	if initCols <= 0 || initRows <= 0 {
		initCols, initRows = 200, 50
	}

	startedAt := time.Now()
	startedAtText := reg.StartedAt
	if startedAtText != "" {
		if parsed, err := time.Parse(time.RFC3339, startedAtText); err == nil {
			startedAt = parsed
		}
	} else {
		startedAtText = startedAt.Format(time.RFC3339)
	}
	rawLogPath, jsonlPath := reg.LogPath, reg.JSONLPath
	if rawLogPath == "" || jsonlPath == "" {
		rawLogPath, jsonlPath = sessionlog.Paths(cfg.Hub.LogDir, sessionlog.Metadata{
			SessionID: sessionID,
			Provider:  provider,
			CWD:       cwd,
			StartedAt: startedAt,
		})
	}
	// セッションログが無効（既定）なら生 PTY ログ（.log）を一切作らない。
	// .log には API キー・トークン・パスワードがマスクされず残るため、
	// オプトインした利用者のみ記録する。lf == nil 時は以降の書き込みが
	// すべてスキップされる（ptyPump 側で nil チェック済み）。
	var lf *os.File
	logger.Info("session_log_gate", "session_id", sessionID, "provider", provider, "enabled", cfg.Log.SessionEnabled, "raw_log_path", rawLogPath)
	if cfg.Log.SessionEnabled {
		_ = os.MkdirAll(filepath.Dir(rawLogPath), sessionlog.PrivateDirMode)
		f, err := os.OpenFile(rawLogPath, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, sessionlog.PrivateFileMode)
		if err != nil {
			logger.Warn("session raw log create failed", "session_id", sessionID, "path", rawLogPath, "err", err)
		} else {
			lf = f
		}
	}
	if lf != nil {
		defer lf.Close()
	}

	// Claude: 共有 .claude/settings.local.json を一切触らず、wrapper 所有の temp
	// settings を `--settings` で渡して statusLine（usage-relay）を有効化する。
	// reg.TokenStatusbar が false（UI バー無効）なら付けない＝従来の挙動を維持。
	// diag 継続: docs/local/archive/v0.5.x/bugfix_statusline-settings-skip_2026-07-10.md
	// Hub 側 statusline_gate_hub と突き合わせるため、reg 受信値と reg.Type を残す。
	// 2026-07-19 時点: hub.log では token_statusbar_send=true のみ（false 0 件）で
	// Hub ゲートは原因候補から外れている。wrapper 側 reg=false や settings 書込失敗が
	// 残仮説のため、再現ログが揃うまで意図的に残置（原因確定後に外す）。
	logger.Info("statusline_gate_wrapper",
		"session_id", sessionID,
		"provider", provider,
		"reg_type", reg.Type,
		"reg_token_statusbar", reg.TokenStatusbar)
	// crossSessionInbound は orchestration の子（Auto=true）にだけ入れる。子は
	// bypassPermissions 固定・人間が張り付かない自走前提で、cross-session messaging の
	// 既定だと受信メッセージが承認ダイアログ付きで hold され dialogExpiry まで止まる
	// （usage_hooks.go: ClaudeSettingsOptions.CrossSessionInboundAccept 参照）。
	// conductor と手動セッションは人間が UI を見ているので既定のままにする。
	// ネイティブ Windows には機能自体が無いので書かない（WSL 内は GOOS=linux で対象）。
	// login セッションには --settings を渡さない。`claude auth login` は
	// インタラクティブセッション用のフラグを受け付けず、付けると起動に失敗する。
	settingsOpts := ClaudeSettingsOptions{
		StatusLine:                !loginMode && provider == "claude" && reg.TokenStatusbar,
		CrossSessionInboundAccept: !loginMode && provider == "claude" && reg.OrchestrationID != "" && reg.Auto && runtime.GOOS != "windows",
	}
	if settingsOpts.enabled() {
		exe, exeErr := os.Executable()
		if exeErr != nil {
			exe = "many-ai-cli"
		}
		hp := UsageHookParams{
			HubURL:    fmt.Sprintf("http://127.0.0.1:%d", cfg.Hub.Port),
			Token:     cfg.Token,
			SessionID: sessionID,
			ExePath:   exe,
		}
		// exe パスにスペースが含まれると、Claude が statusLine を実行する Git Bash と
		// PowerShell で必要なクォート形式が非互換（usage_hooks.go: toShellPath 参照）のため、
		// status line（トークン/使用量表示）が沈黙して動かないことがある。沈黙バグを可視化する。
		if settingsOpts.StatusLine && strings.ContainsAny(exe, " ") {
			logger.Warn("claude statusLine exe path contains a space; the token/usage status line may silently fail to run because Git Bash and PowerShell need incompatible quoting — install many-ai-cli to a path without spaces",
				"session_id", sessionID, "exe_path", exe)
		}
		if slPath, cleanup, slErr := WriteClaudeSessionSettings(hp, settingsOpts); slErr == nil {
			// 分岐到達＋temp 作成成功を stderr ログで確定できるよう 1 行残す（diag 継続）。
			logger.Info("claude_settings_written", "session_id", sessionID, "path", slPath,
				"status_line", settingsOpts.StatusLine, "cross_session_inbound_accept", settingsOpts.CrossSessionInboundAccept)
			providerArgs = append(providerArgs, "--settings", slPath)
			defer cleanup()
		} else {
			logger.Warn("claude session settings write failed", "session_id", sessionID, "err", slErr)
		}
	}

	if *utf8Session {
		applyUTF8Session()
	}
	if provider == "opencode" && !loginMode {
		permValue := "ask"
		if *permissionMode == "bypassPermissions" {
			permValue = "allow"
		}
		if cleanupCfg, cfgErr := prepareOpenCodeConfig(cwd, permValue, logger); cfgErr != nil {
			logger.Warn("opencode: failed to prepare opencode.json permission config", "session_id", sessionID, "err", cfgErr)
		} else {
			defer cleanupCfg()
		}
	}
	// C2 (plan_orchestration-spawn-ui-exposure.md): conductor / orchestration child
	// セッションの実 CLI プロセスにだけ、`many-ai-cli orchestrate spawn` が自力で
	// Hub API を叩けるよう session ID と Hub token を env 経由で渡す。AI がプロンプト上で
	// token を扱わずに済むよう、コマンドライン引数ではなく継承 env に載せる（usage-relay と
	// 同じ受け渡しパターン）。オーケストレーション対象外セッションには一切付与しない
	// （既存セッションの env 露出面を広げないため）。
	var providerExtraEnv []string
	if reg.OrchestrationID != "" {
		providerExtraEnv = []string{
			fmt.Sprintf("MANY_AI_CLI_SESSION_ID=%d", sessionID),
			fmt.Sprintf("%s=%s", hubTokenEnvName, cfg.Token),
		}
	}
	ps, err := startProcess(provider, providerArgs, cwd, initCols, initRows, providerExtraEnv)
	if err != nil {
		// Hub 側の spawn ログ (~/.many-ai-cli/logs/spawn/<provider>-<ts>.log) に
		// 何が起きたかを残し、Hub UI のセッションカード「Disconnected」表示に
		// 「reason: provider not found in PATH」等を 1 行付けるための reason
		// コードを session_end で送る。生のスタックトレースは UI に流さない。
		diagnoseStartFailure(os.Stderr, provider, providerArgs, err)
		reason := classifyStartFailure(err)
		_ = websocket.JSON.Send(conn, proto.Message{
			Type:      "session_end",
			SessionID: sessionID,
			State:     "error",
			ExitCode:  1,
			Reason:    reason,
		})
		_ = conn.Close()
		return err
	}
	defer ps.Close()

	wses := newWrapperSession(conn, sessionID, wrapperSendWriteTimeout(cfg))

	// sendInputAck は「この seq の入力は PTY に入っている」ことを Hub へ返す。
	// 新規書き込み後と、再送を握り潰したときの両方から呼ぶ（後者で ack を返さないと
	// Hub 側の in-flight が永久に残り、切断のたびに再送され続ける）。
	sendInputAck := func(seq int64) {
		if seq <= 0 {
			return
		}
		if err := wses.sendMsg(proto.Message{
			Type:      "pty_input_ack",
			SessionID: wses.getSID(),
			InputSeq:  seq,
		}); err != nil {
			// PTY 書き込みは成功済みなので、ack の送信失敗だけでは
			// PTY や wrapper の処理を止めない。Hub は未 ack として再送するが、
			// maxProcessedInputSeq により二重書き込みにはならない。
			logger.Warn("pty_input_ack send failed", "session_id", wses.getSID(), "input_seq", seq, "err", err)
		}
	}

	// Recent PTY output buffer: replayed to UI after a successful reconnect so
	// the new session card has context for what happened during the gap.
	var (
		replayMu    sync.Mutex
		replayBuf   bytes.Buffer
		replayTotal int64 // PTY から読み出した累計バイト数（reattach で Hub へ申告する）
	)
	appendReplay := func(chunk []byte) {
		replayMu.Lock()
		defer replayMu.Unlock()
		replayBuf.Write(chunk)
		replayTotal += int64(len(chunk))
		if replayBuf.Len() > replayBufferLimit {
			replayBuf.Next(replayBuf.Len() - replayBufferLimit)
		}
	}
	// snapshotReplay は replay バッファと累計バイト数を同一ロック下で返す。
	// 別々に取ると両者の間に appendReplay が挟まり、Hub 側の差分計算
	// （累計の差で replay の末尾を切り出す）が 1 チャンクぶんずれる。
	snapshotReplay := func() ([]byte, int64) {
		replayMu.Lock()
		defer replayMu.Unlock()
		out := make([]byte, replayBuf.Len())
		copy(out, replayBuf.Bytes())
		return out, replayTotal
	}

	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	// hub_shutdown を受信したら立つフラグ。意図的シャットダウンと判定し、
	// reconnect supervisor は probeHubAlive と ensureHub を skip して直接
	// grace 期間に入る。PTY は kill しない（= ユーザーが手動で Hub を再起動
	// したら reattach で復活、新コンソール窓ポップアップも発生しない）。
	// 再接続成功時に false に戻す。
	var intentionalShutdown atomic.Bool
	var transportBroken atomic.Bool
	var lastReattachAt atomic.Int64

	reconnectCh := make(chan struct{}, 1)
	notifyReconnect := func() {
		select {
		case reconnectCh <- struct{}{}:
		default:
		}
	}

	startReceiveLoop := func(c *websocket.Conn) {
		go func() {
			defer func() {
				if r := recover(); r != nil {
					logger.Error("hub-to-wrapper goroutine panic", "session_id", wses.getSID(), "recover", r)
					wasCurrent := wses.isCurrentConn(c)
					wses.clearConn(c)
					if wasCurrent && !transportBroken.Load() {
						notifyReconnect()
					}
				}
			}()
			for {
				var m proto.Message
				if err := websocket.JSON.Receive(c, &m); err != nil {
					select {
					case <-done:
						return
					default:
					}
					logger.Info("hub WS read failed", "session_id", wses.getSID(), "err", err)
					wasCurrent := wses.isCurrentConn(c)
					wses.clearConn(c)
					if wasCurrent && !transportBroken.Load() {
						notifyReconnect()
					}
					return
				}
				switch m.Type {
				case "pty_input":
					if len(m.Data) > 0 {
						data := m.Data
						var writeErr error
						if wses.inputSeqAlreadyProcessed(m.InputSeq) {
							// Hub が未 ack と判断して再送してきたが、この seq は既に PTY へ
							// 書き終えている（PTY 書き込み後・ack 到達前に WS が切れると起きる）。
							// 二重に書くと確定 \r が 2 回入って後続プロンプトを誤承認するため、
							// 書かずに ack だけ返して Hub の in-flight を解消する。
							sendInputAck(m.InputSeq)
							// 観測専用の一時コード（原因が確定したら撤去）。台帳 id=hub-input-deliver-trace
							traceInputDeliver(logger, cfg.Log.SessionEnabled, "pty_write_dup", wses.getSID(),
								"input_seq", m.InputSeq, "bytes", len(m.Data))
							break
						}
						if lead, rest := splitLeadingClearControl(provider, data); lead != nil {
							if err := writePTY(ps, lead); err != nil {
								writeErr = err
								logPTYWriteError(logger, wses.getSID(), "clear_prefix", err)
							}
							time.Sleep(clearPrefixSplitDelay)
							data = rest
						}
						if provider == "claude" && len(data) > 1 && data[0] == '@' && looksLikeInjectPath(data[1:]) {
							// PTYW-7 (report_bug_security_quality_audit_2026-07-05.md):
							// 旧形式の inject (@path\rtext\r) は互換のため分割する。
							// 単に data[0] == '@' で判定していた頃は、@user_mention 等の一般テキストも
							// 誤って分割対象になっていた。@ 直後が絶対パスの形状 (POSIX の `/` 始まり or
							// Windows のドライブレター `[A-Za-z]:[\\/]`) の時のみ旧経路とみなすよう狭めた。
							// 新形式 (@path text\r) は画像参照と本文を同じ入力行に残し、最後の Enter だけ分離する。
							if idx := bytes.IndexByte(data, '\r'); idx >= 0 && idx < len(data)-1 {
								if err := writeWithTrailingEnter(ps, data[:idx+1], 150*time.Millisecond); err != nil {
									if writeErr == nil {
										writeErr = err
									}
									logPTYWriteError(logger, wses.getSID(), "inject_path", err)
								}
								// ConPTY fix: text\r を1チャンクで書くと \r が Enter でなく改行扱いになる場合がある
								rest := data[idx+1:]
								if err := writeWithTrailingEnter(ps, rest, 20*time.Millisecond); err != nil {
									if writeErr == nil {
										writeErr = err
									}
									logPTYWriteError(logger, wses.getSID(), "inject_text", err)
								}
							} else {
								if err := writeWithTrailingEnter(ps, data, 20*time.Millisecond); err != nil {
									if writeErr == nil {
										writeErr = err
									}
									logPTYWriteError(logger, wses.getSID(), "inject", err)
								}
							}
						} else if shouldSplitTrailingEnter(provider, data) {
							// Windows ConPTY では text+\r を1チャンクで書き込むと
							// \r が Enter ではなく改行として処理される場合がある。
							// Codex / OpenCode は入力反映直後の Enter を取りこぼすことがあるため長めに待つ。
							delay := trailingEnterDelay(provider)
							if err := writeWithTrailingEnter(ps, data, delay); err != nil {
								writeErr = err
								logPTYWriteError(logger, wses.getSID(), "input_enter", err)
							}
						} else {
							// ブラケットペースト等で \r なしの大量入力が来る場合も
							// writePTYChunked でチャンク分割して書き込む。
							// splitBracketedPasteSubmit で \r が分離された後の
							// ペースト本文はこのブランチを通るため、以前の
							// writeWithTrailingEnter 経由と同等の書き込み挙動を維持する。
							if err := writePTYChunked(ps, data); err != nil {
								writeErr = err
								logPTYWriteError(logger, wses.getSID(), "input", err)
							}
						}
						if writeErr == nil {
							// 記録は書き込み成功時のみ。失敗分を記録すると、Hub が
							// 再送しても「処理済み」と見なして握り潰してしまう。
							wses.markInputSeqProcessed(m.InputSeq)
							sendInputAck(m.InputSeq)
						}
						// 観測専用の一時コード（原因が確定したら撤去）。台帳 id=hub-input-deliver-trace
						traceInputDeliver(logger, cfg.Log.SessionEnabled, "pty_write", wses.getSID(),
							"input_seq", m.InputSeq, "bytes", len(m.Data), "err", writeErr != nil)
					}
				case "pty_resize":
					if m.Cols > 0 && m.Rows > 0 {
						_ = ps.Resize(uint16(m.Cols), uint16(m.Rows))
					}
				case "attach_file":
					if err := HandleAttach(m, ps); err != nil {
						logger.Warn("attach inject failed", "err", err)
					}
				case "hub_shutdown":
					logger.Info("hub_shutdown received — skipping reconnect and ensureHub", "session_id", wses.getSID(), "reason", m.Reason)
					intentionalShutdown.Store(true)
					wses.clearConn(c)
					notifyReconnect()
					return
				case proto.TypeSessionDismissed:
					// Hub 側でこのセッションが明示的に閉じられた（UI の × / 一括 dismiss 等）。
					// WS の EOF から意図を推定する経路（reconnectSupervisor）は
					// postReattachGuard(10s) 以内だと「回線不調」に倒れ、2 秒後に再接続して
					// セッションを復活させてしまう
					// （bugfix_session-dismiss-ignored-within-reattach-guard_2026-08-13.md）。
					// 通知が届いた時点で推定は不要なので、PTY ごと終了して supervisor を起こさない。
					// hub_shutdown と違い PTY は残さない（あちらは Hub の手動再起動を待つため）。
					logger.Info("session_dismissed received — terminating PTY", "session_id", wses.getSID(), "reason", m.Reason)
					wses.clearConn(c)
					_ = ps.Close()
					closeDone()
					return
				}
			}
		}()
	}
	startReceiveLoop(conn)

	transportFailure := func(fault ptyOutputTransportFault) {
		if fault.Err != nil {
			logger.Warn("PTY output transport failure", "session_id", wses.getSID(), "kind", fault.Kind, "queue_capacity", fault.QueueCapacity, "chunk_bytes", fault.ChunkBytes, "err", fault.Err)
		} else {
			logger.Warn("PTY output transport failure", "session_id", wses.getSID(), "kind", fault.Kind, "queue_capacity", fault.QueueCapacity, "chunk_bytes", fault.ChunkBytes)
		}
		// Set this before closing the socket. The receive loop may observe the
		// close first; supervisor must still treat this as a reconnectable
		// transport fault even while the Hub HTTP endpoint remains alive.
		transportBroken.Store(true)
		wses.abortCurrentConn()
		notifyReconnect()
	}
	outputWriter := newPTYOutputWriter(wses, ptyOutputQueueCapacity, transportFailure)
	go outputWriter.run()

	// Reconnect supervisor: distinguishes intentional session close (Hub HTTP
	// alive) from Hub process exit (Hub HTTP unreachable). The former kills PTY
	// immediately (preserving dismiss / kill-all / idle-timeout UX). For a Hub
	// process exit, auto_shutdown=true treats closing the Hub terminal as the end
	// of all wrapped sessions. When auto_shutdown=false, wrappers wait for a
	// manual Hub restart during the configured grace period, but never spawn a
	// replacement Hub on their own; otherwise closing the Hub terminal revives the
	// Windows console and Web UI unexpectedly.
	// hub_shutdown 受信（= UI からの Hub のみ停止）は intentional フラグ経由
	// の特別経路で、PTY は kill せず grace 期間中の手動 Hub 再起動を待つ。
	sup := &reconnectSupervisor{
		cfg:              cfg,
		logger:           logger,
		ws:               wses,
		ps:               ps,
		provider:         provider,
		display:          display,
		cwd:              cwd,
		label:            *label,
		model:            *model,
		agentSessionID:   agentSessionID,
		startedAtText:    startedAtText,
		rawLogPath:       rawLogPath,
		jsonlPath:        jsonlPath,
		intentional:      &intentionalShutdown,
		done:             done,
		closeDone:        closeDone,
		reconnectCh:      reconnectCh,
		startReceiveLoop: startReceiveLoop,
		snapshotReplay:   snapshotReplay,
		transportBroken:  &transportBroken,
		lastReattachAt:   &lastReattachAt,
		resumeOutput:     outputWriter.resume,
	}
	go sup.run()

	// PTY 出力 → Hub WS (pty_data)
	// carry holds an incomplete UTF-8 sequence from the previous read that must
	// be prepended to the next chunk before decoding (pumpChunk handles this).
	rawMaxBytes := int64(cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	ptyPump(ps, lf, rawMaxBytes, outputWriter.enqueue, outputWriter.finish, appendReplay, closeDone)

	waitErr := ps.Wait()
	state, code, signal := classifyExit(waitErr)
	endMsg := proto.Message{Type: "session_end", SessionID: wses.getSID(), State: state, ExitCode: code}
	if signal != "" {
		endMsg.Signal = signal
	}
	wses.sendMsg(endMsg)
	// Close the current conn after the final message is sent.
	wses.closeAnyConn()
	if signal != "" {
		logger.Info("wrapper exit", "session_id", wses.getSID(), "state", state, "exit_code", code, "signal", signal)
	} else {
		logger.Info("wrapper exit", "session_id", wses.getSID(), "state", state, "exit_code", code)
	}
	return waitErr
}

// classifyExit turns ps.Wait()'s error into a coarse state label
// ("completed"/"error") plus the concrete exit code and, on Unix when the
// child was terminated by a signal (e.g. OOM kill, SIGTERM/SIGKILL), the
// signal name. This lets session_end logging distinguish a clean non-zero
// exit from a forcibly killed process instead of collapsing both into a
// fixed code=1.
func classifyExit(waitErr error) (state string, code int, signal string) {
	if waitErr == nil {
		return "completed", 0, ""
	}
	state, code = "error", 1
	var exitErr *exec.ExitError
	if errors.As(waitErr, &exitErr) {
		code = exitErr.ExitCode()
		if signaled, name := exitSignalInfo(exitErr); signaled {
			signal = name
		}
	}
	return state, code, signal
}

// dialAndRegister opens a WS to the Hub and performs the register handshake.
func dialAndRegister(cfg *config.Config, provider, display, cwd, label, model, agentSessionID string, termCols, termRows int) (*websocket.Conn, proto.Message, error) {
	wsURL := url.URL{Scheme: "ws", Host: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Path: "/ws"}
	// Origin はサーバの wsHandshake 許可リスト（http://127.0.0.1:<port>）に一致させる。
	// websocket.Dial は内部で url.ParseRequestURI(origin) を呼ぶため空文字は不可（empty url エラー）。
	// 末尾スラッシュ付き・ポートなしの "http://127.0.0.1/" だと許可リストに一致せず bad status。
	origin := fmt.Sprintf("http://127.0.0.1:%d", cfg.Hub.Port)
	conn, err := websocket.Dial(wsURL.String(), "", origin)
	if err != nil {
		return nil, proto.Message{}, err
	}
	homeDir, codexHome, claudeDir := userSkillDirs()
	if err := websocket.JSON.Send(conn, proto.Message{
		Type:           "register",
		Role:           "wrapper",
		Provider:       provider,
		Display:        display,
		CWD:            cwd,
		Label:          label,
		Model:          model,
		PID:            os.Getpid(),
		Shell:          DetectShell(),
		Token:          cfg.Token,
		HomeDir:        homeDir,
		CodexHome:      codexHome,
		ClaudeDir:      claudeDir,
		AgentSessionID: agentSessionID,
		SubscriptionID: activeSubscriptionID(),
		Cols:           termCols,
		Rows:           termRows,
	}); err != nil {
		_ = conn.Close()
		return nil, proto.Message{}, err
	}
	var reg proto.Message
	if err := websocket.JSON.Receive(conn, &reg); err != nil {
		_ = conn.Close()
		return nil, proto.Message{}, err
	}
	// registered 以外（例: エラー応答や予期しない先頭フレーム）だと TokenStatusbar 等が
	// ゼロ値のまま残り statusline が黙って無効になる。型不一致は明示エラーにする。
	if reg.Type != "registered" {
		_ = conn.Close()
		return nil, proto.Message{}, fmt.Errorf("unexpected register response type %q (want registered)", reg.Type)
	}
	return conn, reg, nil
}

func dialAndReattach(cfg *config.Config, sessionID int, provider, display, cwd, label, model, agentSessionID, startedAt, rawLogPath, jsonlPath string, termCols, termRows int, replay []byte, ptyBytes int64) (*websocket.Conn, int, error) {
	wsURL := url.URL{Scheme: "ws", Host: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Path: "/ws"}
	// Origin はポート付きで許可リストに一致させる（理由は dialAndRegister のコメント参照）。
	origin := fmt.Sprintf("http://127.0.0.1:%d", cfg.Hub.Port)
	conn, err := websocket.Dial(wsURL.String(), "", origin)
	if err != nil {
		return nil, 0, err
	}
	homeDir, codexHome, claudeDir := userSkillDirs()
	if err := websocket.JSON.Send(conn, proto.Message{
		Type:           "reattach",
		Role:           "wrapper",
		SessionID:      sessionID,
		Provider:       provider,
		Display:        display,
		CWD:            cwd,
		Label:          label,
		Model:          model,
		PID:            os.Getpid(),
		Shell:          DetectShell(),
		Token:          cfg.Token,
		HomeDir:        homeDir,
		CodexHome:      codexHome,
		ClaudeDir:      claudeDir,
		AgentSessionID: agentSessionID,
		SubscriptionID: activeSubscriptionID(),
		Cols:           termCols,
		Rows:           termRows,
		StartedAt:      startedAt,
		LogPath:        rawLogPath,
		JSONLPath:      jsonlPath,
		ReplayB64:      base64.StdEncoding.EncodeToString(replay),
		PTYBytes:       ptyBytes,
	}); err != nil {
		_ = conn.Close()
		return nil, 0, err
	}
	var resp proto.Message
	if err := websocket.JSON.Receive(conn, &resp); err != nil {
		_ = conn.Close()
		return nil, 0, err
	}
	if resp.Type == "reattach_reject" {
		_ = conn.Close()
		return nil, 0, nil
	}
	if resp.Type != "reattach_ack" || resp.SessionID <= 0 {
		_ = conn.Close()
		return nil, 0, fmt.Errorf("unexpected reattach response: %s", resp.Type)
	}
	return conn, resp.SessionID, nil
}

func userSkillDirs() (homeDir, codexHome, claudeDir string) {
	homeDir, _ = os.UserHomeDir()
	return homeDir, os.Getenv("CODEX_HOME"), os.Getenv("CLAUDE_CONFIG_DIR")
}

// activeSubscriptionID returns the subscription profile id the Hub selected for
// this session, or "" when the session uses the CLI's own login environment.
func activeSubscriptionID() string {
	return strings.TrimSpace(os.Getenv(subscription.SessionEnvVar))
}

// probeHubAlive returns true if the Hub HTTP server responds at 127.0.0.1:port.
// Used after a WS read failure to decide whether the disconnect was intentional
// (Hub still up — dismiss / kill-all / idle-timeout) or a Hub crash.
func probeHubAlive(cfg *config.Config) bool {
	u := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, url.QueryEscape(cfg.Token))
	client := &http.Client{Timeout: hubProbeTimeout}
	resp, err := client.Get(u)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func waitForHubReady(cfg *config.Config, timeout time.Duration) bool {
	deadline := time.Now().Add(timeout)
	for {
		if probeHubAlive(cfg) {
			return true
		}
		if time.Now().After(deadline) {
			return false
		}
		time.Sleep(hubStartupPoll)
	}
}

func runningHubPort(cfg *config.Config) (int, bool) {
	return hubruntime.RunningPort(cfg.Hub.Port, cfg.Token)
}

func ensureHub(cfg *config.Config, logger *slog.Logger) error {
	// Hub にスポーンされた場合（MANY_AI_CLI=1）、Hub は既に動いている。
	// 新 Hub を起動すると PID ファイル経由で実際の Hub が kill される危険があるため
	// プローブと起動を一切スキップする。
	if os.Getenv("MANY_AI_CLI") == "1" {
		return nil
	}
	if port, ok := runningHubPort(cfg); ok {
		// The runtime ledger may point at a fallback port. Keep this effective
		// value local to the wrapper process; it is not persisted as config.
		cfg.Hub.Port = port
		// 既存 Hub が古いバイナリ（ディスクの exe と内容が食い違う）で、かつ
		// アクティブセッションが無ければ自動で停止する。停止できたら下の spawn
		// 経路で新バイナリの Hub を起動し直す。そうでなければ既存 Hub を使う。
		if !maybeRestartStaleHub(cfg, logger) {
			return nil
		}
	}
	// 設定ポートが WSL 側 Hub など別プロセスに使用されている場合に備え、
	// 実際にバインドできるポートを確認してから Hub を起動する。
	// cfg.Hub.Port を更新することで後続の dialAndRegister も正しいポートを使う。
	cfg.Hub.Port = findFreePort(cfg.Hub.Port)
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	serve := exec.Command(exe, "serve", "--port", strconv.Itoa(cfg.Hub.Port))
	prepareHubSpawn(serve)
	if err := serve.Start(); err != nil {
		return err
	}
	// 親 wrapper が Wait しないと Unix でゾンビ化する。reaper は成功・失敗両方で必須。
	go func() {
		_ = serve.Wait()
	}()
	if !waitForHubReady(cfg, hubStartupTimeout) {
		if serve.Process != nil {
			_ = serve.Process.Kill()
		}
		// Wait は上の reaper が回収する。短い猶予でプロセス消滅を待つ。
		deadline := time.Now().Add(2 * time.Second)
		for serve.ProcessState == nil && time.Now().Before(deadline) {
			time.Sleep(20 * time.Millisecond)
		}
		return fmt.Errorf("hub did not become ready on port %d within %s", cfg.Hub.Port, hubStartupTimeout)
	}
	return nil
}

// findFreePort は preferred ポートから順に 127.0.0.1 でバインドできる最初のポートを返す。
// preferred が空いていればそのまま返す。100 ポート試してすべて塞がれていたら preferred を返す。
func findFreePort(preferred int) int {
	for p := preferred; p < preferred+100; p++ {
		ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", p))
		if err == nil {
			_ = ln.Close()
			return p
		}
	}
	return preferred
}

type processSession interface {
	io.ReadWriteCloser
	Wait() error
	Resize(cols, rows uint16) error
}
