package hub

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/execpath"
	"many-ai-cli/internal/provider"
	"many-ai-cli/internal/sessionlog"
)

// cli_update.go is the "更新" / "全部更新" button's backend
// (docs/local/plan_provider-cli-update_c3_update-api.md, child C1/C2/C3):
// decide whether a provider CLI can be updated right now, actually run its
// update command, classify the outcome, keep a per-provider execution log,
// and make sure an update in flight and a new session launch for the same
// provider can never overlap. Nothing here runs on a timer — every update is
// a POST a human pressed.

// The 6 reasons a provider cannot be updated right now (親 plan C1). Exactly
// one of these is ever set on an ineligible cliUpdatePlan / excluded entry.
const (
	cliUpdateReasonNotInstalled       = "not_installed"
	cliUpdateReasonUpdateDisabled     = "update_disabled"
	cliUpdateReasonNotConfigured      = "update_not_configured"
	cliUpdateReasonRunningSessions    = "running_sessions"
	cliUpdateReasonAlreadyUpdating    = "already_updating"
	cliUpdateReasonLoginMayBeRequired = "login_may_be_required"
)

// The provider-level outcomes a finished update job status can carry, plus
// the two in-flight states.
const (
	cliUpdateStateQueued        = "queued"
	cliUpdateStateRunning       = "running"
	cliUpdateStateUpdated       = "updated"
	cliUpdateStateLatest        = "latest"
	cliUpdateStateUnknown       = "unknown"
	cliUpdateStateFailed        = "failed"
	cliUpdateStateFileInUse     = "file_in_use"
	cliUpdateStateLoginRequired = "login_required"
)

// cliUpdateOutputCap bounds the combined stdout+stderr text kept per update
// run. Larger than cliVersionOutputCap (8KB) because a real update command
// (npm install, pnpm add -g, etc.) can be considerably chattier than a
// --version check, and the full text is what gets written to the log file.
const cliUpdateOutputCap = 1 * 1024 * 1024

// cliUpdateMaxConcurrency bounds how many update commands run at once in the
// non-serial (default) mode, mirroring cliVersionMaxConcurrency.
const cliUpdateMaxConcurrency = 8

// cliUpdateJobHistoryLimit is "ジョブはメモリに直近 10 件だけ持つ".
const cliUpdateJobHistoryLimit = 10

// cliUpdateLogServeMaxBytes is the "上限 256 KB" the log-body endpoint serves.
// The log file on disk itself is not truncated to this size — only the HTTP
// response is.
const cliUpdateLogServeMaxBytes = 256 * 1024

// cliUpdateLoginRequiredPattern mirrors update-ai-clis.ps1's
// Test-AuthenticationFailure regex verbatim (親 plan 前提節: 「ログイン要求の
// 判定は update-ai-clis.ps1 の Test-AuthenticationFailure の正規表現をそのまま
// 移す」).
var cliUpdateLoginRequiredPattern = regexp.MustCompile(`(?i)(\[unauthenticated\]|\bunauthenticated\b|\bnot\s+authenticated\b|\bauthentication\s+(?:required|failed)\b|\blog[- ]?in\s+(?:required|needed)\b|\bsign[- ]?in\s+(?:required|needed)\b)`)

// cliUpdateFileInUseMarkers is the single table of "実行ファイルが使用中" output
// substrings (子 plan C2: 「使用中系の判定語は 1 か所の表に持つ」).
var cliUpdateFileInUseMarkers = []string{
	"EBUSY", "EPERM", "ETXTBSY",
	"Text file busy", "resource busy", "being used by another process",
	"Access is denied", "アクセスが拒否", "別のプロセスが使用中",
}

func cliUpdateOutputLooksFileInUse(output string) bool {
	for _, marker := range cliUpdateFileInUseMarkers {
		if strings.Contains(output, marker) {
			return true
		}
	}
	return false
}

