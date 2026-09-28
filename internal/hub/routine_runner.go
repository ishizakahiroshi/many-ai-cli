package hub

import (
	"context"
	"errors"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

func (s *Server) startRoutine(id, requestID, trigger string, now time.Time) (routineRun, bool, error) {
	m := s.routines
	m.mu.Lock()
	next := m.copyLocked()
	var definition *routine
	for i := range next.Routines {
		if next.Routines[i].ID == id {
			definition = &next.Routines[i]
			break
		}
	}
	if definition == nil {
		m.mu.Unlock()
		return routineRun{}, false, errRoutineMissing
	}
	if trigger == "schedule" && (!definition.Enabled || definition.Schedule.Kind == "manual" || requestID != "schedule:"+definition.NextRunAt) {
		m.mu.Unlock()
		return routineRun{}, false, errRoutineScheduleChanged
	}
	requestKey := id + "|" + requestID
	for _, run := range next.Runs {
		if run.RoutineID == id && ((requestID != "" && (run.RequestID == requestID || next.Requests[requestKey] == run.ID)) || routineActive(run.Status)) {
			if requestID != "" && next.Requests[requestKey] != run.ID {
				next.Requests[requestKey] = run.ID
				if err := m.commitLocked(next); err != nil {
					m.mu.Unlock()
					return routineRun{}, false, err
				}
			}
			m.mu.Unlock()
			return run, true, nil
		}
	}
	run, err := newRoutineRun(*definition, requestID, trigger, now)
	if err != nil {
		m.mu.Unlock()
		return routineRun{}, false, err
	}
	if trigger == "schedule" {
		due, err := nextRoutineTime(definition.Schedule, now)
		if err != nil {
			m.mu.Unlock()
			return routineRun{}, false, err
		}
		definition.NextRunAt = routineTime(due)
	}
	next.Runs = append(next.Runs, run)
	if requestID != "" {
		next.Requests[requestKey] = run.ID
	}
	err = m.commitLocked(next)
	m.mu.Unlock()
	if err != nil {
		return routineRun{}, false, err
	}
	s.safeGo("routine launch", func() { s.launchRoutine(run) })
	return run, false, nil
}

var errRoutineScheduleChanged = errors.New("routine schedule changed")

func newRoutineRun(def routine, requestID, trigger string, now time.Time) (routineRun, error) {
	id, err := routineID()
	if err != nil {
		return routineRun{}, err
	}
	return routineRun{ID: id, RoutineID: def.ID, RoutineName: def.Name, CWD: def.CWD, Provider: def.Provider, Model: def.Model, Prompt: def.Prompt, Trigger: trigger, Status: "starting", StartedAt: routineTime(now), UpdatedAt: routineTime(now), RequestID: requestID, SessionLabel: "routine-" + id}, nil
}

func (s *Server) launchRoutine(run routineRun) {
	m := s.routines
	launch := m.launch
	if launch == nil {
		launch = s.spawnRoutineSession
	}
	id, err := launch(run)
	m.mu.Lock()
	next := m.copyLocked()
	for i := range next.Runs {
		current := &next.Runs[i]
		if current.ID != run.ID {
			continue
		}
		// A very short CLI may publish its answer before the spawn wait returns.
		if !routineActive(current.Status) {
			break
		}
		current.UpdatedAt = routineTime(time.Now())
		if err != nil {
			current.Status = "failed"
			current.Error = "The AI session could not be started. Check the provider installation and project directory."
			current.FinishedAt = current.UpdatedAt
		} else {
			current.SessionID = id
			current.HubInstanceID = s.instanceID
			current.Status = "running"
		}
	}
	errSave := m.commitLocked(next)
	m.mu.Unlock()
	if errSave != nil {
		s.logger.Warn("routine launch state could not be saved", "err", errSave)
	}
}

func (s *Server) spawnRoutineSession(run routineRun) (int, error) {
	definition := routine{Name: run.RoutineName, CWD: run.CWD, Provider: run.Provider, Model: run.Model, Prompt: run.Prompt, Schedule: routineSchedule{Kind: "manual"}}
	if err := validateRoutine(&definition); err != nil {
		return 0, err
	}
	if s.orchestration == nil {
		return 0, errors.New("session launcher unavailable")
	}
	prompt := run.Prompt + "\n\nWhen this routine finishes or needs user action, end your response with " + string(doneSummaryMarkerOpen) + " a short factual result, including any unfinished work or unperformed checks " + string(doneSummaryMarkerClose) + ". Do not claim success from merely reaching the end of the response."
	launchPrompt := s.screenSpawnLaunchPrompt(run.Provider, "", "", prompt)
	meta := pendingChild{SpawnedAt: time.Now(), PromptAtLaunch: launchPrompt != ""}
	if launchPrompt == "" {
		meta.InitialPrompt = prompt
	}
	s.orchestration.mu.Lock()
	s.orchestration.pending[run.SessionLabel] = meta
	s.orchestration.mu.Unlock()
	id, err := s.spawnWrappedSession(spawnWrappedSpec{Context: s.hubContext(), Provider: run.Provider, CWD: run.CWD, Model: run.Model, Label: run.SessionLabel, InitialPrompt: launchPrompt}, 30*time.Second)
	if err != nil {
		s.orchestration.mu.Lock()
		delete(s.orchestration.pending, run.SessionLabel)
		s.orchestration.mu.Unlock()
	}
	return id, err
}

func (s *Server) runRoutines(ctx context.Context) {
	if s.routines == nil || s.routines.loadErr != nil {
		return
	}
	// Startup explicitly skips missed schedules. It never replays a saved prompt.
	s.tickRoutineSchedules(time.Now(), true)
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			s.refreshRoutineRuns(now)
			s.tickRoutineSchedules(now, false)
		}
	}
}

