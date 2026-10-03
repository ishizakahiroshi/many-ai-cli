package dotsbridge

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

var testNow = time.Date(2026, 10, 3, 10, 0, 0, 0, time.UTC)

type rig struct {
	s    *Store
	path string
	j    Job
	l    Lease
	h    *FakeHub
	w    *Worker
	i    Intent
	now  time.Time
}

func makeRig(t *testing.T) *rig {
	t.Helper()
	p := filepath.Join(t.TempDir(), "bridge.db")
	s, err := Open(p)
	must(t, err)
	t.Cleanup(func() { s.Close() })
	j, err := s.Create("campaign", RouteM, "color", testNow)
	must(t, err)
	_, err = s.Ingest(event(j, "question", 1, "色は？", RouteM), testNow)
	must(t, err)
	l, err := s.Claim(j.ID, "worker", testNow)
	must(t, err)
	h := NewFakeHub("instance-1")
	w := &Worker{Store: s, Hub: h, Consumers: h, Routine: "routine", ExpectedInstance: "instance-1"}
	return &rig{s: s, path: p, j: j, l: l, h: h, w: w, now: testNow}
}
func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}
func (r *rig) launch(t *testing.T) {
	t.Helper()
	var err error
	r.i, err = r.w.Launch(context.Background(), r.l, r.now)
	must(t, err)
}
func (r *rig) answer(t *testing.T) []byte {
	t.Helper()
	raw, err := r.w.mockAnswer(context.Background(), r.h, r.i, r.now)
	must(t, err)
	return raw
}
func (r *rig) reopen(t *testing.T) {
	t.Helper()
	must(t, r.s.Close())
	s, err := Open(r.path)
	must(t, err)
	r.s = s
	r.w.Store = s
	t.Cleanup(func() { s.Close() })
}
func (r *rig) export(t *testing.T) Export { t.Helper(); e, err := r.s.Export(); must(t, err); return e }
func count(t *testing.T, s *Store, table string) int {
	t.Helper()
	var n int
	must(t, s.db.QueryRow(`SELECT count(*) FROM `+table).Scan(&n))
	return n
}
func requireHeld(t *testing.T, err error) {
	t.Helper()
	if err == nil {
		t.Fatal("expected fail-closed hold")
	}
}