// classifyCLIUpdateOutcome maps a finished update run to exactly one of the
// 6 result states. beforeOK/afterOK are whether the corresponding
// checkOneCLIVersion call itself succeeded (Error == ""); the text compared
// is the full version_text, never parsed as a number (親 plan 前提節).
func classifyCLIUpdateOutcome(output string, exitCode int, timedOut bool, startErr error, beforeOK bool, beforeText string, afterOK bool, afterText string) string {
	switch {
	case startErr != nil, timedOut:
		return cliUpdateStateFailed
	case cliUpdateLoginRequiredPattern.MatchString(output):
		return cliUpdateStateLoginRequired
	case exitCode != 0:
		if cliUpdateOutputLooksFileInUse(output) {
			return cliUpdateStateFileInUse
		}
		return cliUpdateStateFailed
	case !beforeOK || !afterOK:
		return cliUpdateStateUnknown
	case strings.TrimSpace(beforeText) == strings.TrimSpace(afterText):
		return cliUpdateStateLatest
	default:
		return cliUpdateStateUpdated
	}
}

// cliUpdatePlan is one provider's "can it update right now, and with what
// argv" answer. Eligible is false for exactly one of the 6 reason codes
// above. It never touches the "更新中" set or s.sessions — that atomic
// check lives in Server.beginProviderUpdate, run only once a POST actually
// starts a job, so a mere GET (eligibility listing) never itself blocks
// anything.
type cliUpdatePlan struct {
	Provider   string
	Eligible   bool
	Reason     string
	Executable string   // display: the selected launch candidate name (e.g. "claude", copilot's "gh")
	Argv       []string // display: executable followed by update args, e.g. ["claude", "update"]

	resolvedPath string // internal: providerCommandLookPath's absolute hit, used to actually exec
}

// planCLIUpdate resolves whether provider id can be updated right now. It is
// a free function (not a *Server method) so it is testable without a Server
// fixture.
func planCLIUpdate(id string, registry *provider.Registry) cliUpdatePlan {
	plan := cliUpdatePlan{Provider: id}
	definition, ok := registry.Lookup(id)
	if !ok || definition.Launch == nil {
		plan.Reason = cliUpdateReasonNotInstalled
		return plan
	}
	candidateName, candidatePath, found := selectCLIUpdateExecutable(definition.Launch)
	if !found {
		plan.Reason = cliUpdateReasonNotInstalled
		return plan
	}
	update := definition.Update
	if !provider.UpdateEnabled(update) {
		if update != nil && update.LoginMayBeRequired {
			plan.Reason = cliUpdateReasonLoginMayBeRequired
		} else {
			plan.Reason = cliUpdateReasonUpdateDisabled
		}
		return plan
	}
	argv, _ := provider.ResolveUpdateArgv(definition.Definition, candidateName)
	if argv == nil {
		plan.Reason = cliUpdateReasonNotConfigured
		return plan
	}
	plan.Executable = candidateName
	plan.Argv = argv
	plan.resolvedPath = candidatePath
	plan.Eligible = true
	return plan
}

// selectCLIUpdateExecutable mirrors selectCLIVersionExecutablePath but also
// returns the candidate name that resolved (launch.Executable, or one of
// executable_candidates), since provider.ResolveUpdateArgv compares against
// that name — not the absolute resolved path — to tell the primary
// executable (e.g. copilot's own binary) from a secondary fallback
// candidate (its "gh" launch fallback) that has no configured update method.
func selectCLIUpdateExecutable(launch *provider.LaunchDefinition) (name, path string, found bool) {
	if launch == nil {
		return "", "", false
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
		if resolved, err := providerCommandLookPath(candidate); err == nil {
			return candidate, resolved, true
		}
	}
	return "", "", false
}

// --- provider updating guard (子 plan C1) -----------------------------------

// countSessionsForProviderLocked counts entries of s.sessions for provider,
// following the same 「稼働中セッション」basis as activeLogBases: presence in
// s.sessions, regardless of whether the wrapper connection is still live
// (a disconnected-but-not-dismissed card still counts — session_dismiss is
// what actually removes the entry).
func countSessionsForProviderLocked(sessions map[int]*session, providerID string) int {
	n := 0
	for _, ses := range sessions {
		if ses != nil && ses.Provider == providerID {
			n++
		}
	}
	return n
}