func (s *Server) tickRoutineSchedules(now time.Time, startup bool) {
	m := s.routines
	m.mu.Lock()
	defs := append([]routine{}, m.data.Routines...)
	m.mu.Unlock()
	for _, def := range defs {
		if !def.Enabled || def.Schedule.Kind == "manual" || def.NextRunAt == "" {
			continue
		}
		due, err := time.Parse(time.RFC3339Nano, def.NextRunAt)
		if err != nil || due.After(now) {
			continue
		}
		// Late ticks after host sleep use the same no-catch-up policy as restart.
		if startup || now.Sub(due) > time.Minute {
			s.skipRoutineSchedule(def, now, "The scheduled time passed while the Hub was unavailable.")
			continue
		}
		_, existing, err := s.startRoutine(def.ID, "schedule:"+def.NextRunAt, "schedule", now)
		if err != nil {
			if errors.Is(err, errRoutineScheduleChanged) || errors.Is(err, errRoutineMissing) {
				continue
			}
			s.logger.Warn("routine schedule could not start", "err", err)
			continue
		}
		if existing {
			s.skipRoutineSchedule(def, now, "The previous run was still active at the scheduled time.")
		}
	}
}

func (s *Server) skipRoutineSchedule(def routine, now time.Time, reason string) {
	m := s.routines
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	for i := range next.Routines {
		item := &next.Routines[i]
		if item.ID != def.ID || item.NextRunAt != def.NextRunAt {
			continue
		}
		due, err := nextRoutineTime(item.Schedule, now)
		if err != nil {
			return
		}
		run, err := newRoutineRun(*item, "schedule:"+item.NextRunAt, "schedule", now)
		if err != nil {
			return
		}
		run.Status = "skipped"
		run.Error = reason
		run.FinishedAt = run.StartedAt
		item.NextRunAt = routineTime(due)
		next.Runs = append(next.Runs, run)
		if err := m.commitLocked(next); err != nil {
			s.logger.Warn("routine skip could not be saved", "err", err)
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

func (s *Server) activeRoutineSession(sessionID int, at time.Time) bool {
	if s.routines == nil || s.routines.loadErr != nil {
		return false
	}
	s.sessionsMu.Lock()
	label := ""
	if ses := s.sessions[sessionID]; ses != nil {
		label = ses.LaunchLabel
	}
	s.sessionsMu.Unlock()
	if label == "" {
		return false
	}
	m := s.routines
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, run := range m.data.Runs {
		started, _ := time.Parse(time.RFC3339Nano, run.StartedAt)
		if run.SessionLabel == label && routineActive(run.Status) && !at.Before(started) {
			return true
		}
	}
	return false
}

func (s *Server) routineRunURL(sessionID int) string {
	if s.routines == nil || s.routines.loadErr != nil {
		return ""
	}
	s.sessionsMu.Lock()
	label := ""
	if ses := s.sessions[sessionID]; ses != nil {
		label = ses.LaunchLabel
	}
	s.sessionsMu.Unlock()
	if label == "" {
		return ""
	}
	m := s.routines
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, run := range m.data.Runs {
		if run.SessionLabel == label {
			return "/?routine_run=" + run.ID
		}
	}
	return ""
}
