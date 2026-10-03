package dotsbridge

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"time"
)

//go:embed testdata/color.json
var colorFixture []byte

//go:embed testdata/recovery.json
var recoveryFixture []byte

type fixture struct {
	Case       string `json:"case"`
	Question   string `json:"question"`
	Answer     string `json:"answer"`
	Completion string `json:"completion"`
}
type TrialReport struct {
	Offline           bool     `json:"offline"`
	Candidate         Route    `json:"candidate"`
	Case              string   `json:"case"`
	JobID             string   `json:"job_id"`
	State             string   `json:"state"`
	Launches          int      `json:"launches"`
	LogicalDeliveries int      `json:"logical_deliveries"`
	Stages            []string `json:"stages"`
	Limit             string   `json:"limit"`
}

func event(j Job, phase string, sequence int64, payload string, source Route) Envelope {
	e := Envelope{CampaignID: j.Campaign, CaseID: j.Case, JobID: j.ID, LogicalMessageID: phase + ":" + j.ID, Phase: phase, QuestionID: "color-question", Sequence: sequence, Source: source, SourceEventID: phase + ":" + j.ID, Sender: "fake-dot", Origin: "dot", Thread: j.thread(source), RouteEpoch: j.RouteEpoch, ExecutionEpoch: j.ExecutionEpoch, Payload: payload, ReceivedAt: j.CreatedAt, ExpiresAt: j.ExpiresAt}
	e.PayloadHash = e.Hash()
	return e
}
func (w *Worker) mockAnswer(ctx context.Context, h *FakeHub, i Intent, now time.Time) ([]byte, error) {
	cap, err := h.consumer(i.Run.ID)
	if err != nil {
		return nil, err
	}
	input, err := w.Acquire(ctx, i.JobID, cap, now)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(Answer{input, "青"})
	if err != nil {
		return nil, err
	}
	if err = h.Finish(i.Run.ID, true); err != nil {
		return nil, err
	}
	return raw, nil
}
func RunMock(s *Store, campaign string, route Route, caseID string, now time.Time) (TrialReport, error) {
	report := TrialReport{Offline: true, Candidate: route, Case: caseID, Limit: "fake only; real MCP/Slack/dots/Hub/AI, UX and cost unverified"}
	rawFixture := colorFixture
	if caseID == "recovery" {
		rawFixture = recoveryFixture
	}
	var f fixture
	if err := json.Unmarshal(rawFixture, &f); err != nil {
		return report, err
	}
	if f.Case != caseID {
		return report, ErrHeld
	}
	j, err := s.Create(campaign, route, caseID, now)
	if err != nil {
		return report, err
	}
	report.JobID = j.ID
	// A rerun never creates another run. Partial trials require explicit reconciliation.
	if j.State != "created" {
		return report, ErrHeld
	}
	report.Stages = append(report.Stages, "request_saved", "fake_dot_question")
	transport := NewFakeTransport(route)
	q := event(j, "question", 1, f.Question, route)
	if _, err = transport.Receive(s, q, now); err != nil {
		return report, err
	}
	report.Stages = append(report.Stages, "question_saved")
	other := RouteS
	if route == RouteS {
		other = RouteM
	}
	if caseID == "recovery" {
		dup := q
		dup.Source = other
		dup.Thread = j.thread(other)
		dup.SourceEventID = "duplicate:" + j.ID
		if _, err = NewFakeTransport(other).Receive(s, dup, now); err != nil {
			return report, err
		}
		blocked := event(j, "blocked", 2, "合成回答待ち", route)
		if _, err = transport.Receive(s, blocked, now); err != nil {
			return report, err
		}
	}
	l, err := s.Claim(j.ID, "mock-worker", now)
	if err != nil {
		return report, err
	}
	h := NewFakeHub("synthetic-hub")
	w := Worker{s, h, h, "synthetic-routine", "synthetic-hub", nil}
	i, err := w.Launch(context.Background(), l, now)
	if err != nil {
		return report, err
	}
	report.Stages = append(report.Stages, "fake_hub_run")
	if caseID == "recovery" {
		if err = s.SwitchRoute(j.ID, other, now); err != nil {
			return report, err
		}
		transport = NewFakeTransport(other)
	}
	answer, err := w.mockAnswer(context.Background(), h, i, now)
	if err != nil {
		return report, err
	}
	if err = w.Collect(context.Background(), l, answer, now); err != nil {
		return report, err
	}
	report.Stages = append(report.Stages, "answer_saved")
	j, err = s.Job(j.ID)
	if err != nil {
		return report, err
	}
	o, err := s.BeginDelivery(j.ID, j.Route, j.RouteEpoch, now)
	if err != nil {
		return report, err
	}
	if err = s.FinishDelivery(o, transport.Send(o), 0, now); err != nil {
		return report, err
	}
	report.Stages = append(report.Stages, "fake_dot_same_job_continued")
	complete := event(j, "completed", 3, f.Completion, j.Route)
	if _, err = transport.Receive(s, complete, now); err != nil {
		return report, err
	}
	report.Stages = append(report.Stages, "completion_saved")
	j, err = s.Job(j.ID)
	if err != nil {
		return report, err
	}
	report.State = j.State
	_, report.Launches = h.Counts()
	_, report.LogicalDeliveries = transport.Counts()
	if j.State != "completed" {
		return report, fmt.Errorf("mock completion was not applied")
	}
	return report, nil
}