// runningSessionCountForProvider is the read-only version of the same count,
// used by the eligibility listing endpoint (display only — not a gate).
func (s *Server) runningSessionCountForProvider(providerID string) int {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	return countSessionsForProviderLocked(s.sessions, providerID)
}

// providerUpdating reports whether providerID is mid-update. Called by the 3
// spawn entry points right after they validate the provider, so a launch
// that arrives after an update has started is refused (子 plan C1: 「更新中の
// 集合に入った後に起動が来たら必ず止まる」).
func (s *Server) providerUpdating(providerID string) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	_, ok := s.updatingCLIProviders[providerID]
	return ok
}

// beginProviderUpdate atomically checks that providerID has no running
// sessions and is not already updating, and if eligible marks it updating —
// all inside one sessionsMu critical section, closing the gap between "no
// sessions right now" and "a spawn starts before the update begins" (子 plan
// C1: 「判定の後にセッションが始まる隙間を作らない」). The reverse direction is
// providerUpdating's job.
func (s *Server) beginProviderUpdate(providerID string) (ok bool, reason string, runningSessions int) {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if _, updating := s.updatingCLIProviders[providerID]; updating {
		return false, cliUpdateReasonAlreadyUpdating, 0
	}
	if n := countSessionsForProviderLocked(s.sessions, providerID); n > 0 {
		return false, cliUpdateReasonRunningSessions, n
	}
	if s.updatingCLIProviders == nil {
		s.updatingCLIProviders = map[string]struct{}{}
	}
	s.updatingCLIProviders[providerID] = struct{}{}
	return true, "", 0
}

// endProviderUpdate releases the "更新中" mark, always called via defer once
// a provider's update run finishes (success or failure) so a launch can
// proceed again.
func (s *Server) endProviderUpdate(providerID string) {
	s.sessionsMu.Lock()
	delete(s.updatingCLIProviders, providerID)
	s.sessionsMu.Unlock()
}

// hubContext returns Run's cancel-on-shutdown context, so background work
// started from an HTTP handler (the update job runner) actually stops when
// the Hub shuts down instead of orphaning a running update process (親 plan
// 決まったこと表: 「Hub の終了時は実行中の更新を止める」). Falls back to
// context.Background() for tests / callers that build *Server without
// calling Run.
func (s *Server) hubContext() context.Context {
	s.hubCtxMu.Lock()
	defer s.hubCtxMu.Unlock()
	if s.hubCtx != nil {
		return s.hubCtx
	}
	return context.Background()
}

// --- job model ---------------------------------------------------------

// cliUpdateProviderStatus is one provider's live (or finished) state within
// an update job.
type cliUpdateProviderStatus struct {
	Provider      string   `json:"provider"`
	State         string   `json:"state"`
	Executable    string   `json:"executable,omitempty"`
	Argv          []string `json:"argv,omitempty"`
	VersionBefore string   `json:"version_before,omitempty"`
	VersionAfter  string   `json:"version_after,omitempty"`
	ExitCode      int      `json:"exit_code,omitempty"`
	StartedAt     string   `json:"started_at,omitempty"`
	FinishedAt    string   `json:"finished_at,omitempty"`
	LogAvailable  bool     `json:"log_available,omitempty"`

	logPath string // internal: not serialized, read by the log-body endpoint
}

// cliUpdateExcludedEntry is one provider POST /api/cli-updates declined to
// run, with the reason code the画面 uses to render fixed copy.
type cliUpdateExcludedEntry struct {
	Provider        string `json:"provider"`
	Reason          string `json:"reason"`
	RunningSessions int    `json:"running_sessions,omitempty"`
}

// cliUpdateJob is one POST /api/cli-updates call's bookkeeping. Fields are
// unexported: callers only ever see it through cliUpdateJobResponse.
type cliUpdateJob struct {
	id        string
	startedAt time.Time
	serial    bool

	mu       sync.Mutex
	accepted []string // provider IDs, request order
	statuses map[string]*cliUpdateProviderStatus
	excluded []cliUpdateExcludedEntry // fixed at creation, never mutated
}

