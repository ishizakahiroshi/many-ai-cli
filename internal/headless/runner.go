package headless

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
)

// runner.go is the non-PTY half of many-ai-cli: exec.Cmd plus pipes
// (元設計 9 節).
//
// Why not a PTY: a headless CLI writes a machine-readable stream, and a PTY
// would change it — colour and cursor codes appear, JSON gets decorated, and a
// process that thinks it has a terminal waits at prompts nobody can answer.
//
// What this file guarantees:
//
//   - The prompt is handed over as data (stdin) or as one argv entry. It is
//     never interpolated into a command string, and no shell is involved
//     (元設計 26 節).
//   - stdout is read line by line and handed to the format's parser; a line the
//     parser cannot read never fails the run (元設計 10 節).
//   - The verdict is the process exit code, not anything the model said
//     (元設計 11 節 / 31 節 9).
//   - Cancelling or timing out kills the whole process tree, on every OS
//     (proc_windows.go / proc_other.go).

// maxLineBytes bounds one delivered line of provider stdout. A stream-json line
// carries a whole model turn — a tool result can embed a whole file — so this
// is generous; past it the line is delivered in pieces rather than dropped
// (readLines), which the structured parser reports as plain output lines
// instead of losing them. Stopping to read at the cap is not an option: an
// unread pipe fills, and a provider blocked on stdout never exits.
const maxLineBytes = 1 << 20

// Run executes one headless launch and blocks until the process exits, the
// context is cancelled, or the timeout fires.
//
// emit is called with one complete line of session output at a time, already
// terminated with CRLF: the Hub hands these to the browser's terminal exactly
// like PTY bytes, and a bare LF would leave the next line indented by the
// previous one's length. emit is called from one goroutine at a time.
//
// The returned error is non-nil only when the process could not be started at
// all. A process that ran and failed is a Result with State "error" — the
// caller reports that as a session that ended, not as a launch that broke.
func Run(ctx context.Context, spec Spec, emit func([]byte)) (Result, error) {
	parser, ok := ParserFor(spec.Format)
	if !ok {
		return Result{}, fmt.Errorf("headless: no parser for output format %q", spec.Format)
	}
	if emit == nil {
		emit = func([]byte) {}
	}
	if ctx == nil {
		ctx = context.Background()
	}
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	// timedOut is written by the timer goroutine and read after the process
	// exits, so it is atomic rather than a plain bool.
	var timedOut atomic.Bool
	if spec.Timeout > 0 {
		timer := time.AfterFunc(spec.Timeout, func() {
			timedOut.Store(true)
			cancel()
		})
		defer timer.Stop()
	}

	cmd := exec.Command(spec.Exe, spec.Argv...) // #nosec G204 -- argv は定義表と起動側が組んだ引数で、shell を介さない（元設計 26 節）
	cmd.Dir = spec.CWD
	cmd.Env = spec.Env
	configureProcCmd(cmd)

	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return Result{}, fmt.Errorf("headless: stdout pipe: %w", err)
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return Result{}, fmt.Errorf("headless: stderr pipe: %w", err)
	}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return Result{}, fmt.Errorf("headless: stdin pipe: %w", err)
	}

	rawOut, rawErr := openRawLogs(spec.RawLogPrefix)
	defer closeRawLogs(rawOut, rawErr)

	if err := cmd.Start(); err != nil {
		_ = stdin.Close()
		return Result{}, err
	}
	job, jobErr := attachProcessJob(cmd)
	if jobErr == nil {
		defer closeProcessJob(job)
	}

	// The readers start before the prompt is written. A prompt longer than the
	// pipe buffer and a provider that prints before it has read stdin would
	// otherwise deadlock each other — we block writing stdin, it blocks writing
	// stdout — with nobody draining either end.
	var wg sync.WaitGroup
	var emitMu sync.Mutex
	emitLine := func(line string) {
		emitMu.Lock()
		defer emitMu.Unlock()
		emit([]byte(line + "\r\n"))
	}
	wg.Add(2)
	go func() {
		defer wg.Done()
		readLines(stdout, rawOut, func(line []byte) {
			for _, event := range parser.Parse(line) {
				emitLine(event.Line())
			}
		})
	}()
	go func() {
		defer wg.Done()
		readLines(stderr, rawErr, func(line []byte) {
			if len(line) == 0 {
				return
			}
			emitLine(Event{Type: EventStderr, Text: string(line)}.Line())
		})
	}()

	// The prompt goes in and stdin is closed straight away. Closing it is as
	// important as writing it: a CLI in print mode reads stdin to EOF, and an
	// open pipe is an unfinished prompt it will wait for forever. It runs in
	// its own goroutine so a provider that never reads stdin cannot hold up the
	// readers above.
	go writePrompt(stdin, spec)

	// Cancellation has to reach the whole process tree, not just the process we
	// started: a provider is usually a shim that launches the real binary, and
	// that binary inherits our stdout/stderr pipes. Kill only the shim and the
	// pipes stay open, the readers never see EOF, and this function never
	// returns (which on Windows is also what would have closed the job).
	// killProcessTree therefore terminates the job / process group first.
	done := make(chan struct{})
	go func() {
		select {
		case <-runCtx.Done():
			killProcessTree(cmd, job)
		case <-done:
		}
	}()

	wg.Wait()
	waitErr := cmd.Wait()
	close(done)

	result := Result{State: "completed"}
	if waitErr != nil {
		result.State = "error"
		result.ExitCode = 1
		var exitErr *exec.ExitError
		if errors.As(waitErr, &exitErr) {
			result.ExitCode = exitErr.ExitCode()
		}
	}
	switch {
	case timedOut.Load():
		result.TimedOut = true
		result.State = "error"
	case runCtx.Err() != nil:
		result.Canceled = true
		result.State = "error"
	}
	return result, nil
}

