package headless

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

// The provider CLI in these tests is this test binary, re-executed with
// -test.run=TestHeadlessHelperProcess and told what to do through one
// environment variable (元設計 25 節: fixture 中心・実 CLI を CI 必須にしない).
// The point is that a real process really starts, really writes to stdout and
// stderr, and really exits with a code — the parts that a fake io.Reader cannot
// check.
const helperModeEnv = "MANY_AI_CLI_HEADLESS_TEST_MODE"

func TestHeadlessHelperProcess(t *testing.T) {
	mode := os.Getenv(helperModeEnv)
	if mode == "" {
		t.Skip("not a helper process invocation")
	}
	switch mode {
	case "stream":
		// A synthetic stream-json run: one turn, one tool call, one result.
		os.Stdout.WriteString(`{"type":"system","subtype":"init","model":"test-model"}` + "\n")
		os.Stdout.WriteString(`{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working"}]}}` + "\n")
		os.Stdout.WriteString("this line is not json\n")
		os.Stdout.WriteString(`{"type":"result","subtype":"success","result":"finished"}` + "\n")
		os.Stderr.WriteString("a warning on stderr\n")
	case "echo-stdin":
		buf := make([]byte, 4096)
		n, _ := os.Stdin.Read(buf)
		os.Stdout.WriteString("prompt=" + strings.TrimSpace(string(buf[:n])) + "\n")
	case "echo-args":
		os.Stdout.WriteString("args=" + strings.Join(os.Args[1:], "|") + "\n")
	case "echo-env":
		os.Stdout.WriteString("env=" + os.Getenv("MANY_AI_CLI_HEADLESS_TEST_VALUE") + "\n")
	case "fail":
		os.Stdout.WriteString("partial work\n")
		os.Stderr.WriteString("boom\n")
		os.Exit(3)
	case "hang":
		os.Stdout.WriteString("started\n")
		time.Sleep(30 * time.Second)
	case "long-line":
		// One line three times longer than maxLineBytes, then a short one.
		// Reading must not stop at the cap: the tail line is the proof that the
		// stream was drained past it.
		os.Stdout.WriteString(strings.Repeat("x", 3*maxLineBytes) + "\n")
		os.Stdout.WriteString("tail\n")
	case "grandchild-hang":
		// A shim: start the real "provider" (this binary again, hanging) with
		// our stdout inherited, then hang ourselves. Killing only this process
		// would leave the grandchild holding the pipe open.
		child := exec.Command(os.Args[0], "-test.run=^TestHeadlessHelperProcess$")
		child.Env = append(os.Environ(), helperModeEnv+"=hang")
		child.Stdout = os.Stdout
		child.Stderr = os.Stderr
		if err := child.Start(); err != nil {
			os.Stderr.WriteString("grandchild start failed: " + err.Error() + "\n")
			os.Exit(2)
		}
		os.Stdout.WriteString("shim started\n")
		time.Sleep(30 * time.Second)
	}
	// Exit before the testing framework prints anything, or the PASS line ends
	// up in the "provider" output the parent is parsing.
	os.Exit(0)
}

// helperSpec builds a Spec that launches this test binary as the provider.
func helperSpec(t *testing.T, mode, format string, argv ...string) Spec {
	t.Helper()
	exe, err := os.Executable()
	if err != nil {
		t.Fatalf("os.Executable: %v", err)
	}
	return Spec{
		Exe:    exe,
		Argv:   append([]string{"-test.run=^TestHeadlessHelperProcess$"}, argv...),
		Format: format,
		CWD:    t.TempDir(),
		Env:    append(os.Environ(), helperModeEnv+"="+mode),
	}
}

type lineSink struct {
	mu    sync.Mutex
	lines []string
}

func (s *lineSink) emit(chunk []byte) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.lines = append(s.lines, string(chunk))
}

func (s *lineSink) joined() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return strings.Join(s.lines, "")
}