func newCLIUpdateJob(id string, serial bool, accepted []string, excluded []cliUpdateExcludedEntry, now time.Time) *cliUpdateJob {
	statuses := make(map[string]*cliUpdateProviderStatus, len(accepted))
	for _, p := range accepted {
		statuses[p] = &cliUpdateProviderStatus{Provider: p, State: cliUpdateStateQueued}
	}
	return &cliUpdateJob{
		id:        id,
		startedAt: now,
		serial:    serial,
		accepted:  accepted,
		statuses:  statuses,
		excluded:  excluded,
	}
}

func (j *cliUpdateJob) setStatus(providerID string, status cliUpdateProviderStatus) {
	status.Provider = providerID
	j.mu.Lock()
	j.statuses[providerID] = &status
	j.mu.Unlock()
}

// snapshot returns a race-free copy of the job's current state for the GET
// status endpoint.
func (j *cliUpdateJob) snapshot() (providers []cliUpdateProviderStatus, excluded []cliUpdateExcludedEntry) {
	j.mu.Lock()
	defer j.mu.Unlock()
	providers = make([]cliUpdateProviderStatus, 0, len(j.accepted))
	for _, p := range j.accepted {
		if st := j.statuses[p]; st != nil {
			providers = append(providers, *st)
		}
	}
	excluded = append([]cliUpdateExcludedEntry(nil), j.excluded...)
	return providers, excluded
}

func (j *cliUpdateJob) logPathFor(providerID string) (string, bool) {
	j.mu.Lock()
	defer j.mu.Unlock()
	st := j.statuses[providerID]
	if st == nil || st.logPath == "" {
		return "", false
	}
	return st.logPath, true
}

func newCLIUpdateJobID() string {
	var raw [8]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "cliu-" + strconv.FormatInt(time.Now().UnixNano(), 16)
	}
	return "cliu-" + hex.EncodeToString(raw[:])
}

// registerCLIUpdateJob stores job, evicting the oldest once past
// cliUpdateJobHistoryLimit (「ジョブはメモリに直近 10 件だけ持つ」).
func (s *Server) registerCLIUpdateJob(job *cliUpdateJob) {
	s.cliUpdateJobsMu.Lock()
	defer s.cliUpdateJobsMu.Unlock()
	if s.cliUpdateJobByID == nil {
		s.cliUpdateJobByID = map[string]*cliUpdateJob{}
	}
	s.cliUpdateJobs = append(s.cliUpdateJobs, job)
	s.cliUpdateJobByID[job.id] = job
	for len(s.cliUpdateJobs) > cliUpdateJobHistoryLimit {
		oldest := s.cliUpdateJobs[0]
		s.cliUpdateJobs = s.cliUpdateJobs[1:]
		delete(s.cliUpdateJobByID, oldest.id)
	}
}

func (s *Server) cliUpdateJobLookup(id string) (*cliUpdateJob, bool) {
	s.cliUpdateJobsMu.Lock()
	defer s.cliUpdateJobsMu.Unlock()
	job, ok := s.cliUpdateJobByID[id]
	return job, ok
}

// --- execution -----------------------------------------------------------

// runCLIUpdateJob runs every accepted provider's update, concurrently unless
// job.serial (「失敗した分を1本ずつやり直す」), bounded by
// cliUpdateMaxConcurrency in the concurrent case.
func (s *Server) runCLIUpdateJob(job *cliUpdateJob, registry *provider.Registry, plans map[string]cliUpdatePlan) {
	ctx := s.hubContext()
	run := func(id string) {
		defer s.endProviderUpdate(id)
		s.runCLIUpdateJobProvider(ctx, job, registry, plans[id])
	}
	if job.serial {
		for _, id := range job.accepted {
			run(id)
		}
		return
	}
	sem := make(chan struct{}, cliUpdateMaxConcurrency)
	var wg sync.WaitGroup
	for _, id := range job.accepted {
		wg.Add(1)
		sem <- struct{}{}
		go func(id string) {
			defer wg.Done()
			defer func() { <-sem }()
			run(id)
		}(id)
	}
	wg.Wait()
}

