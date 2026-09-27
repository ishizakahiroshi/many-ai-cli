package hub

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func routineTestServer(t *testing.T) *Server {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "routine-test-token"
	s.instanceID = "test-hub-instance"
	s.routines = newRoutineManager(filepath.Join(t.TempDir(), "routines.json"))
	s.routines.data.Routines = []routine{{ID: "saved", Name: "Review changes", CWD: t.TempDir(), Provider: "codex", Prompt: "Summarize the latest changes.", Schedule: routineSchedule{Kind: "manual"}, CreatedAt: "2026-01-01T00:00:00Z", UpdatedAt: "2026-01-01T00:00:00Z"}}
	s.routines.launch = func(r routineRun) (int, error) { return 42, nil }
	return s
}

func routineRequest(s *Server, method, path string, body any) *httptest.ResponseRecorder {
	encoded, _ := json.Marshal(body)
	r := prefsAuthReq(method, path, encoded, "application/json")
	w := httptest.NewRecorder()
	if len(path) >= 17 && path[:17] == "/api/routine-runs" {
		s.handleRoutineRuns(w, r)
	} else {
		s.handleRoutines(w, r)
	}
	return w
}

func waitRoutineStatus(t *testing.T, s *Server, id, status string) routineRun {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		s.routines.mu.Lock()
		for _, run := range s.routines.data.Runs {
			if run.ID == id && run.Status == status {
				s.routines.mu.Unlock()
				return run
			}
		}
		s.routines.mu.Unlock()
		select {
		case <-deadline:
			t.Fatalf("run %s did not reach %s", id, status)
		case <-time.After(time.Millisecond):
		}
	}
}

func TestRoutineConcurrentAdmissionAndRetryRemainOneRun(t *testing.T) {
	s := routineTestServer(t)
	var calls atomic.Int32
	s.routines.launch = func(r routineRun) (int, error) { calls.Add(1); return 42, nil }
	var wg sync.WaitGroup
	results := make(chan routineRun, 20)
	for i := 0; i < 20; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			run, _, err := s.startRoutine("saved", "same-click", "manual", time.Now())
			if err != nil {
				t.Error(err)
				return
			}
			results <- run
		}()
	}
	wg.Wait()
	close(results)
	var id string
	for run := range results {
		if id != "" && run.ID != id {
			t.Fatal("duplicate run")
		}
		id = run.ID
	}
	waitRoutineStatus(t, s, id, "running")
	if calls.Load() != 1 {
		t.Fatalf("launches = %d", calls.Load())
	}
	// A second tap while active has its own durable receipt, even after completion.
	if _, _, err := s.startRoutine("saved", "second-click", "manual", time.Now()); err != nil {
		t.Fatal(err)
	}
	s.sessions[42] = &session{ID: 42, Label: "routine-" + id}
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "Changes reviewed. Tests were not run.", At: time.Now().Format(time.RFC3339)})
	for _, key := range []string{"same-click", "second-click"} {
		run, existing, err := s.startRoutine("saved", key, "manual", time.Now())
		if err != nil || !existing || run.ID != id {
			t.Fatalf("retry changed execution: %+v %v", run, err)
		}
	}
	if calls.Load() != 1 {
		t.Fatal("retry launched again")
	}
}

func TestRoutineSaveFailureDoesNotLaunchOrChangeMemory(t *testing.T) {
	s := routineTestServer(t)
	var calls atomic.Int32
	s.routines.launch = func(r routineRun) (int, error) { calls.Add(1); return 42, nil }
	s.routines.write = func(string, routineFile) error { return errors.New("disk unavailable") }
	if _, _, err := s.startRoutine("saved", "click", "manual", time.Now()); err == nil {
		t.Fatal("save should fail")
	}
	if len(s.routines.data.Runs) != 0 || calls.Load() != 0 {
		t.Fatal("unsaved run changed memory or launched")
	}
	def := s.routines.data.Routines[0]
	def.Name = "Changed"
	w := routineRequest(s, http.MethodPut, "/api/routines/saved?token=routine-test-token", def)
	if w.Code != 500 || s.routines.data.Routines[0].Name == "Changed" {
		t.Fatalf("failed save changed routine: %d", w.Code)
	}
}

