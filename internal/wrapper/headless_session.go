package wrapper

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/headless"
	"many-ai-cli/internal/proto"
)

// headless_session.go is the `many-ai-cli wrap <provider> --headless` half of
// the wrapper (子 plan: docs/local/plan_child_execution_modes_headless.md 内部 C2).
//
// **Why the runner lives inside the wrapper process rather than inside the Hub.**
// A headless run could have been an exec.Cmd in the Hub, but then the Hub would
// have two kinds of session: one that registers over the WebSocket, logs,
// survives a Hub restart, can be stopped from the UI and replays its output,
// and one that does none of that until each piece is written a second time. By
// running it under `wrap`, a headless session reaches the Hub through exactly
// the same register → output → session_end path as a PTY session, and
// everything already built on that path works without knowing this mode exists.
//
// What is deliberately *not* reused from the interactive path: the Claude
// settings file (statusLine has nothing to draw in print mode), the delegation
// prompt (a one-shot worker does not spawn children), the PTY resize / input
// handling, and the reconnect supervisor (a run that loses its Hub has nothing
// to reattach to — 元設計 22 節 accepts that for the first version).

// headlessSession is one non-interactive run, resolved by Run before it starts.
type headlessSession struct {
	cfg    *config.Config
	logger *slog.Logger
	conn   *websocket.Conn
	// sessionID is the Hub session this run reports as. The session already
	// exists: register happened before this ran, so the card is on screen from
	// the first line of output.
	sessionID int
	provider  string
	// customProvider is the config.yaml `custom_providers:` entry when this
	// launch names one. Its `command:` is split into argv and resolved through
	// the same resolveCmd as an interactive launch, so npm shims and the
	// per-provider exceptions stay in one place.
	customProvider   config.CustomProvider
	isCustomProvider bool
	// providerArgs are the arguments the launch contributes (model, effort,
	// permission mode, allowlist, Claude's --session-id) — built by the same
	// code the PTY path uses (親 plan からの差分 5).
	providerArgs []string
	promptPath   string
	cwd          string
	extraEnv     []string
}

// runHeadlessSession executes the run and reports its end to the Hub. It
// returns an error only when the run could not be started; a provider that ran
// and failed ends with a session_end carrying its exit code, exactly like a PTY
// session that exited non-zero.
func runHeadlessSession(s headlessSession) error {
	def, ok := config.HeadlessDefFor(s.provider, s.cfg)
	if !ok {
		return fmt.Errorf("provider %q has no headless definition; run it without --headless, or add a headless: block to its custom_providers entry", s.provider)
	}
	var customArgv []string
	if s.isCustomProvider {
		argv, argvErr := s.customProvider.Argv()
		if argvErr != nil {
			// Same failure shape as the PTY path: the diagnostic goes to the
			// spawn log and the session ends with a reason, rather than a new
			// kind of error nobody has seen before
			// (decision 7, plan_custom-provider-spawn-execution.md).
			return s.failStart(argvErr, nil)
		}
		customArgv = argv
	}
	prompt, err := takePromptFile(s.promptPath)
	if err != nil {
		return err
	}
	// The Hub wrote the prompt before this session had an id; fill it in now
	// (internal/headless/prompt.go explains why the id is a placeholder).
	prompt = headless.ExpandPrompt(prompt, s.sessionID)

	argv := headless.BuildArgv(def, s.providerArgs, prompt)
	exe, args := resolveCmd(s.provider, customArgv, argv)

	spec := headless.Spec{
		Exe:       exe,
		Argv:      args,
		Prompt:    prompt,
		PromptVia: def.PromptVia,
		Format:    def.Format,
		CWD:       s.cwd,
		// Colour is turned off rather than inherited: a headless stream is
		// meant to be read by a parser, and escape sequences in it are at best
		// noise. The normalized lines are sanitized again on the way out
		// (internal/headless/event.go).
		Env:          append(childEnv(os.Environ(), config.TerminalColorOff), s.extraEnv...),
		RawLogPrefix: headlessRawLogPrefix(s.cfg, s.provider, time.Now()),
	}

	wses := newWrapperSession(s.conn, s.sessionID, wrapperSendWriteTimeout(s.cfg))
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	s.watchHubForStop(wses, cancel)

	emit := func(chunk []byte) {
		if err := wses.sendMsg(proto.Message{
			Type:      "pty_data",
			SessionID: wses.getSID(),
			Data:      chunk,
		}); err != nil {
			// The run is not the Hub's to fail: a lost connection loses the
			// display, not the work. There is no reconnect in this mode, so
			// this is logged at debug to avoid one line per output line.
			s.logger.Debug("headless output send failed", "session_id", wses.getSID(), "err", err)
		}
	}

	s.logger.Info("headless run starting",
		"session_id", s.sessionID, "provider", s.provider,
		"format", def.Format, "prompt_via", def.PromptVia,
		"raw_log", spec.RawLogPrefix != "")

	result, runErr := headless.Run(ctx, spec, emit)
	if runErr != nil {
		return s.failStart(runErr, args)
	}

	// The provider's own verdict is already in the stream; this last line is
	// the one thing only the wrapper knows, and it is what "completed" means
	// here (元設計 11 節: process exit is the completion signal).
	emit([]byte(headlessEndLine(result) + "\r\n"))
	_ = wses.sendMsg(proto.Message{
		Type:      "session_end",
		SessionID: wses.getSID(),
		State:     result.State,
		ExitCode:  result.ExitCode,
	})
	wses.closeAnyConn()
	s.logger.Info("headless run finished",
		"session_id", s.sessionID, "provider", s.provider,
		"state", result.State, "exit_code", result.ExitCode,
		"timed_out", result.TimedOut, "canceled", result.Canceled)
	return nil
}