// runCLIUpdateJobProvider runs one provider's update command to completion,
// classifies the outcome, writes its log file, and refreshes the「バージョン
// 確認」cache entry so the screen shows the new version without a second
// button press.
func (s *Server) runCLIUpdateJobProvider(ctx context.Context, job *cliUpdateJob, registry *provider.Registry, plan cliUpdatePlan) {
	startedAt := time.Now()
	job.setStatus(plan.Provider, cliUpdateProviderStatus{
		State:      cliUpdateStateRunning,
		Executable: plan.Executable,
		Argv:       plan.Argv,
		StartedAt:  startedAt.UTC().Format(time.RFC3339),
	})

	definition, ok := registry.Lookup(plan.Provider)
	if !ok {
		// The registry changed out from under us between planning and
		// running (e.g. settings edited mid-job); there is nothing left to
		// run.
		job.setStatus(plan.Provider, cliUpdateProviderStatus{
			State:      cliUpdateStateFailed,
			Executable: plan.Executable,
			Argv:       plan.Argv,
			StartedAt:  startedAt.UTC().Format(time.RFC3339),
			FinishedAt: time.Now().UTC().Format(time.RFC3339),
		})
		return
	}

	before := checkOneCLIVersion(ctx, plan.Provider, definition)
	beforeOK := before.Error == ""

	timeout := time.Duration(provider.ResolveUpdateTimeoutSeconds(definition.Update)) * time.Second
	var args []string
	if len(plan.Argv) > 1 {
		args = plan.Argv[1:]
	}
	exe, resolvedArgs := execpath.Resolve(plan.resolvedPath, args)
	output, exitCode, timedOut, startErr := runCLIProcessCapped(ctx, exe, resolvedArgs, timeout, cliUpdateOutputCap)

	var after cliVersionResult
	afterOK := false
	if startErr == nil && !timedOut && exitCode == 0 {
		after = checkOneCLIVersion(ctx, plan.Provider, definition)
		afterOK = after.Error == ""
	}

	state := classifyCLIUpdateOutcome(output, exitCode, timedOut, startErr, beforeOK, before.VersionText, afterOK, after.VersionText)
	finishedAt := time.Now()

	status := cliUpdateProviderStatus{
		State:         state,
		Executable:    plan.Executable,
		Argv:          plan.Argv,
		VersionBefore: before.VersionLine,
		VersionAfter:  after.VersionLine,
		ExitCode:      exitCode,
		StartedAt:     startedAt.UTC().Format(time.RFC3339),
		FinishedAt:    finishedAt.UTC().Format(time.RFC3339),
	}

	if logPath, err := writeCLIUpdateLog(s.cfg.Hub.LogDir, plan.Provider, startedAt, finishedAt, plan.Argv, exitCode, timedOut, startErr, before.VersionLine, after.VersionLine, output); err == nil {
		status.logPath = logPath
		status.LogAvailable = true
	} else {
		s.logger.Warn("cli update: log write failed", "provider", plan.Provider, "err", err)
	}

	job.setStatus(plan.Provider, status)

	// 終わったら、その provider のバージョン確認の直近結果も更新後の値で差し替える。
	if afterOK {
		s.cliVersions.refreshCLIVersionResult(after)
	} else if state == cliUpdateStateLatest && beforeOK {
		s.cliVersions.refreshCLIVersionResult(before)
	}
}

// writeCLIUpdateLog writes one provider's update run to
// <LogDir>/cli-updates/<開始時刻>_<provider>.log, returning its path.
func writeCLIUpdateLog(logDir, providerID string, startedAt, finishedAt time.Time, argv []string, exitCode int, timedOut bool, startErr error, versionBefore, versionAfter, output string) (string, error) {
	dir := filepath.Join(logDir, "cli-updates")
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", fmt.Errorf("cli update log dir: %w", err)
	}
	name := fmt.Sprintf("%s_%s.log", startedAt.Format("20060102-150405.000"), safeToken(providerID))
	path := filepath.Join(dir, name)

	var b strings.Builder
	fmt.Fprintf(&b, "provider: %s\n", providerID)
	fmt.Fprintf(&b, "argv: %s\n", strings.Join(argv, " "))
	fmt.Fprintf(&b, "started_at: %s\n", startedAt.UTC().Format(time.RFC3339))
	fmt.Fprintf(&b, "finished_at: %s\n", finishedAt.UTC().Format(time.RFC3339))
	fmt.Fprintf(&b, "exit_code: %d\n", exitCode)
	fmt.Fprintf(&b, "timed_out: %v\n", timedOut)
	if startErr != nil {
		fmt.Fprintf(&b, "start_error: %v\n", startErr)
	}
	fmt.Fprintf(&b, "version_before: %s\n", versionBefore)
	fmt.Fprintf(&b, "version_after: %s\n", versionAfter)
	b.WriteString("--- output ---\n")
	b.WriteString(output)

	if err := os.WriteFile(path, []byte(b.String()), sessionlog.PrivateFileMode); err != nil {
		return "", fmt.Errorf("cli update log write: %w", err)
	}
	return path, nil
}