func TestRoutineScheduleDSTAndWeekdays(t *testing.T) {
	tests := []struct{ name, kind, clock, zone, after, want string }{
		{"weekday", "weekdays", "09:00", "Asia/Tokyo", "2026-09-25T09:01:00+09:00", "2026-09-28T09:00:00+09:00"},
		{"spring gap", "daily", "02:30", "America/New_York", "2026-03-08T00:00:00-05:00", "2026-03-09T02:30:00-04:00"},
		{"fall repeated once", "daily", "01:30", "America/New_York", "2026-11-01T01:31:00-04:00", "2026-11-02T01:30:00-05:00"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			after, _ := time.Parse(time.RFC3339, tt.after)
			want, _ := time.Parse(time.RFC3339, tt.want)
			got, err := nextRoutineTime(routineSchedule{Kind: tt.kind, Time: tt.clock, Timezone: tt.zone}, after)
			if err != nil || !got.Equal(want) {
				t.Fatalf("got %s, %v; want %s", got, err, want)
			}
		})
	}
	for _, sch := range []routineSchedule{{Kind: "weekly"}, {Kind: "daily", Time: "25:00", Timezone: "UTC"}, {Kind: "daily", Time: "09:00", Timezone: "not/a-zone"}} {
		if _, err := nextRoutineTime(sch, time.Now()); err == nil {
			t.Fatalf("invalid schedule accepted: %+v", sch)
		}
	}
}

func TestRoutineRestartSkipsMissedTimeAndRetainsHistory(t *testing.T) {
	s := routineTestServer(t)
	now := time.Date(2026, 9, 28, 10, 0, 0, 0, time.UTC)
	d := &s.routines.data.Routines[0]
	d.Enabled = true
	d.Schedule = routineSchedule{Kind: "daily", Time: "09:00", Timezone: "UTC"}
	d.NextRunAt = "2026-09-28T09:00:00Z"
	s.routines.launch = func(r routineRun) (int, error) { t.Error("missed schedule launched"); return 0, nil }
	s.tickRoutineSchedules(now, true)
	if len(s.routines.data.Runs) != 1 || s.routines.data.Runs[0].Status != "skipped" {
		t.Fatal("missing skipped run")
	}
	loaded := newRoutineManager(s.routines.path)
	if loaded.loadErr != nil || len(loaded.data.Runs) != 1 || loaded.data.Routines[0].NextRunAt != "2026-09-29T09:00:00Z" {
		t.Fatalf("restart state = %+v", loaded.data)
	}
	s.tickRoutineSchedules(now.Add(time.Minute), false)
	if len(s.routines.data.Runs) != 1 {
		t.Fatal("missed time recorded more than once")
	}
}

func TestRoutineScheduleDisabledDuringTickDoesNotLaunch(t *testing.T) {
	s := routineTestServer(t)
	s.routines.data.Routines[0].NextRunAt = "2026-09-28T09:00:00Z"
	_, _, err := s.startRoutine("saved", "schedule:2026-09-28T09:00:00Z", "schedule", time.Now())
	if !errors.Is(err, errRoutineScheduleChanged) || len(s.routines.data.Runs) != 0 {
		t.Fatal("disabled schedule launched")
	}
}

func TestRoutineResultFrozenAndDeletedDefinitionRetainsRun(t *testing.T) {
	s := routineTestServer(t)
	run, _, err := s.startRoutine("saved", "click", "manual", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	waitRoutineStatus(t, s, run.ID, "running")
	s.sessions[42] = &session{ID: 42, Label: run.SessionLabel}
	w := routineRequest(s, http.MethodDelete, "/api/routines/saved?token=routine-test-token", nil)
	if w.Code != 409 {
		t.Fatalf("active delete = %d", w.Code)
	}
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "First result; validation is incomplete.", At: time.Now().Format(time.RFC3339)})
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "Later unrelated conversation.", At: time.Now().Format(time.RFC3339)})
	finished := waitRoutineStatus(t, s, run.ID, "finished")
	if finished.Summary != "First result; validation is incomplete." || !finished.ResultAvailable {
		t.Fatal("run result was replaced")
	}
	if got := s.routineRunURL(42); got != "/?routine_run="+run.ID {
		t.Fatalf("notification URL = %s", got)
	}
	w = routineRequest(s, http.MethodDelete, "/api/routines/saved?token=routine-test-token", nil)
	if w.Code != 200 || len(s.routines.data.Runs) != 1 {
		t.Fatal("delete lost history")
	}
	if loaded := newRoutineManager(s.routines.path); loaded.loadErr != nil || len(loaded.data.Runs) != 1 {
		t.Fatal("history not persistent")
	}
	// Session number reuse must never expose a different conversation as this run.
	s.sessions[42] = &session{ID: 42, Label: "unrelated"}
	if s.routineRunURL(42) != "" {
		t.Fatal("notification followed reused live ID")
	}
}

