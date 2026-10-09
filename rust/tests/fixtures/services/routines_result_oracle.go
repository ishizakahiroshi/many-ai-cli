//go:build ignore

// Fixed Go 21d0bc7935a2c4696fb89ccff2e324157a528c2d result oracle.
// Exact routine records and methods copied from the fixed source; only Server
// dependencies below are synthetic. In-memory state only; no files or processes.
package main

import (
	"encoding/json"
	"errors"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"os"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

type routineSchedule struct {
	Kind     string `json:"kind"`
	Time     string `json:"time,omitempty"`
	Timezone string `json:"timezone,omitempty"`
}

type routine struct {
	ID             string          `json:"id"`
	Name           string          `json:"name"`
	CWD            string          `json:"cwd"`
	Provider       string          `json:"provider"`
	Model          string          `json:"model,omitempty"`
	Prompt         string          `json:"prompt"`
	Enabled        bool            `json:"enabled"`
	Schedule       routineSchedule `json:"schedule"`
	NextRunAt      string          `json:"next_run_at,omitempty"`
	CreatedAt      string          `json:"created_at"`
	UpdatedAt      string          `json:"updated_at"`
	CompletionMode string          `json:"completion_mode"`
}

type routineRun struct {
	ID              string `json:"id"`
	RoutineID       string `json:"routine_id"`
	RoutineName     string `json:"routine_name"`
	CWD             string `json:"cwd"`
	Provider        string `json:"provider"`
	Model           string `json:"model,omitempty"`
	Prompt          string `json:"prompt"`
	Trigger         string `json:"trigger"`
	Status          string `json:"status"`
	SessionID       int    `json:"session_id,omitempty"`
	SessionDBID     int64  `json:"session_db_id,omitempty"`
	HubInstanceID   string `json:"hub_instance_id,omitempty"`
	StartedAt       string `json:"started_at"`
	UpdatedAt       string `json:"updated_at"`
	FinishedAt      string `json:"finished_at,omitempty"`
	Summary         string `json:"summary,omitempty"`
	Result          string `json:"result,omitempty"`
	ResultTruncated bool   `json:"result_truncated,omitempty"`
	ResultAvailable bool   `json:"result_available"`
	Error           string `json:"error,omitempty"`
	RequestID       string `json:"request_id,omitempty"`
	SessionLabel    string `json:"session_label"`
}

type routineFile struct {
	Version  int               `json:"version"`
	Routines []routine         `json:"routines"`
	Runs     []routineRun      `json:"runs"`
	Requests map[string]string `json:"requests,omitempty"`
}

type routineManager struct {
	mu             sync.Mutex
	data           routineFile
	path           string
	loadErr        error
	write          func(string, routineFile) error
	missingSince   map[string]time.Time
	pendingResults map[string]routineRun
	// Tests replace only the process launch, never the admission or persistence path.
	launch func(routineRun) (int, error)
}

type observedSession struct {
	LaunchLabel     string
	State           string
	pendingApproval *bool
	Activity        proto.SessionActivity
	lastOutputAt    time.Time
}
type savedOverview struct{ ID int64 }
type storage interface {
	SessionOverviewByLiveSession(int) (savedOverview, error)
}
type quietLogger struct{}

func (quietLogger) Warn(string, ...any) {}

type Server struct {
	routines     *routineManager
	sessionsMu   sync.Mutex
	sessions     map[int]*observedSession
	sessionStore storage
	instanceID   string
	logger       quietLogger
}

const doneSummaryMaxRunes = 320

func (m *routineManager) commitLocked(next routineFile) error {
	if m.loadErr != nil {
		return m.loadErr
	}
	if err := m.write(m.path, next); err != nil {
		return err
	}
	m.data = next
	return nil
}

func (m *routineManager) copyLocked() routineFile {
	next := routineFile{Version: 1, Routines: append([]routine{}, m.data.Routines...), Runs: append([]routineRun{}, m.data.Runs...), Requests: map[string]string{}}
	for key, value := range m.data.Requests {
		next.Requests[key] = value
	}
	return next
}

func routineActive(status string) bool {
	return status == "starting" || status == "running" || status == "waiting"
}

func (s *Server) recordRoutineDone(summary proto.DoneSummary) {
	if s.routines == nil || s.routines.loadErr != nil || summary.Fallback {
		return
	}
	s.sessionsMu.Lock()
	label := ""
	if ses := s.sessions[summary.SessionID]; ses != nil {
		label = ses.LaunchLabel
	}
	s.sessionsMu.Unlock()
	if label == "" {
		return
	}
	m := s.routines
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	for i := range next.Runs {
		run := &next.Runs[i]
		if run.SessionLabel != label || !routineActive(run.Status) {
			continue
		}
		run.SessionID = summary.SessionID
		run.HubInstanceID = s.instanceID
		if s.sessionStore != nil {
			if saved, err := s.sessionStore.SessionOverviewByLiveSession(summary.SessionID); err == nil {
				run.SessionDBID = saved.ID
			}
		}
		run.Status = "finished"
		run.FinishedAt = summary.At
		run.UpdatedAt = summary.At
		run.Error = ""
		result := sessionlog.MaskSecrets(summary.Text)
		run.Summary = truncateDoneSummary(result)
		run.Result = truncateUTF8Bytes(result, 256*1024)
		run.ResultTruncated = len(result) > len(run.Result)
		run.ResultAvailable = result != ""
		if err := m.commitLocked(next); err != nil {
			if m.pendingResults == nil {
				m.pendingResults = map[string]routineRun{}
			}
			m.pendingResults[run.ID] = *run
			s.logger.Warn("routine result could not be saved", "err", err)
		}
		return
	}
}

func (s *Server) refreshRoutineRuns(now time.Time) {
	m := s.routines
	// Never hold the session lock while accessing disk or the routine lock.
	type observed struct {
		id         int
		state      string
		waiting    bool
		lastOutput time.Time
	}
	observations := map[string]observed{}
	s.sessionsMu.Lock()
	for id, ses := range s.sessions {
		// Key by LaunchLabel (fixed at register time), not the mutable display
		// Label: a card rename must not orphan the routine's run tracking
		// (plan_session-card-label-edit.md C1).
		observations[ses.LaunchLabel] = observed{id: id, state: ses.State, waiting: ses.pendingApproval != nil || ses.Activity.AwaitingUser, lastOutput: ses.lastOutputAt}
	}
	s.sessionsMu.Unlock()
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	changed := false
	for i := range next.Runs {
		run := &next.Runs[i]
		if pending, ok := m.pendingResults[run.ID]; ok && routineActive(run.Status) {
			*run = pending
			changed = true
		}
		if !routineActive(run.Status) {
			continue
		}
		before := *run
		obs, ok := observations[run.SessionLabel]
		if ok && obs.state != "disconnected" {
			delete(m.missingSince, run.ID)
			run.SessionID = obs.id
			run.HubInstanceID = s.instanceID
			if s.sessionStore != nil && run.SessionDBID == 0 {
				if saved, err := s.sessionStore.SessionOverviewByLiveSession(obs.id); err == nil {
					run.SessionDBID = saved.ID
				}
			}
			switch {
			case obs.state == "completed" || obs.state == "error":
				run.Status = "finished"
				run.FinishedAt = routineTime(now)
				if obs.state == "error" {
					run.Status = "failed"
					run.Error = "The AI session ended with an error."
				}
			case obs.waiting:
				run.Status = "waiting"
				run.Error = ""
			case obs.state == "standby" && !obs.lastOutput.IsZero() && now.Sub(obs.lastOutput) > 30*time.Second:
				run.Status = "waiting"
				run.Error = "No completion result has been received. Open the session to confirm the outcome."
			default:
				run.Status = "running"
				run.Error = ""
			}
		} else {
			missing := m.missingSince[run.ID]
			if missing.IsZero() {
				m.missingSince[run.ID] = now
				continue
			}
			if now.Sub(missing) > 90*time.Second {
				run.Status = "interrupted"
				run.FinishedAt = routineTime(now)
				run.Error = "The original session is unavailable. Its result has not been confirmed."
			}
		}
		if *run != before {
			run.UpdatedAt = routineTime(now)
			changed = true
		}
	}
	if changed {
		if err := m.commitLocked(next); err != nil {
			s.logger.Warn("routine status could not be saved", "err", err)
		} else {
			clear(m.pendingResults)
		}
	}
}

func truncateDoneSummary(text string) string {
	text = strings.Join(strings.Fields(strings.TrimSpace(text)), " ")
	if utf8.RuneCountInString(text) <= doneSummaryMaxRunes {
		return text
	}
	runes := []rune(text)
	return strings.TrimSpace(string(runes[:doneSummaryMaxRunes])) + "…"
}

func truncateUTF8Bytes(s string, max int) string {
	if max <= 0 {
		return ""
	}
	if len(s) <= max {
		return s
	}
	cut := 0
	for i := range s {
		if i > max {
			break
		}
		cut = i
	}
	if cut <= 0 {
		return ""
	}
	return strings.TrimSpace(s[:cut])
}

func routineTime(t time.Time) string { return t.UTC().Format(time.RFC3339Nano) }
func main() {
	fail := true
	m := &routineManager{data: routineFile{Version: 1, Routines: []routine{}, Runs: []routineRun{{ID: "run", RoutineID: "definition", Status: "starting", SessionLabel: "routine-run"}}}, missingSince: map[string]time.Time{}, pendingResults: map[string]routineRun{}, write: func(string, routineFile) error {
		if fail {
			return errors.New("synthetic disk failure")
		}
		return nil
	}}
	s := &Server{routines: m, sessions: map[int]*observedSession{42: {LaunchLabel: "routine-run", State: "running"}}, instanceID: "synthetic-hub"}
	type Receipt struct {
		Stage   string     `json:"stage"`
		Run     routineRun `json:"run"`
		Pending string     `json:"pending"`
	}
	out := []Receipt{}
	capture := func(stage string) { out = append(out, Receipt{stage, m.data.Runs[0], m.pendingResults["run"].Result}) }
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "first pending", At: "2023-11-14T22:14:00Z"})
	capture("first failed save")
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "second pending", At: "2023-11-14T22:15:00Z"})
	capture("second failed save")
	fail = false
	s.refreshRoutineRuns(time.Unix(1700000100, 0))
	capture("retry persisted")
	s.recordRoutineDone(proto.DoneSummary{SessionID: 42, Text: "later committed result must not replace", At: "2023-11-14T22:16:00Z"})
	capture("after committed result")
	json.NewEncoder(os.Stdout).Encode(out)
}