// failStart reports a launch that never became a process: the human-readable
// diagnostic goes to the wrapper's stderr (which the Hub keeps as the spawn
// log) and the session ends with the same short reason code the PTY path uses,
// so the card says "provider not found in PATH" rather than just disconnecting.
func (s headlessSession) failStart(err error, args []string) error {
	diagnoseStartFailure(os.Stderr, s.provider, args, err)
	_ = websocket.JSON.Send(s.conn, proto.Message{
		Type:      "session_end",
		SessionID: s.sessionID,
		State:     "error",
		ExitCode:  1,
		Reason:    classifyStartFailure(err),
	})
	_ = s.conn.Close()
	return err
}

// headlessEndLine is the final line of a headless session's output.
func headlessEndLine(result headless.Result) string {
	switch {
	case result.TimedOut:
		return headless.Event{Type: headless.EventRunFailed, Text: "run timed out"}.Line()
	case result.Canceled:
		return headless.Event{Type: headless.EventRunFailed, Text: "run was stopped"}.Line()
	default:
		return headless.Event{
			Type:    headless.EventRunCompleted,
			Text:    fmt.Sprintf("exit=%d", result.ExitCode),
			IsError: result.State != "completed",
		}.Line()
	}
}

// watchHubForStop turns the Hub's own stop signals into a cancelled run. The
// PTY path closes the PTY for these; here the equivalent is killing the process
// tree, which is what cancelling the context does (元設計 21 節).
//
// pty_input is ignored on purpose. A headless provider read its prompt and
// closed stdin; there is nowhere for typed text to go, and pretending otherwise
// would be worse than the input box being disabled.
func (s headlessSession) watchHubForStop(wses *wrapperSession, cancel context.CancelFunc) {
	conn := s.conn
	go func() {
		for {
			var m proto.Message
			if err := websocket.JSON.Receive(conn, &m); err != nil {
				return
			}
			switch m.Type {
			case proto.TypeSessionDismissed:
				s.logger.Info("session_dismissed received — stopping headless run",
					"session_id", wses.getSID(), "reason", m.Reason)
				cancel()
				return
			case "hub_shutdown":
				// The run keeps going: it is a one-shot worker with a finite
				// end, and killing someone's model turn because the Hub was
				// restarted would cost more than the lost output.
				s.logger.Info("hub_shutdown received — headless output will stop being delivered",
					"session_id", wses.getSID(), "reason", m.Reason)
				return
			}
		}
	}()
}

// takePromptFile reads the initial prompt and deletes the file. The prompt
// travels as a file so it never appears in an argument list, a process table or
// a log; deleting it here is the other half of that — the Hub writes it, the
// wrapper consumes it, and nothing is left behind
// (internal/doctor/residue.go's rule: a feature that writes into the user's
// files designs its own collection).
//
// The contents are never logged, by this function or any caller: it is the
// user's own instruction to their AI.
func takePromptFile(path string) (string, error) {
	if path == "" {
		return "", nil
	}
	defer func() { _ = os.Remove(path) }()
	data, err := os.ReadFile(path) // #nosec G304 -- path は Hub が組んだ一時ファイルで、利用者の入力ではない
	if err != nil {
		return "", fmt.Errorf("read prompt file: %w", err)
	}
	return string(data), nil
}

// headlessRawLogPrefix returns the path prefix for this run's raw stdout /
// stderr logs, or "" when they must not be written.
//
// The gate is log.session_enabled, the same opt-in that governs the raw PTY log
// — and for the same reason: raw provider output carries API keys, tokens and
// passwords unmasked (internal/config: LogConfig.SessionEnabled). 元設計 20 節
// asks for the files and for private file mode; it does not get to make raw
// output appear on disk by default when the rest of the product has decided it
// does not.
func headlessRawLogPrefix(cfg *config.Config, provider string, now time.Time) string {
	if cfg == nil || !cfg.Log.SessionEnabled {
		return ""
	}
	dir := cfg.Hub.LogDir
	if dir == "" {
		return ""
	}
	return filepath.Join(dir, "headless", fmt.Sprintf("%s_%s", provider, now.Format("20060102-150405.000")))
}