// refreshCLIVersionResult replaces providerResult's entry in the last known
// 「バージョン確認」batch (or appends one if there is none yet), so the
// screen reflects the update immediately without a fresh checkCLIVersions
// run across every provider.
func (s *cliVersionState) refreshCLIVersionResult(result cliVersionResult) {
	if s == nil {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.last == nil {
		s.last = &cliVersionResponse{CheckedAt: time.Now().UTC().Format(time.RFC3339)}
	}
	for i := range s.last.Results {
		if s.last.Results[i].Provider == result.Provider {
			s.last.Results[i] = result
			return
		}
	}
	s.last.Results = append(s.last.Results, result)
}

// --- HTTP handlers ---------------------------------------------------------

type cliUpdateCreateRequest struct {
	Providers []string `json:"providers"`
	Serial    bool     `json:"serial"`
}

type cliUpdateCreateResponse struct {
	JobID    string                   `json:"job_id"`
	Accepted []string                 `json:"accepted"`
	Excluded []cliUpdateExcludedEntry `json:"excluded"`
}

type cliUpdateJobResponse struct {
	JobID     string                    `json:"job_id"`
	StartedAt string                    `json:"started_at"`
	Serial    bool                      `json:"serial"`
	Providers []cliUpdateProviderStatus `json:"providers"`
	Excluded  []cliUpdateExcludedEntry  `json:"excluded"`
}

type cliUpdateLogResponse struct {
	Content   string `json:"content"`
	Truncated bool   `json:"truncated"`
}

type cliUpdateEligibilityEntry struct {
	Provider        string   `json:"provider"`
	Eligible        bool     `json:"eligible"`
	Reason          string   `json:"reason,omitempty"`
	RunningSessions int      `json:"running_sessions,omitempty"`
	Executable      string   `json:"executable,omitempty"`
	Argv            []string `json:"argv,omitempty"`
}

type cliUpdateEligibilityResponse struct {
	Results []cliUpdateEligibilityEntry `json:"results"`
}

// handleCLIUpdatesCreate is POST /api/cli-updates: starts a new update job
// for the requested providers, excluding any that are not eligible right now
// (with a reason code) instead of failing the whole request.
func (s *Server) handleCLIUpdatesCreate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body cliUpdateCreateRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	ids := normalizeCLIVersionProviderIDs(body.Providers)
	if len(ids) == 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "providers is required")
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}

	var accepted []string
	excluded := make([]cliUpdateExcludedEntry, 0)
	plans := make(map[string]cliUpdatePlan, len(ids))
	for _, id := range ids {
		plan := planCLIUpdate(id, registry)
		if !plan.Eligible {
			excluded = append(excluded, cliUpdateExcludedEntry{Provider: id, Reason: plan.Reason})
			continue
		}
		ok, reason, running := s.beginProviderUpdate(id)
		if !ok {
			excluded = append(excluded, cliUpdateExcludedEntry{Provider: id, Reason: reason, RunningSessions: running})
			continue
		}
		plans[id] = plan
		accepted = append(accepted, id)
	}

	job := newCLIUpdateJob(newCLIUpdateJobID(), body.Serial, accepted, excluded, time.Now())
	s.registerCLIUpdateJob(job)
	if len(accepted) > 0 {
		s.safeGo("cli_update_job_"+job.id, func() {
			s.runCLIUpdateJob(job, registry, plans)
		})
	}

	writeJSON(w, cliUpdateCreateResponse{JobID: job.id, Accepted: accepted, Excluded: excluded})
}

