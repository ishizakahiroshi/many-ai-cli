package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"many-ai-cli/internal/execpath"
	"many-ai-cli/internal/provider"
)

// cli_version.go is the "バージョン確認" button's backend
// (docs/local/plan_provider-cli-update_c2_version-api.md C1/C2): run every
// enabled AI provider's --version (or its manifest's version_args) at once,
// on request only. Nothing here runs on a timer or at Hub startup.

const (
	// cliVersionOutputCap bounds the combined stdout+stderr text kept per
	// provider. Past it, output is silently discarded rather than dropping the
	// run — many CLIs also print update banners or telemetry notices before
	// the version line, and the first line is what version_line uses anyway.
	cliVersionOutputCap = 8 * 1024
	// cliVersionMaxConcurrency bounds how many provider CLIs run at once so a
	// user with many custom providers configured cannot fork a burst of
	// processes in one click.
	cliVersionMaxConcurrency = 8
)

// cliVersionCheckTimeout is a var, not a const, so tests can shorten it
// instead of waiting out a real 10s timeout to exercise the kill path.
var cliVersionCheckTimeout = 10 * time.Second

// cliVersionResult is one provider's outcome. Error is one of exactly four
// values ("見つからない" / "打ち切り" / "終了コード N" / "出力が空") so the UI
// can render a fixed set of reasons instead of arbitrary Go error text.
type cliVersionResult struct {
	Provider             string `json:"provider"`
	Executable           string `json:"executable,omitempty"`
	VersionLine          string `json:"version_line,omitempty"`
	VersionText          string `json:"version_text,omitempty"`
	ExecutableModifiedAt string `json:"executable_modified_at,omitempty"`
	ExitCode             int    `json:"exit_code"`
	Error                string `json:"error,omitempty"`
}

type cliVersionResponse struct {
	CheckedAt string             `json:"checked_at"`
	Results   []cliVersionResult `json:"results"`
}

// cliVersionState keeps the single most recent result in memory (経緯:
// 2026-09-23 決まったこと表「直近の結果は保存しない」) plus, while a run is in
// progress, the in-flight run every concurrent caller waits on instead of
// starting a second one.
type cliVersionState struct {
	mu       sync.Mutex
	last     *cliVersionResponse
	inflight *cliVersionInflight
}

type cliVersionInflight struct {
	done   chan struct{}
	result cliVersionResponse
}

func newCLIVersionState() *cliVersionState {
	return &cliVersionState{}
}

// run executes work unless a run is already in progress, in which case it
// waits for that run and returns its result — "同時に 2 回押されたら、2 回目は
// 実行中の結果を待って同じものを返す（二重に走らせない）". A nil receiver (a
// *Server built without newCLIVersionState, e.g. a narrow test fixture) just
// runs work uncached rather than panicking.
func (s *cliVersionState) run(ctx context.Context, work func(context.Context) cliVersionResponse) cliVersionResponse {
	if s == nil {
		return work(ctx)
	}
	s.mu.Lock()
	if s.inflight != nil {
		inflight := s.inflight
		s.mu.Unlock()
		<-inflight.done
		return inflight.result
	}
	inflight := &cliVersionInflight{done: make(chan struct{})}
	s.inflight = inflight
	s.mu.Unlock()

	result := work(ctx)

	s.mu.Lock()
	inflight.result = result
	s.last = &result
	s.inflight = nil
	s.mu.Unlock()
	close(inflight.done)
	return result
}