func TestRoutineRestartRebindsLabelAndUnknownCompletion(t *testing.T) {
	s := routineTestServer(t)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", time.Now())
	run.SessionID = 42
	run.Status = "running"
	run.HubInstanceID = "old-hub"
	s.routines.data.Runs = []routineRun{run}
	s.sessions[42] = &session{ID: 42, Label: "unrelated", State: "completed"}
	s.sessions[55] = &session{ID: 55, Label: run.SessionLabel, State: "waiting", Activity: SessionActivity{AwaitingUser: true}}
	s.refreshRoutineRuns(time.Now())
	got := s.routines.data.Runs[0]
	if got.SessionID != 55 || got.Status != "waiting" || got.HubInstanceID != s.instanceID {
		t.Fatalf("rebind = %+v", got)
	}
	s.recordRoutineDone(proto.DoneSummary{SessionID: 55, Text: "Output is quiet.", Fallback: true, At: time.Now().Format(time.RFC3339)})
	got = s.routines.data.Runs[0]
	if got.Status != "waiting" || got.ResultAvailable || got.Summary != "" {
		t.Fatalf("unknown result claimed as an answer: %+v", got)
	}
	s.sessions[55].State = "completed"
	s.sessions[55].Activity = SessionActivity{}
	s.refreshRoutineRuns(time.Now())
	got = s.routines.data.Runs[0]
	if got.Status != "finished" || got.ResultAvailable {
		t.Fatalf("process end should be finished without result: %+v", got)
	}
}

func TestRoutineMissingAfterRestartNeverReplaysPrompt(t *testing.T) {
	s := routineTestServer(t)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", time.Now())
	s.routines.data.Runs = []routineRun{run}
	now := time.Now()
	s.refreshRoutineRuns(now)
	s.refreshRoutineRuns(now.Add(91 * time.Second))
	if got := s.routines.data.Runs[0]; got.Status != "interrupted" || got.ResultAvailable {
		t.Fatalf("missing run = %+v", got)
	}
}

func TestRoutineHTTPAuthValidationAndStaleEdit(t *testing.T) {
	s := routineTestServer(t)
	if w := routineRequest(s, http.MethodGet, "/api/routines", nil); w.Code != 401 {
		t.Fatalf("unauthenticated status=%d", w.Code)
	}
	if w := routineRequest(s, http.MethodGet, "/api/routines?token=routine-test-token", nil); w.Code != 200 {
		t.Fatalf("list status=%d", w.Code)
	}
	def := s.routines.data.Routines[0]
	def.UpdatedAt = "old-revision"
	if w := routineRequest(s, http.MethodPut, "/api/routines/saved?token=routine-test-token", def); w.Code != 409 {
		t.Fatalf("stale status=%d body=%s", w.Code, w.Body.String())
	}
	def.UpdatedAt = ""
	def.CWD = filepath.VolumeName(def.CWD) + string(filepath.Separator)
	if w := routineRequest(s, http.MethodPost, "/api/routines?token=routine-test-token", def); w.Code != 400 {
		t.Fatalf("broad cwd status=%d", w.Code)
	}
	if w := routineRequest(s, http.MethodPost, "/api/routines/saved/runs?token=routine-test-token", map[string]string{}); w.Code != 400 {
		t.Fatalf("missing request status=%d", w.Code)
	}
}