func TestCrossIngressDedup(t *testing.T) {
	r := makeRig(t)
	q := event(r.j, "question", 1, "色は？", RouteS)
	receipt, err := r.s.Ingest(q, r.now)
	must(t, err)
	if !receipt.Ack || receipt.Disposition != "duplicate logical" {
		t.Fatal(receipt)
	}
	if count(t, r.s, "inbox") != 2 || count(t, r.s, "logical_events") != 1 {
		t.Fatal("delivery/semantic cardinality")
	}
	r.launch(t)
	raw := r.answer(t)
	must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	if len(r.export(t).Outbox) != 1 {
		t.Fatal("duplicate logical answer")
	}
	p, n := r.h.Counts()
	if p != 1 || n != 1 {
		t.Fatal(p, n)
	}
}
func TestPayloadHashConflicts(t *testing.T) {
	for _, source := range []Route{RouteM, RouteS} {
		t.Run(string(source), func(t *testing.T) {
			r := makeRig(t)
			q := event(r.j, "question", 1, "別の本文", source)
			_, err := r.s.Ingest(q, r.now)
			if !errors.Is(err, ErrConflict) {
				t.Fatal(err)
			}
			j, err := r.s.Job(r.j.ID)
			must(t, err)
			if j.State != "needs_reconcile" {
				t.Fatal(j.State)
			}
			_, err = r.w.Launch(context.Background(), r.l, r.now)
			requireHeld(t, err)
			p, _ := r.h.Counts()
			if p != 0 {
				t.Fatal("POST after conflict")
			}
		})
	}
}
func TestOutOfOrder(t *testing.T) {
	r := makeRig(t)
	early := event(r.j, "completed", 5, "done", RouteM)
	receipt, err := r.s.Ingest(early, r.now)
	must(t, err)
	if !strings.Contains(receipt.Disposition, "prerequisite") {
		t.Fatal(receipt)
	}
	_, err = r.s.Ingest(event(r.j, "blocked", 3, "wait", RouteM), r.now)
	must(t, err)
	late := event(r.j, "blocked", 2, "old", RouteS)
	late.LogicalMessageID = "late-blocked"
	late.PayloadHash = late.Hash()
	receipt, err = r.s.Ingest(late, r.now)
	must(t, err)
	if !strings.Contains(receipt.Disposition, "old sequence") {
		t.Fatal(receipt)
	}
	j, err := r.s.Job(r.j.ID)
	must(t, err)
	if j.State != "blocked" || j.Sequence != 3 {
		t.Fatal(j)
	}
}
func TestSelfReplyIgnored(t *testing.T) {
	r := makeRig(t)
	e := event(r.j, "question", 9, "loop", RouteS)
	e.Origin = "bridge"
	e.LogicalMessageID = "self"
	e.PayloadHash = e.Hash()
	receipt, err := r.s.Ingest(e, r.now)
	must(t, err)
	if !strings.Contains(receipt.Disposition, "self reply") {
		t.Fatal(receipt)
	}
	if count(t, r.s, "logical_events") != 1 {
		t.Fatal("self reply accepted")
	}
	p, _ := r.h.Counts()
	if p != 0 {
		t.Fatal("self reply launched AI")
	}
}
func TestInFlightRouteSwitchKeepsResult(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	request := r.i.RequestID
	must(t, r.s.SwitchRoute(r.j.ID, RouteS, r.now))
	raw := r.answer(t)
	must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	e := r.export(t)
	if len(e.Outbox) != 1 || e.Outbox[0].Route != RouteS || e.Outbox[0].RouteEpoch != 2 || e.Launches[0].RequestID != request || e.Jobs[0].ExecutionEpoch != 1 {
		t.Fatal(e)
	}
	must(t, r.s.SwitchRoute(r.j.ID, RouteM, r.now))
	o, err := r.s.BeginDelivery(r.j.ID, RouteS, 2, r.now)
	requireHeld(t, err)
	if o.ID != "" {
		t.Fatal("stale route lease")
	}
	o, err = r.s.BeginDelivery(r.j.ID, RouteM, 3, r.now)
	must(t, err)
	must(t, r.s.FinishDelivery(o, Delivered, 0, r.now))
	_, err = r.s.BeginDelivery(r.j.ID, RouteM, 3, r.now)
	requireHeld(t, err)
}
func TestRouteFailoverAndReturn(t *testing.T) {
	for _, route := range []Route{RouteM, RouteS} {
		for _, caseID := range []string{"color", "recovery"} {
			t.Run(string(route)+caseID, func(t *testing.T) {
				s, err := Open(filepath.Join(t.TempDir(), "trial.db"))
				must(t, err)
				defer s.Close()
				report, err := RunMock(s, "comparison", route, caseID, testNow)
				must(t, err)
				if report.State != "completed" || report.Launches != 1 || report.LogicalDeliveries != 1 || len(report.Stages) != 7 {
					t.Fatal(report)
				}
				_, err = RunMock(s, "comparison", route, caseID, testNow)
				requireHeld(t, err)
			})
		}
	}
}
func TestHubAliasCollision(t *testing.T) {
	r := makeRig(t)
	var manual Run
	r.h.BeforePOST(func() { var err error; manual, err = r.h.Manual("routine", "manual-request", 77); must(t, err) })
	_, err := r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	cap, err := r.h.consumer(manual.ID)
	must(t, err)
	_, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	requireHeld(t, err)
	e := r.export(t)
	if e.Launches[0].Consumed || e.Jobs[0].State != "needs_reconcile" {
		t.Fatal(e)
	}
	_, err = r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	p, n := r.h.Counts()
	if p != 1 || n != 1 {
		t.Fatal(p, n)
	}
}
func TestStaleLeaseCannotLaunch(t *testing.T) {
	for _, mutate := range []func(*Lease){func(l *Lease) { l.ID = "forged" }, func(l *Lease) { l.Owner = "different" }, func(l *Lease) { l.Epoch++ }, func(l *Lease) { l.Until = l.Until.Add(time.Second) }} {
		t.Run("exact", func(t *testing.T) {
			r := makeRig(t)
			l := r.l
			mutate(&l)
			_, err := r.w.Launch(context.Background(), l, r.now)
			requireHeld(t, err)
			p, _ := r.h.Counts()
			if p != 0 {
				t.Fatal("stale lease POST")
			}
		})
	}
	r := makeRig(t)
	_, err := r.w.Launch(context.Background(), r.l, r.j.ExpiresAt)
	requireHeld(t, err)
	p, _ := r.h.Counts()
	if p != 0 {
		t.Fatal("expired lease POST")
	}
}
func TestMailboxResultCorrelation(t *testing.T) {
	cases := map[string]func(*Answer){"previous job": func(a *Answer) { a.JobID = "previous" }, "question": func(a *Answer) { a.QuestionID = "other" }, "epoch": func(a *Answer) { a.ExecutionEpoch++ }, "run": func(a *Answer) { a.RunID = "other" }, "session": func(a *Answer) { a.SessionID++ }, "instance": func(a *Answer) { a.HubInstanceID = "old" }, "request": func(a *Answer) { a.RequestID = "old" }, "nonce": func(a *Answer) { a.DispatchNonce = "self-claim" }}
	for name, mutate := range cases {
		t.Run(name, func(t *testing.T) {
			r := makeRig(t)
			r.launch(t)
			raw := r.answer(t)
			var a Answer
			must(t, json.Unmarshal(raw, &a))
			mutate(&a)
			bad, err := json.Marshal(a)
			must(t, err)
			requireHeld(t, r.w.Collect(context.Background(), r.l, bad, r.now))
			if count(t, r.s, "outbox") != 0 || count(t, r.s, "quarantine") != 1 {
				t.Fatal("invalid result escaped")
			}
			must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
		})
	}
	for name, raw := range map[string][]byte{"partial": []byte(`{"job_id":`), "oversized": []byte(strings.Repeat("x", MaxResultBytes+1)), "trailing": []byte(`{} {}`), "unknown": []byte(`{"unexpected":true}`)} {
		t.Run(name, func(t *testing.T) {
			r := makeRig(t)
			r.launch(t)
			requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.now))
			e := r.export(t)
			if len(e.Outbox) != 0 || len(e.Quarantine) != 1 || len(e.Quarantine[0].Sample) > MaxResultBytes {
				t.Fatal("bad bounded quarantine")
			}
		})
	}
}
func TestStaleExecutionOwnerRejected(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw := r.answer(t)
	next, err := r.w.FenceAfterTerminal(context.Background(), r.l, "worker2", r.now)
	must(t, err)
	if next.Epoch != 2 {
		t.Fatal(next)
	}
	requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	e := r.export(t)
	if len(e.Outbox) != 0 || len(e.Quarantine) != 1 {
		t.Fatal(e)
	}
	_, err = r.w.Launch(context.Background(), next, r.now)
	requireHeld(t, err)
	p, _ := r.h.Counts()
	if p != 1 {
		t.Fatal("fenced owner relaunched")
	}
}
func TestInputRequiresVerifiedConsumerAndOneUse(t *testing.T) {
	r := makeRig(t)
	_, err := r.w.Acquire(context.Background(), r.j.ID, ConsumerCapability{key: "self-claimed"}, r.now)
	requireHeld(t, err)
	r.launch(t)
	_, err = r.w.Acquire(context.Background(), r.j.ID, ConsumerCapability{key: r.i.Nonce}, r.now)
	requireHeld(t, err)
	other, err := r.h.Manual("other-routine", "manual", r.i.Run.SessionID)
	must(t, err)
	otherCap, err := r.h.consumer(other.ID)
	must(t, err)
	_, err = r.w.Acquire(context.Background(), r.j.ID, otherCap, r.now)
	requireHeld(t, err)
	cap, err := r.h.consumer(r.i.Run.ID)
	must(t, err)
	input, err := r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	must(t, err)
	if input.RunID != r.i.Run.ID || input.DispatchNonce != r.i.Nonce {
		t.Fatal(input)
	}
	_, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	requireHeld(t, err)
}
func TestSessionReuseCannotAuthorizeWrongRun(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	other := NewFakeHub("instance-2")
	_, err := other.Manual("routine", r.i.RequestID, r.i.Run.SessionID)
	must(t, err)
	cap, err := other.consumer("fake-run-1")
	must(t, err)
	r.w.Consumers = other
	_, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	requireHeld(t, err)
	if r.export(t).Launches[0].Consumed {
		t.Fatal("session reuse exposed input")
	}
}
func TestPostResponseLossGETReconcile(t *testing.T) {
	r := makeRig(t)
	r.h.LosePOSTResponse(true)
	r.launch(t)
	if r.i.State != "bound" || r.i.Run.ID == "" {
		t.Fatal(r.i)
	}
	raw := r.answer(t)
	must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	p, n := r.h.Counts()
	if p != 1 || n != 1 {
		t.Fatal(p, n)
	}
}
func TestUnknownRunNeverReposts(t *testing.T) {
	r := makeRig(t)
	r.h.LosePOSTResponse(true)
	r.h.BeforePOST(func() { r.h.HideHistory(true) })
	_, err := r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	r.reopen(t)
	_, err = r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	p, n := r.h.Counts()
	if p != 1 || n != 1 {
		t.Fatal(p, n)
	}
	e := r.export(t)
	if e.Budgets[0].Launches != 1 || e.Jobs[0].State != "needs_reconcile" {
		t.Fatal(e)
	}
}
func TestSaveFailurePreventsPOST(t *testing.T) {
	for _, point := range []string{"launch_intent", "pre_post"} {
		t.Run(point, func(t *testing.T) {
			r := makeRig(t)
			r.s.fail = func(p string) error {
				if p == point {
					return errors.New("simulated persistence failure")
				}
				return nil
			}
			_, err := r.w.Launch(context.Background(), r.l, r.now)
			requireHeld(t, err)
			r.s.fail = nil
			p, _ := r.h.Counts()
			if p != 0 {
				t.Fatal("POST before required persistence")
			}
			if point == "launch_intent" {
				e := r.export(t)
				if len(e.Launches) != 0 || e.Budgets[0].Launches != 0 {
					t.Fatal("rollback failed")
				}
			}
		})
	}
}
func TestCrashReconcile(t *testing.T) {
	for _, point := range []string{"after_intent_before_post", "after_post_before_bind"} {
		t.Run(point, func(t *testing.T) {
			r := makeRig(t)
			r.w.Fault = func(p string) error {
				if p == point {
					return errors.New("crash")
				}
				return nil
			}
			_, err := r.w.Launch(context.Background(), r.l, r.now)
			requireHeld(t, err)
			r.reopen(t)
			r.w.Fault = nil
			i, err := r.w.Launch(context.Background(), r.l, r.now)
			if point == "after_intent_before_post" {
				requireHeld(t, err)
			} else {
				must(t, err)
				if i.State != "bound" {
					t.Fatal(i)
				}
			}
			p, _ := r.h.Counts()
			expected := 1
			if point == "after_intent_before_post" {
				expected = 0
			}
			if p != expected {
				t.Fatal(p)
			}
			if r.export(t).Budgets[0].Launches != 1 {
				t.Fatal("budget reset")
			}
		})
	}
	for _, point := range []string{"result", "after_result_commit"} {
		t.Run(point, func(t *testing.T) {
			r := makeRig(t)
			r.launch(t)
			raw := r.answer(t)
			if point == "result" {
				r.s.fail = func(p string) error {
					if p == point {
						return errors.New("disk failure")
					}
					return nil
				}
			} else {
				r.w.Fault = func(p string) error {
					if p == point {
						return errors.New("crash")
					}
					return nil
				}
			}
			requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.now))
			r.s.fail = nil
			r.w.Fault = nil
			r.reopen(t)
			must(t, r.w.Collect(context.Background(), r.l, raw, r.now))
			if count(t, r.s, "outbox") != 1 {
				t.Fatal("result recovery duplicated effect")
			}
		})
	}
}
func TestAckBeforeDBFailureAndAckLoss(t *testing.T) {
	s, err := Open(filepath.Join(t.TempDir(), "inbox.db"))
	must(t, err)
	defer s.Close()
	j, err := s.Create("ack", RouteM, "color", testNow)
	must(t, err)
	e := event(j, "question", 1, "色", RouteM)
	s.fail = func(p string) error {
		if p == "inbox" {
			return errors.New("disk failed")
		}
		return nil
	}
	receipt, err := s.Ingest(e, testNow)
	requireHeld(t, err)
	if receipt.Ack || count(t, s, "inbox") != 0 {
		t.Fatal("ACK before durable receipt")
	}
	s.fail = nil
	_, err = s.Ingest(e, testNow)
	must(t, err)
	receipt, err = s.Ingest(e, testNow)
	must(t, err)
	if !receipt.Ack || receipt.Disposition != "duplicate delivery" || count(t, s, "logical_events") != 1 {
		t.Fatal(receipt)
	}
}
func TestRelayOutageRestartRetainsBudget(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	r.reopen(t)
	must(t, r.s.SwitchRoute(r.j.ID, RouteS, r.now))
	_, err := r.w.Launch(context.Background(), r.l, r.now)
	must(t, err)
	p, _ := r.h.Counts()
	if p != 1 || r.export(t).Budgets[0].Launches != 1 {
		t.Fatal("outage or route reset launch budget")
	}
	_, err = r.s.Create("campaign", RouteM, "recovery", r.now)
	must(t, err)
	if r.export(t).Budgets[0].Jobs != 2 {
		t.Fatal("job budget")
	}
}
func TestLateAnswerRetainedAndNoKill(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw := r.answer(t)
	requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.j.ExpiresAt))
	e := r.export(t)
	if len(e.Outbox) != 0 || len(e.Quarantine) != 1 || e.Jobs[0].State != "needs_reconcile" || !strings.Contains(e.Quarantine[0].Reason, "late") {
		t.Fatal(e)
	}
	p, n := r.h.Counts()
	if p != 1 || n != 1 {
		t.Fatal(p, n)
	}
}
func TestFinishedWithoutResultHeld(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw := r.answer(t)
	must(t, r.h.Finish(r.i.Run.ID, false))
	requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	e := r.export(t)
	if len(e.Outbox) != 0 || len(e.Quarantine) != 1 || e.Jobs[0].State != "needs_reconcile" {
		t.Fatal(e)
	}
}
func TestConcurrentOwnersAndRoutineLock(t *testing.T) {
	s, err := Open(filepath.Join(t.TempDir(), "concurrent.db"))
	must(t, err)
	defer s.Close()
	j, err := s.Create("concurrent", RouteM, "color", testNow)
	must(t, err)
	_, err = s.Ingest(event(j, "question", 1, "color", RouteM), testNow)
	must(t, err)
	var winners atomic.Int32
	var wg sync.WaitGroup
	for n := 0; n < 12; n++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, e := s.Claim(j.ID, "worker", testNow); e == nil {
				winners.Add(1)
			}
		}()
	}
	wg.Wait()
	if winners.Load() != 1 {
		t.Fatal(winners.Load())
	}
	r := makeRig(t)
	r.w.Fault = func(p string) error {
		if p == "after_intent_before_post" {
			return errors.New("crash")
		}
		return nil
	}
	_, err = r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	j2, err := r.s.Create("other", RouteS, "color", r.now)
	must(t, err)
	_, err = r.s.Ingest(event(j2, "question", 1, "color", RouteS), r.now)
	must(t, err)
	l2, err := r.s.Claim(j2.ID, "worker2", r.now)
	must(t, err)
	r.w.Fault = nil
	_, err = r.w.Launch(context.Background(), l2, r.now)
	requireHeld(t, err)
	p, _ := r.h.Counts()
	if p != 0 {
		t.Fatal("routine lock lost")
	}
}
func TestDeliveryUncertaintyNeverBlindResends(t *testing.T) {
	for _, kind := range []string{"unknown", "crash", "ack_commit_failure"} {
		t.Run(kind, func(t *testing.T) {
			r := makeRig(t)
			r.launch(t)
			must(t, r.w.Collect(context.Background(), r.l, r.answer(t), r.now))
			o, err := r.s.BeginDelivery(r.j.ID, RouteM, 1, r.now)
			must(t, err)
			transport := NewFakeTransport(RouteM)
			outcome := transport.Send(o)
			if kind == "unknown" {
				outcome = Uncertain
				must(t, r.s.FinishDelivery(o, outcome, 0, r.now))
			} else if kind == "ack_commit_failure" {
				r.s.fail = func(p string) error {
					if p == "delivery_ack" {
						return errors.New("disk")
					}
					return nil
				}
				requireHeld(t, r.s.FinishDelivery(o, outcome, 0, r.now))
				r.s.fail = nil
			}
			r.reopen(t)
			must(t, r.s.RecoverDelivery(r.j.ID))
			requireHeld(t, r.s.SwitchRoute(r.j.ID, RouteS, r.now))
			_, err = r.s.BeginDelivery(r.j.ID, RouteM, 1, r.now)
			requireHeld(t, err)
			calls, effects := transport.Counts()
			if calls != 1 || effects != 1 || r.export(t).Outbox[0].State != "needs_reconcile" {
				t.Fatal(calls, effects, r.export(t))
			}
		})
	}
}
func TestDeliveryAttemptOwnershipBudgetAndBackoff(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	must(t, r.w.Collect(context.Background(), r.l, r.answer(t), r.now))
	for n := 0; n < 3; n++ {
		o, err := r.s.BeginDelivery(r.j.ID, RouteM, 1, r.now)
		must(t, err)
		forged := o
		forged.AttemptID = "forged"
		requireHeld(t, r.s.FinishDelivery(forged, Delivered, 0, r.now))
		must(t, r.s.FinishDelivery(o, NotSent, 5*time.Second, r.now))
		_, err = r.s.BeginDelivery(r.j.ID, RouteM, 1, r.now)
		requireHeld(t, err)
		r.now = r.now.Add(5 * time.Second)
	}
	if r.export(t).Outbox[0].Attempts != 3 || r.export(t).Jobs[0].State != "needs_reconcile" {
		t.Fatal(r.export(t))
	}
}
func TestStopFreezesClaimsLaunchDeliveryAndRetainsState(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw := r.answer(t)
	must(t, r.s.Stop(r.now))
	r.reopen(t)
	requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.now))
	_, err := r.s.Create("new", RouteM, "color", r.now)
	if !errors.Is(err, ErrStopped) {
		t.Fatal(err)
	}
	if !r.export(t).Stopped || len(r.export(t).Quarantine) != 1 {
		t.Fatal(r.export(t))
	}
}
func TestDedicatedDatabaseOwnership(t *testing.T) {
	p := filepath.Join(t.TempDir(), "other.db")
	db, err := sql.Open("sqlite", p)
	must(t, err)
	_, err = db.Exec(`CREATE TABLE jobs(id TEXT); INSERT INTO jobs VALUES('other application')`)
	must(t, err)
	must(t, db.Close())
	before, err := os.ReadFile(p)
	must(t, err)
	s, err := Open(p)
	if err == nil {
		s.Close()
		t.Fatal("unrelated DB accepted")
	}
	after, err := os.ReadFile(p)
	must(t, err)
	if string(before) != string(after) {
		t.Fatal("unrelated DB modified")
	}
}
func TestExportDoesNotExposeDispatchCapabilities(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw, err := json.Marshal(r.export(t))
	must(t, err)
	if strings.Contains(string(raw), r.i.Nonce) || strings.Contains(string(raw), r.l.ID) {
		t.Fatal("capability exported")
	}
}