func (s *cliVersionState) lastResult() (cliVersionResponse, bool) {
	if s == nil {
		return cliVersionResponse{}, false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.last == nil {
		return cliVersionResponse{}, false
	}
	return *s.last, true
}

// checkCLIVersions runs providerIDs (or, when empty, every enabled non-shell
// provider) concurrently, bounded by cliVersionMaxConcurrency, and returns
// one result per requested id in the same order.
func checkCLIVersions(ctx context.Context, registry *provider.Registry, providerIDs []string) cliVersionResponse {
	ids := providerIDs
	if len(ids) == 0 {
		ids = allCLIVersionProviderIDs(registry)
	}
	results := make([]cliVersionResult, len(ids))
	sem := make(chan struct{}, cliVersionMaxConcurrency)
	var wg sync.WaitGroup
	for i, id := range ids {
		wg.Add(1)
		sem <- struct{}{}
		go func(i int, id string) {
			defer wg.Done()
			defer func() { <-sem }()
			definition, ok := registry.Lookup(id)
			if !ok || definition.Launch == nil {
				results[i] = cliVersionResult{Provider: id, Error: "見つからない"}
				return
			}
			results[i] = checkOneCLIVersion(ctx, id, definition)
		}(i, id)
	}
	wg.Wait()
	return cliVersionResponse{
		CheckedAt: time.Now().UTC().Format(time.RFC3339),
		Results:   results,
	}
}

// allCLIVersionProviderIDs is the "省略・空なら「有効で、shell ではない AI
// provider 全部」" default. shell is excluded defensively even though its id
// is reserved and never present in the registry (handleProviderCreate).
func allCLIVersionProviderIDs(registry *provider.Registry) []string {
	if registry == nil {
		return nil
	}
	var ids []string
	for _, summary := range registry.List() {
		if summary.ID == "shell" || !summary.Enabled {
			continue
		}
		ids = append(ids, summary.ID)
	}
	return ids
}

// checkOneCLIVersion resolves the same executable candidate wrapper's launch
// would pick (providerCommandFound's candidate order, first LookPath hit),
// unwraps Windows npm shims through execpath the same way, then runs the
// provider's version_args and classifies the outcome.
func checkOneCLIVersion(ctx context.Context, id string, definition provider.EffectiveDefinition) cliVersionResult {
	result := cliVersionResult{Provider: id}

	candidatePath, found := selectCLIVersionExecutablePath(definition.Launch)
	if !found {
		result.Error = "見つからない"
		return result
	}

	versionArgs := provider.ResolveVersionArgs(definition.Update)
	exe, args := execpath.Resolve(candidatePath, versionArgs)
	result.Executable = exe
	if info, statErr := os.Stat(exe); statErr == nil {
		result.ExecutableModifiedAt = info.ModTime().UTC().Format(time.RFC3339)
	}

	output, exitCode, timedOut, startErr := runCLIVersionCommand(ctx, exe, args, cliVersionCheckTimeout)
	result.VersionText = output
	result.VersionLine = firstNonEmptyLine(output)
	result.ExitCode = exitCode

	switch {
	case timedOut:
		result.Error = "打ち切り"
	case startErr != nil:
		// LookPath found the candidate but exec.Start still failed (e.g. removed
		// between LookPath and Start, or not actually executable). Reported the
		// same as not found: either way there is nothing to run.
		result.Error = "見つからない"
	case exitCode != 0:
		result.Error = fmt.Sprintf("終了コード %d", exitCode)
	case strings.TrimSpace(output) == "":
		result.Error = "出力が空"
	}
	return result
}

// selectCLIVersionExecutablePath mirrors providerCommandFound's candidate
// order (launch.Executable, then launch.ExecutableCandidates) but returns the
// resolved path of the first PATH hit instead of just a bool, since the
// version check actually has to run it.
func selectCLIVersionExecutablePath(launch *provider.LaunchDefinition) (string, bool) {
	if launch == nil {
		return "", false
	}
	var candidates []string
	if launch.Executable != "" {
		candidates = append(candidates, launch.Executable)
	}
	candidates = append(candidates, launch.ExecutableCandidates...)
	for _, candidate := range candidates {
		if candidate == "" {
			continue
		}
		if path, err := providerCommandLookPath(candidate); err == nil {
			return path, true
		}
	}
	return "", false
}

// runCLIVersionCommand runs exe(args...) with no stdin (a CLI waiting on
// interactive input would otherwise hang until the timeout every time),
// capped combined stdout+stderr, and kills the whole process tree — not just
// the process this started — on cancel or timeout. See
// cli_version_proc_windows.go / cli_version_proc_other.go: a provider
// resolved through a Windows npm shim can still be cmd.exe running node, and
// killing only cmd.exe would leave node running past the budget.
//
// timedOut is tracked via its own timer (mirrors internal/headless.Run)
// rather than inspecting ctx.Err() after Wait returns, which would also read
// true on the ordinary race between a process finishing and the timer firing.
func runCLIVersionCommand(ctx context.Context, exe string, args []string, timeout time.Duration) (output string, exitCode int, timedOut bool, startErr error) {
	return runCLIProcessCapped(ctx, exe, args, timeout, cliVersionOutputCap)
}

// runCLIProcessCapped is runCLIVersionCommand generalized over the output
// cap, so cli_update.go's longer-running update commands can keep more
// output than the 8KB version-check cap without duplicating the
// timeout/kill-process-tree plumbing. The OS-specific tree-kill hooks it
// calls (configureCLIVersionProcAttr etc.) are not actually version-check
// specific — just named for their first caller — and are reused as-is.
func runCLIProcessCapped(ctx context.Context, exe string, args []string, timeout time.Duration, outputCap int) (output string, exitCode int, timedOut bool, startErr error) {
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	var timedOutFlag atomic.Bool
	if timeout > 0 {
		timer := time.AfterFunc(timeout, func() {
			timedOutFlag.Store(true)
			cancel()
		})
		defer timer.Stop()
	}

	cmd := exec.Command(exe, args...) // #nosec G204 -- argv is provider-definition-resolved (executable candidate + update.version_args), never a shell string
	cmd.Env = sanitizeEnv(os.Environ())
	cmd.Stdin = nil

	buf := &cappedWriter{limit: outputCap}
	cmd.Stdout = buf
	cmd.Stderr = buf
	configureCLIVersionProcAttr(cmd)

	if err := cmd.Start(); err != nil {
		return "", 0, false, err
	}
	job, jobErr := attachCLIVersionProcessJob(cmd)
	if jobErr == nil {
		defer closeCLIVersionProcessJob(job)
	}

	done := make(chan struct{})
	go func() {
		select {
		case <-runCtx.Done():
			killCLIVersionProcessTree(cmd, job)
		case <-done:
		}
	}()

	waitErr := cmd.Wait()
	close(done)

	exitCode = 0
	if waitErr != nil {
		exitCode = 1
		var exitErr *exec.ExitError
		if errors.As(waitErr, &exitErr) {
			exitCode = exitErr.ExitCode()
		}
	}
	return buf.String(), exitCode, timedOutFlag.Load(), nil
}

// cappedWriter drops bytes past limit instead of erroring, so a chatty CLI
// never sees a short write on stdout/stderr and blocks on a full pipe until
// the timeout kills it. Stdout and stderr are copied by two goroutines
// os/exec starts internally, so writes are concurrent and need the mutex.
type cappedWriter struct {
	mu    sync.Mutex
	buf   bytes.Buffer
	limit int
}

func (w *cappedWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if remaining := w.limit - w.buf.Len(); remaining > 0 {
		if len(p) > remaining {
			w.buf.Write(p[:remaining])
		} else {
			w.buf.Write(p)
		}
	}
	return len(p), nil
}

func (w *cappedWriter) String() string {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.buf.String()
}

// firstNonEmptyLine is version_line: the first non-blank line of
// version_text, CRLF-tolerant.
func firstNonEmptyLine(text string) string {
	for _, raw := range strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n") {
		line := strings.TrimSpace(raw)
		if line != "" {
			return line
		}
	}
	return ""
}

