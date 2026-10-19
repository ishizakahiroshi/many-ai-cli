package hub

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
	_ "time/tzdata" // Scheduled routines also work on hosts without an OS zone database.

	"many-ai-cli/internal/config"
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

func newRoutineManager(path string) *routineManager {
	m := &routineManager{path: path, write: writeRoutineFile, missingSince: map[string]time.Time{}, pendingResults: map[string]routineRun{}, data: routineFile{Version: 1, Routines: []routine{}, Runs: []routineRun{}}}
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return m
	}
	if err != nil {
		m.loadErr = err
		return m
	}
	if err = json.Unmarshal(b, &m.data); err != nil {
		m.loadErr = err
		return m
	}
	if m.data.Version != 1 {
		m.loadErr = errors.New("unsupported routine store version")
	}
	return m
}

func (s *Server) initRoutines() {
	dir, err := config.Dir()
	if err != nil {
		s.routines = &routineManager{loadErr: err}
		return
	}
	s.routines = newRoutineManager(filepath.Join(dir, "routines.json"))
	if s.routines.loadErr != nil {
		s.logger.Warn("routine store unavailable", "err", s.routines.loadErr)
	}
}

// commitLocked publishes memory only after the complete new file has been saved.
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

func writeRoutineFile(path string, data routineFile) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	b, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), "routines-*.tmp")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err = f.Write(b); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Sync(); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	if err = os.Chmod(f.Name(), 0o600); err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}

func routineID() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(b[:]), nil
}

func routineActive(status string) bool {
	return status == "starting" || status == "running" || status == "waiting"
}

func validateRoutine(r *routine) error {
	r.Name = strings.TrimSpace(r.Name)
	r.CWD = strings.TrimSpace(r.CWD)
	r.Prompt = strings.TrimSpace(r.Prompt)
	if r.Name == "" || len(r.Name) > 200 {
		return errors.New("name must contain 1 to 200 bytes")
	}
	if r.Prompt == "" || len(r.Prompt) > spawnInitialPromptMaxLen || sanitizeSpawnInitialPrompt(r.Prompt) != r.Prompt {
		return errors.New("prompt is empty, too long, or contains unsupported control characters")
	}
	if !validOrchestrationProvider(r.Provider) || r.Provider == "shell" {
		return errors.New("unsupported AI provider")
	}
	r.CompletionMode = "marker"
	if r.Provider == "codex" {
		r.CompletionMode = "native_or_marker"
	}
	if strings.HasPrefix(r.Model, "-") || !spawnValidModelLabel(r.Model) {
		return errors.New("invalid model")
	}
	if !filepath.IsAbs(r.CWD) || spawnCwdTooBroad(r.CWD) {
		return errors.New("cwd must be an absolute project directory")
	}
	fi, err := os.Stat(r.CWD)
	if err != nil || !fi.IsDir() {
		return errors.New("cwd does not exist or is not a directory")
	}
	if r.Schedule.Kind == "" {
		r.Schedule.Kind = "manual"
	}
	_, err = nextRoutineTime(r.Schedule, time.Now())
	return err
}

// A civil date is considered once, so the repeated DST hour never runs twice.
// A nonexistent wall time (spring forward) is skipped, not silently moved.
func nextRoutineTime(schedule routineSchedule, after time.Time) (time.Time, error) {
	if schedule.Kind == "manual" {
		return time.Time{}, nil
	}
	if schedule.Kind != "daily" && schedule.Kind != "weekdays" {
		return time.Time{}, errors.New("schedule kind must be manual, daily, or weekdays")
	}
	clock, err := time.Parse("15:04", schedule.Time)
	if err != nil {
		return time.Time{}, errors.New("schedule time must be HH:MM")
	}
	if schedule.Timezone == "" || schedule.Timezone == "Local" {
		return time.Time{}, errors.New("schedule timezone is required")
	}
	loc, err := time.LoadLocation(schedule.Timezone)
	if err != nil {
		return time.Time{}, errors.New("unknown schedule timezone")
	}
	local := after.In(loc)
	for offset := 0; offset < 9; offset++ {
		date := time.Date(local.Year(), local.Month(), local.Day()+offset, 12, 0, 0, 0, loc)
		if schedule.Kind == "weekdays" && (date.Weekday() == time.Saturday || date.Weekday() == time.Sunday) {
			continue
		}
		next := time.Date(date.Year(), date.Month(), date.Day(), clock.Hour(), clock.Minute(), 0, 0, loc)
		if next.Hour() != clock.Hour() || next.Minute() != clock.Minute() || next.Day() != date.Day() {
			continue
		}
		if next.After(after) {
			return next, nil
		}
	}
	return time.Time{}, fmt.Errorf("no next schedule time")
}

func routineTime(t time.Time) string { return t.UTC().Format(time.RFC3339Nano) }