func TestManualRunCannotReadStagingBetweenCheckAndPOST(t *testing.T) {
	r := makeRig(t)
	unauthorizedReads := 0
	r.h.BeforePOST(func() {
		manual, err := r.h.Manual("routine", "manual-gap", r.i.Run.SessionID)
		must(t, err)
		cap, err := r.h.consumer(manual.ID)
		must(t, err)
		if _, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.now); err == nil {
			unauthorizedReads++
		}
	})
	_, err := r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	if unauthorizedReads != 0 || r.export(t).Launches[0].Consumed {
		t.Fatal("manual run read staged input")
	}
}
func TestConcurrentStoresHaveOneExecutionOwner(t *testing.T) {
	p := filepath.Join(t.TempDir(), "shared.db")
	a, err := Open(p)
	must(t, err)
	defer a.Close()
	b, err := Open(p)
	must(t, err)
	defer b.Close()
	j, err := a.Create("parallel", RouteM, "color", testNow)
	must(t, err)
	_, err = a.Ingest(event(j, "question", 1, "color", RouteM), testNow)
	must(t, err)
	var won atomic.Int32
	var wg sync.WaitGroup
	for n := 0; n < 20; n++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			s := a
			if n%2 != 0 {
				s = b
			}
			if _, e := s.Claim(j.ID, "owner", testNow); e == nil {
				won.Add(1)
			}
		}(n)
	}
	wg.Wait()
	if won.Load() != 1 {
		t.Fatal(won.Load())
	}
}
func TestConcurrentMailboxAcquisitionIsOneUse(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	cap, err := r.h.consumer(r.i.Run.ID)
	must(t, err)
	var got atomic.Int32
	var wg sync.WaitGroup
	for n := 0; n < 16; n++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, e := r.w.Acquire(context.Background(), r.j.ID, cap, r.now); e == nil {
				got.Add(1)
			}
		}()
	}
	wg.Wait()
	if got.Load() != 1 {
		t.Fatal(got.Load())
	}
}
func TestCumulativeLaunchBudgetBeforePOST(t *testing.T) {
	r := makeRig(t)
	_, err := r.s.db.Exec(`UPDATE budgets SET launches=2`)
	must(t, err)
	_, err = r.w.Launch(context.Background(), r.l, r.now)
	if !errors.Is(err, ErrBudget) {
		t.Fatal(err)
	}
	p, _ := r.h.Counts()
	if p != 0 || count(t, r.s, "launch_intents") != 0 || count(t, r.s, "routine_locks") != 0 {
		t.Fatal("budget check was not atomic")
	}
}
func TestAcquireCommitFailureReturnsNoInput(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	cap, err := r.h.consumer(r.i.Run.ID)
	must(t, err)
	r.s.fail = func(p string) error {
		if p == "consume_input" {
			return errors.New("disk")
		}
		return nil
	}
	input, err := r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	requireHeld(t, err)
	if input.JobID != "" {
		t.Fatal("input released before commit")
	}
	r.s.fail = nil
	_, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.now)
	must(t, err)
}
func TestDeadlineBlocksClaimAcquireAndDelivery(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	cap, err := r.h.consumer(r.i.Run.ID)
	must(t, err)
	_, err = r.w.Acquire(context.Background(), r.j.ID, cap, r.j.ExpiresAt)
	requireHeld(t, err)
	must(t, r.w.Collect(context.Background(), r.l, r.answer(t), r.now))
	_, err = r.s.BeginDelivery(r.j.ID, RouteM, 1, r.j.ExpiresAt)
	if !errors.Is(err, ErrExpired) {
		t.Fatal(err)
	}
	_, err = r.s.Claim(r.j.ID, "late", r.j.ExpiresAt)
	requireHeld(t, err)
}
func TestDatabaseOutageCannotLaunch(t *testing.T) {
	r := makeRig(t)
	must(t, r.s.Close())
	_, err := r.w.Launch(context.Background(), r.l, r.now)
	requireHeld(t, err)
	p, _ := r.h.Counts()
	if p != 0 {
		t.Fatal("POST during ledger outage")
	}
}
func TestResultCommitAndDeliveryIntentFailureHaveNoEffect(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	must(t, r.w.Collect(context.Background(), r.l, r.answer(t), r.now))
	r.s.fail = func(p string) error {
		if p == "delivery_intent" {
			return errors.New("disk")
		}
		return nil
	}
	o, err := r.s.BeginDelivery(r.j.ID, RouteM, 1, r.now)
	requireHeld(t, err)
	if o.ID != "" {
		t.Fatal("uncommitted delivery attempt returned")
	}
}

func TestQuarantinedNonceIsNotExported(t *testing.T) {
	r := makeRig(t)
	r.launch(t)
	raw := r.answer(t)
	requireHeld(t, r.w.Collect(context.Background(), r.l, raw, r.j.ExpiresAt))
	e, err := json.Marshal(r.export(t))
	must(t, err)
	if strings.Contains(string(e), r.i.Nonce) {
		t.Fatal("quarantined nonce exported")
	}
	var body string
	must(t, r.s.db.QueryRow(`SELECT body FROM quarantine`).Scan(&body))
	if !strings.Contains(body, r.i.Nonce) {
		t.Fatal("local recovery evidence lost")
	}
}