// normalizeCLIVersionProviderIDs trims, drops empties and dedupes while
// preserving request order, so a client-supplied "providers" list cannot run
// the same provider twice concurrently against the shared cappedWriter/exec.
func normalizeCLIVersionProviderIDs(ids []string) []string {
	if len(ids) == 0 {
		return nil
	}
	seen := make(map[string]struct{}, len(ids))
	out := make([]string, 0, len(ids))
	for _, id := range ids {
		id = strings.TrimSpace(id)
		if id == "" {
			continue
		}
		if _, dup := seen[id]; dup {
			continue
		}
		seen[id] = struct{}{}
		out = append(out, id)
	}
	return out
}

type cliVersionRequest struct {
	Providers []string `json:"providers"`
}

// handleCLIVersions dispatches GET (直近の結果) and POST (実行) for
// /api/cli-versions.
func (s *Server) handleCLIVersions(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodPost {
		s.handleCLIVersionsPost(w, r)
		return
	}
	s.handleCLIVersionsGet(w, r)
}

func (s *Server) handleCLIVersionsGet(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	result, ok := s.cliVersions.lastResult()
	if !ok {
		writeJSON(w, cliVersionResponse{Results: []cliVersionResult{}})
		return
	}
	writeJSON(w, result)
}

func (s *Server) handleCLIVersionsPost(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body cliVersionRequest
	if r.Body != nil && r.Body != http.NoBody && r.ContentLength != 0 {
		dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, jsonBodyMaxBytes))
		if err := dec.Decode(&body); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid json")
			return
		}
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	ids := normalizeCLIVersionProviderIDs(body.Providers)

	// r.Context() is cancelled once the HTTP response is written, which would
	// kill an in-flight run's process tree out from under a second caller
	// still waiting on cliVersionState.run's <-inflight.done. Detached on
	// purpose; runCLIVersionCommand's own 10s-per-provider timeout is what
	// actually bounds this.
	result := s.cliVersions.run(context.Background(), func(ctx context.Context) cliVersionResponse {
		return checkCLIVersions(ctx, registry, ids)
	})
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, result)
}