// handleCLIUpdatesItem dispatches GET /api/cli-updates/<job_id> and
// GET /api/cli-updates/<job_id>/<provider>/log.
func (s *Server) handleCLIUpdatesItem(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/cli-updates/")
	parts := strings.Split(rest, "/")
	switch {
	case len(parts) == 1 && parts[0] != "":
		s.handleCLIUpdateJobStatus(w, parts[0])
	case len(parts) == 3 && parts[0] != "" && parts[1] != "" && parts[2] == "log":
		s.handleCLIUpdateJobLog(w, parts[0], parts[1])
	default:
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
	}
}

func (s *Server) handleCLIUpdateJobStatus(w http.ResponseWriter, jobID string) {
	job, ok := s.cliUpdateJobLookup(jobID)
	if !ok {
		writeJSONError(w, http.StatusNotFound, "not_found", "unknown job")
		return
	}
	providers, excluded := job.snapshot()
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, cliUpdateJobResponse{
		JobID:     job.id,
		StartedAt: job.startedAt.UTC().Format(time.RFC3339),
		Serial:    job.serial,
		Providers: providers,
		Excluded:  excluded,
	})
}

func (s *Server) handleCLIUpdateJobLog(w http.ResponseWriter, jobID, providerID string) {
	job, ok := s.cliUpdateJobLookup(jobID)
	if !ok {
		writeJSONError(w, http.StatusNotFound, "not_found", "unknown job")
		return
	}
	logPath, ok := job.logPathFor(providerID)
	if !ok {
		writeJSONError(w, http.StatusNotFound, "not_found", "log not available")
		return
	}
	f, err := os.Open(logPath) // #nosec G703 -- path is generated by writeCLIUpdateLog, never user input
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "log not available")
		return
	}
	defer f.Close()
	buf, err := io.ReadAll(io.LimitReader(f, cliUpdateLogServeMaxBytes+1))
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_failed", "cannot read log")
		return
	}
	truncated := len(buf) > cliUpdateLogServeMaxBytes
	if truncated {
		buf = buf[:cliUpdateLogServeMaxBytes]
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, cliUpdateLogResponse{Content: string(buf), Truncated: truncated})
}

// handleCLIUpdateEligibility is GET /api/cli-update-eligibility: every
// enabled non-shell provider's current "更新できるか" verdict, for the "全部
// 更新（N 件）" button and its confirmation dialog to build from without
// guessing.
func (s *Server) handleCLIUpdateEligibility(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	ids := allCLIVersionProviderIDs(registry)
	results := make([]cliUpdateEligibilityEntry, 0, len(ids))
	for _, id := range ids {
		plan := planCLIUpdate(id, registry)
		entry := cliUpdateEligibilityEntry{
			Provider:   id,
			Eligible:   plan.Eligible,
			Reason:     plan.Reason,
			Executable: plan.Executable,
			Argv:       plan.Argv,
		}
		if entry.Eligible {
			if s.providerUpdating(id) {
				entry.Eligible = false
				entry.Reason = cliUpdateReasonAlreadyUpdating
			} else if n := s.runningSessionCountForProvider(id); n > 0 {
				entry.Eligible = false
				entry.Reason = cliUpdateReasonRunningSessions
				entry.RunningSessions = n
			}
		}
		results = append(results, entry)
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, cliUpdateEligibilityResponse{Results: results})
}

// cleanCliUpdateLogs removes update-run logs (logs/cli-updates/*.log) older
// than cfg.Log.SessionRetentionDays, the same retention the session log
// triplets use (親 plan 決まったこと表: 「session_retention_days（既定 7 日）で
// 自動削除」). A retention of 0 disables cleanup, mirroring cleanSessionLogs.
func (s *Server) cleanCliUpdateLogs() {
	s.cfgMu.Lock()
	days := s.cfg.Log.SessionRetentionDays
	s.cfgMu.Unlock()
	if days <= 0 {
		return
	}
	dir := filepath.Join(s.cfg.Hub.LogDir, "cli-updates")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-time.Duration(days) * 24 * time.Hour)
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}