// writePrompt hands the prompt to the process and closes stdin. A prompt that
// travels as an argument still gets an immediate EOF rather than an open pipe.
func writePrompt(stdin io.WriteCloser, spec Spec) {
	defer func() { _ = stdin.Close() }()
	if config.NormalizeHeadlessPromptVia(spec.PromptVia) != config.HeadlessPromptViaStdin {
		return
	}
	if spec.Prompt == "" {
		return
	}
	_, _ = io.WriteString(stdin, spec.Prompt)
}

// readLines reads r line by line, mirrors each delivered line into raw (when
// the user opted into raw logs) and hands it to onLine. A line longer than
// maxLineBytes is delivered in pieces rather than dropped: bufio.Scanner would
// stop with ErrTooLong at the cap and leave the rest of the stream unread,
// which is a full pipe and a provider that never exits. onLine must not retain
// the slice it is given.
func readLines(r io.Reader, raw *os.File, onLine func([]byte)) {
	br := bufio.NewReaderSize(r, 64*1024)
	deliver := func(line []byte) {
		if raw != nil {
			_, _ = raw.Write(append(append([]byte(nil), line...), '\n'))
		}
		onLine(line)
	}
	var pending []byte
	for {
		chunk, isPrefix, err := br.ReadLine()
		if len(chunk) > 0 {
			pending = append(pending, chunk...)
		}
		if err == nil && isPrefix && len(pending) < maxLineBytes {
			// The line continues past the reader's buffer; keep collecting it.
			continue
		}
		if err == nil {
			// A complete line, or a piece of one that reached the cap.
			deliver(pending)
			pending = pending[:0]
			continue
		}
		if len(pending) > 0 {
			deliver(pending)
		}
		if !errors.Is(err, io.EOF) && !errors.Is(err, os.ErrClosed) {
			onLine([]byte(fmt.Sprintf("many-ai-cli: output stream ended early (%v)", err)))
		}
		return
	}
}

// openRawLogs creates the two raw output files when the caller asked for them.
// **The caller decides whether to ask**: raw provider output carries API keys,
// tokens and passwords unmasked, which is why this repository keeps raw session
// output behind log.session_enabled (internal/config: LogConfig.SessionEnabled).
// The files are private-mode, in the same tree as every other log, and nothing
// else in many-ai-cli writes raw headless output anywhere (元設計 20 節).
func openRawLogs(prefix string) (stdout, stderr *os.File) {
	if prefix == "" {
		return nil, nil
	}
	if err := os.MkdirAll(filepath.Dir(prefix), sessionlog.PrivateDirMode); err != nil {
		return nil, nil
	}
	open := func(suffix string) *os.File {
		f, err := os.OpenFile(prefix+suffix, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode)
		if err != nil {
			return nil
		}
		return f
	}
	return open(".stdout.log"), open(".stderr.log")
}

func closeRawLogs(files ...*os.File) {
	for _, f := range files {
		if f != nil {
			_ = f.Close()
		}
	}
}