func TestRoutineCorruptStoreIsNotOverwritten(t *testing.T) {
	s := routineTestServer(t)
	if err := os.WriteFile(s.routines.path, []byte("broken"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.routines = newRoutineManager(s.routines.path)
	w := routineRequest(s, http.MethodGet, "/api/routines?token=routine-test-token", nil)
	if w.Code != 503 {
		t.Fatalf("corrupt status=%d", w.Code)
	}
	b, _ := os.ReadFile(s.routines.path)
	if string(b) != "broken" {
		t.Fatal("corrupt file overwritten")
	}
}

func TestRoutineNativeReadOnlyCompletionPreservesFullResult(t *testing.T) {
	s := routineTestServer(t)
	now := time.Now().Add(-time.Minute)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", now)
	s.routines.data.Runs = []routineRun{run}
	s.sessions[42] = &session{ID: 42, Provider: "codex", Label: run.SessionLabel, State: "running"}
	text := strings.Repeat("Checked existing files. No files were changed. ", 30)
	s.handleCodexTaskCompletion(42, codexTaskCompletion{TurnID: "first-turn", At: time.Now().Format(time.RFC3339Nano), LastAgentMessage: text})
	got := s.routines.data.Runs[0]
	if got.Status != "finished" || !got.ResultAvailable || got.Result != text || len(got.Summary) >= len(text) {
		t.Fatalf("native readonly result not preserved: %+v", got)
	}
	if got.ResultTruncated {
		t.Fatal("short answer falsely marked truncated")
	}
}

func TestRoutineNativeEmptyResultDoesNotInventAnswer(t *testing.T) {
	s := routineTestServer(t)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", time.Now().Add(-time.Minute))
	s.routines.data.Runs = []routineRun{run}
	s.sessions[42] = &session{ID: 42, Provider: "codex", Label: run.SessionLabel, State: "running"}
	s.handleCodexTaskCompletion(42, codexTaskCompletion{TurnID: "first-turn", At: time.Now().Format(time.RFC3339Nano)})
	got := s.routines.data.Runs[0]
	if got.Status != "finished" || got.ResultAvailable || got.Result != "" || got.Summary != "" {
		t.Fatalf("invented result: %+v", got)
	}
}

func TestRoutineIdleWithoutCompletionNeedsConfirmation(t *testing.T) {
	s := routineTestServer(t)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", time.Now().Add(-time.Minute))
	s.routines.data.Runs = []routineRun{run}
	s.sessions[42] = &session{ID: 42, Label: run.SessionLabel, State: "standby", lastOutputAt: time.Now().Add(-time.Minute)}
	s.refreshRoutineRuns(time.Now())
	got := s.routines.data.Runs[0]
	if got.Status != "waiting" || got.ResultAvailable || got.Error == "" || got.FinishedAt != "" {
		t.Fatalf("idle was misclassified: %+v", got)
	}
}

func TestRoutineHTTPCreateRunAndReadStableDetail(t *testing.T) {
	s := routineTestServer(t)
	def := s.routines.data.Routines[0]
	def.Name = "Morning review"
	w := routineRequest(s, http.MethodPost, "/api/routines?token=routine-test-token", def)
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Routine routine `json:"routine"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.Routine.ID == "saved" || created.Routine.CompletionMode != "native_or_marker" {
		t.Fatalf("create ignored server fields: %+v", created)
	}
	w = routineRequest(s, http.MethodPost, "/api/routines/"+created.Routine.ID+"/runs?token=routine-test-token", map[string]string{"request_id": "mobile-click"})
	if w.Code != 200 {
		t.Fatalf("run=%d %s", w.Code, w.Body.String())
	}
	var started struct {
		Run routineRun `json:"run"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &started); err != nil {
		t.Fatal(err)
	}
	waitRoutineStatus(t, s, started.Run.ID, "running")
	w = routineRequest(s, http.MethodGet, "/api/routine-runs/"+started.Run.ID+"?token=routine-test-token", nil)
	if w.Code != 200 {
		t.Fatalf("detail=%d %s", w.Code, w.Body.String())
	}
	var detail struct {
		Run routineRun `json:"run"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &detail); err != nil {
		t.Fatal(err)
	}
	if detail.Run.ID != started.Run.ID || detail.Run.Prompt != def.Prompt || detail.Run.SessionLabel == "" {
		t.Fatalf("run identity lost: %+v", detail)
	}
	w = routineRequest(s, http.MethodGet, "/api/routine-runs?routine_id="+created.Routine.ID+"&token=routine-test-token", nil)
	if w.Code != 200 || !strings.Contains(w.Body.String(), started.Run.ID) {
		t.Fatalf("history=%d %s", w.Code, w.Body.String())
	}
}

func TestRoutineFailedLaunchAndPersistenceRestartKeepIdempotency(t *testing.T) {
	s := routineTestServer(t)
	s.routines.launch = func(r routineRun) (int, error) { return 0, errors.New("synthetic launch failure") }
	run, _, err := s.startRoutine("saved", "mobile-click", "manual", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	failed := waitRoutineStatus(t, s, run.ID, "failed")
	if failed.Error == "" || failed.ResultAvailable {
		t.Fatalf("failed launch = %+v", failed)
	}
	s.routines = newRoutineManager(s.routines.path)
	s.routines.launch = func(r routineRun) (int, error) { t.Error("retry after restart launched again"); return 0, nil }
	retried, existing, err := s.startRoutine("saved", "mobile-click", "manual", time.Now())
	if err != nil || !existing || retried.ID != run.ID || retried.Status != "failed" {
		t.Fatalf("retry lost original failure: %+v %v", retried, err)
	}
}

func TestRoutineResultSaveFailureRetriesWithoutChangingRun(t *testing.T) {
	s := routineTestServer(t)
	run, _ := newRoutineRun(s.routines.data.Routines[0], "click", "manual", time.Now())
	s.routines.data.Runs = []routineRun{run}
	s.sessions[42] = &session{ID: 42, Label: run.SessionLabel, State: "standby"}
	s.routines.write = func(string, routineFile) error { return errors.New("temporary write failure") }
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "A fixed result.", At: time.Now().Format(time.RFC3339Nano)})
	if s.routines.data.Runs[0].Status != "starting" || len(s.routines.pendingResults) != 1 {
		t.Fatal("failed result was published or dropped")
	}
	s.routines.write = writeRoutineFile
	s.refreshRoutineRuns(time.Now())
	got := s.routines.data.Runs[0]
	if got.ID != run.ID || got.Status != "finished" || got.Result != "A fixed result." || len(s.routines.pendingResults) != 0 {
		t.Fatalf("result recovery lost identity: %+v", got)
	}
}