func TestRunStreamsParsedEventsAndExitsZero(t *testing.T) {
	sink := &lineSink{}
	result, err := Run(context.Background(), helperSpec(t, "stream", config.HeadlessFormatClaudeStreamJSON), sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if result.State != "completed" || result.ExitCode != 0 {
		t.Fatalf("result = %+v, want completed with exit 0", result)
	}
	out := sink.joined()
	for _, want := range []string{"[run] started model=test-model", "[assistant] working", "this line is not json", "[result] success: finished", "[stderr] a warning on stderr"} {
		if !strings.Contains(out, want) {
			t.Errorf("output missing %q:\n%s", want, out)
		}
	}
	// Every line is CRLF-terminated: the browser draws these in a terminal.
	for _, line := range sink.lines {
		if !strings.HasSuffix(line, "\r\n") {
			t.Fatalf("line %q does not end with CRLF", line)
		}
	}
}

// The exit code is the verdict, not what the model said about itself
// (元設計 11 節 / 31 節 9).
func TestRunReportsNonZeroExit(t *testing.T) {
	sink := &lineSink{}
	result, err := Run(context.Background(), helperSpec(t, "fail", config.HeadlessFormatText), sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if result.State != "error" || result.ExitCode != 3 {
		t.Fatalf("result = %+v, want error with exit 3", result)
	}
	if out := sink.joined(); !strings.Contains(out, "partial work") || !strings.Contains(out, "[stderr] boom") {
		t.Errorf("output = %q, want the work and the stderr line", out)
	}
}

func TestRunPassesPromptOnStdin(t *testing.T) {
	sink := &lineSink{}
	spec := helperSpec(t, "echo-stdin", config.HeadlessFormatText)
	spec.Prompt = "review the diff"
	spec.PromptVia = config.HeadlessPromptViaStdin
	if _, err := Run(context.Background(), spec, sink.emit); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if out := sink.joined(); !strings.Contains(out, "prompt=review the diff") {
		t.Errorf("output = %q, want the prompt delivered on stdin", out)
	}
}

func TestRunPassesPromptAsArgumentAndClosesStdin(t *testing.T) {
	sink := &lineSink{}
	def := config.HeadlessDef{PromptVia: config.HeadlessPromptViaArg}
	argv := BuildArgv(def, nil, "review the diff")
	spec := helperSpec(t, "echo-args", config.HeadlessFormatText, argv...)
	spec.Prompt = "review the diff"
	spec.PromptVia = config.HeadlessPromptViaArg
	if _, err := Run(context.Background(), spec, sink.emit); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if out := sink.joined(); !strings.Contains(out, "args=-test.run=^TestHeadlessHelperProcess$|review the diff") {
		t.Errorf("output = %q, want the prompt as one argv entry", out)
	}
}

// The subscription profile reaches the provider as environment, exactly like an
// interactive session (元設計 16 節). Asserting it on the child process rather
// than on the Spec is the rule in CLAUDE/coding.md: "渡したつもり" is not a test.
func TestRunPassesEnvironmentToTheProcess(t *testing.T) {
	sink := &lineSink{}
	spec := helperSpec(t, "echo-env", config.HeadlessFormatText)
	spec.Env = append(spec.Env, "MANY_AI_CLI_HEADLESS_TEST_VALUE=profile-b")
	if _, err := Run(context.Background(), spec, sink.emit); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if out := sink.joined(); !strings.Contains(out, "env=profile-b") {
		t.Errorf("output = %q, want the env value the child actually received", out)
	}
}

func TestRunTimesOutAndKillsTheProcess(t *testing.T) {
	sink := &lineSink{}
	spec := helperSpec(t, "hang", config.HeadlessFormatText)
	spec.Timeout = 300 * time.Millisecond
	started := time.Now()
	result, err := Run(context.Background(), spec, sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 20*time.Second {
		t.Fatalf("Run took %v: the timeout did not reach the process", elapsed)
	}
	if !result.TimedOut || result.State != "error" {
		t.Fatalf("result = %+v, want a timed-out error", result)
	}
}

func TestRunCancelKillsTheProcess(t *testing.T) {
	sink := &lineSink{}
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(300 * time.Millisecond)
		cancel()
	}()
	defer cancel()
	started := time.Now()
	result, err := Run(ctx, helperSpec(t, "hang", config.HeadlessFormatText), sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 20*time.Second {
		t.Fatalf("Run took %v: the cancellation did not reach the process", elapsed)
	}
	if !result.Canceled || result.State != "error" {
		t.Fatalf("result = %+v, want a cancelled error", result)
	}
}

// Cancelling must reach the descendants, not just the process we started: a
// provider CLI is usually a shim in front of the real binary, and that binary
// inherits our pipes. If only the shim died, the readers would wait for the
// grandchild's 30 s sleep and Run would not return in time.
func TestRunCancelKillsTheWholeProcessTree(t *testing.T) {
	sink := &lineSink{}
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(500 * time.Millisecond)
		cancel()
	}()
	defer cancel()
	started := time.Now()
	result, err := Run(ctx, helperSpec(t, "grandchild-hang", config.HeadlessFormatText), sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 15*time.Second {
		t.Fatalf("Run took %v: the grandchild kept the pipe open, so the cancellation did not reach the tree", elapsed)
	}
	if !result.Canceled || result.State != "error" {
		t.Fatalf("result = %+v, want a cancelled error", result)
	}
}

// A line past maxLineBytes is delivered in pieces and reading continues. With a
// scanner that stops at the cap, the provider would block on a full pipe and the
// "tail" line would never arrive.
func TestRunDeliversOverlongLinesInPiecesAndKeepsReading(t *testing.T) {
	sink := &lineSink{}
	started := time.Now()
	result, err := Run(context.Background(), helperSpec(t, "long-line", config.HeadlessFormatText), sink.emit)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 15*time.Second {
		t.Fatalf("Run took %v: reading stalled on the overlong line", elapsed)
	}
	if result.State != "completed" || result.ExitCode != 0 {
		t.Fatalf("result = %+v, want completed with exit 0", result)
	}
	if !strings.Contains(sink.joined(), "tail\r\n") {
		t.Fatalf("the line after the overlong one never arrived; got %d lines", len(sink.lines))
	}
	if len(sink.lines) < 4 {
		t.Fatalf("expected the overlong line in at least 3 pieces plus the tail, got %d lines", len(sink.lines))
	}
}

// An unknown format is refused before anything starts: a run whose output is
// read by the wrong parser looks fine and means something else.
func TestRunRejectsUnknownFormat(t *testing.T) {
	spec := helperSpec(t, "stream", "made-up-format")
	if _, err := Run(context.Background(), spec, nil); err == nil {
		t.Fatal("Run with an unknown format must fail")
	}
}

// Raw provider output is only written when the caller asks for a prefix — the
// opt-in this repository already applies to raw session logs, because the bytes
// are unmasked (元設計 20 節).
func TestRunWritesRawLogsOnlyWhenAsked(t *testing.T) {
	dir := t.TempDir()
	prefix := filepath.Join(dir, "sub", "claude_20260912-000000")
	spec := helperSpec(t, "stream", config.HeadlessFormatClaudeStreamJSON)
	spec.RawLogPrefix = prefix
	if _, err := Run(context.Background(), spec, nil); err != nil {
		t.Fatalf("Run: %v", err)
	}
	stdout, err := os.ReadFile(prefix + ".stdout.log")
	if err != nil {
		t.Fatalf("raw stdout log: %v", err)
	}
	if !strings.Contains(string(stdout), `"type":"result"`) {
		t.Errorf("raw stdout log = %q, want the provider's own bytes", stdout)
	}
	stderr, err := os.ReadFile(prefix + ".stderr.log")
	if err != nil {
		t.Fatalf("raw stderr log: %v", err)
	}
	if !strings.Contains(string(stderr), "a warning on stderr") {
		t.Errorf("raw stderr log = %q, want the stderr bytes", stderr)
	}

	// No prefix, no file anywhere.
	quiet := t.TempDir()
	spec2 := helperSpec(t, "stream", config.HeadlessFormatClaudeStreamJSON)
	spec2.CWD = quiet
	if _, err := Run(context.Background(), spec2, nil); err != nil {
		t.Fatalf("Run: %v", err)
	}
	entries, err := os.ReadDir(quiet)
	if err != nil {
		t.Fatalf("ReadDir: %v", err)
	}
	if len(entries) != 0 {
		t.Errorf("a run without a raw log prefix wrote %d files", len(entries))
	}
}
