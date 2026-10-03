package dotsbridge

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"time"
)

// Acquire releases staged input only to the independently verified run consumer.
// No file path, self-claimed run ID, or nonce can grant access by itself.
func (w *Worker) Acquire(ctx context.Context, jobID string, cap ConsumerCapability, now time.Time) (Input, error) {
	var input Input
	if err := w.check(); err != nil {
		return input, err
	}
	verified, err := w.Consumers.VerifyConsumer(ctx, cap)
	if err != nil {
		return input, ErrConsumer
	}
	// Re-read the Hub detail, rather than trusting session ID reuse or stale issuer state.
	current, err := w.Hub.Get(ctx, verified.ID)
	if err != nil || current.identity() != verified.identity() || !current.active() {
		return input, ErrConsumer
	}
	err = w.Store.transaction("consume_input", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		j, err := getJob(tx, jobID)
		if err != nil {
			return err
		}
		if err = j.live(now); err != nil {
			return err
		}
		if !j.exact(j.Lease, now) {
			return ErrLease
		}
		i, err := getIntent(tx, jobID)
		if err != nil {
			return err
		}
		if i.State != "bound" || i.Consumed || !current.valid() || i.Run.identity() != current.identity() || i.ExecutionEpoch != j.ExecutionEpoch {
			return ErrConsumer
		}
		i.Consumed = true
		if err = putIntent(tx, i); err != nil {
			return err
		}
		input = Input{i.JobID, i.QuestionID, i.ExecutionEpoch, i.RequestID, i.Run.ID, i.Run.HubInstanceID, i.Run.SessionID, i.Nonce, i.Input}
		return nil
	})
	if err != nil {
		return Input{}, err
	}
	return input, nil
}
func (w *Worker) Collect(ctx context.Context, l Lease, raw []byte, now time.Time) error {
	if err := w.check(); err != nil {
		return err
	}
	var answer Answer
	reason := ""
	if len(raw) > MaxResultBytes {
		reason = "oversized result"
	} else {
		dec := json.NewDecoder(bytes.NewReader(raw))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&answer); err != nil {
			reason = "partial/invalid result JSON"
		} else if err = dec.Decode(new(any)); !errors.Is(err, io.EOF) {
			reason = "trailing result JSON"
		}
	}
	var i Intent
	err := w.Store.transaction("read", func(tx *sql.Tx) error { var e error; i, e = getIntent(tx, l.JobID); return e })
	if err != nil {
		return err
	}
	run, runErr := w.Hub.Get(ctx, i.Run.ID)
	var held bool
	err = w.Store.transaction("result", func(tx *sql.Tx) error {
		j, e := getJob(tx, l.JobID)
		if e != nil {
			return e
		}
		cur, e := getIntent(tx, j.ID)
		if e != nil {
			return e
		}
		cause := reason
		if cause == "" && (j.Lease != l || l.ID == "" || cur.ExecutionEpoch != j.ExecutionEpoch) {
			cause = "stale execution owner"
		}
		if cause == "" && (answer.JobID != j.ID || answer.QuestionID != cur.QuestionID || answer.ExecutionEpoch != cur.ExecutionEpoch || answer.RequestID != cur.RequestID || answer.RunID != cur.Run.ID || answer.HubInstanceID != cur.Run.HubInstanceID || answer.SessionID != cur.Run.SessionID || answer.DispatchNonce != cur.Nonce || !cur.Consumed || answer.Answer != "青" || answer.Task != cur.Input) {
			cause = "result correlation mismatch"
		}
		if cause == "" && (runErr != nil || run.identity() != cur.Run.identity() || run.Status != "finished" || !run.ResultAvailable) {
			cause = "run result unavailable or unverified"
		}
		if cause == "" && !now.Before(j.ExpiresAt) {
			cause = "late answer after deadline"
		}
		if cause == "" {
			if e = stopped(tx); e != nil {
				if !errors.Is(e, ErrStopped) {
					return e
				}
				cause = "answer after bridge stop"
			}
		}
		if cause == "" && j.State == "needs_reconcile" {
			cause = "job held for reconciliation"
		}
		if cause != "" {
			held = true
			if e = quarantine(tx, j.ID, cause, raw); e != nil {
				return e
			}
			if cause == "stale execution owner" || cause == "result correlation mismatch" || reason != "" {
				return nil
			}
			return hold(tx, j, cause)
		}
		old, e := getOutbox(tx, j.ID)
		if e == nil {
			if old.Payload != answer.Answer {
				held = true
				return hold(tx, j, "answer payload conflict")
			}
			return nil
		}
		if !errors.Is(e, sql.ErrNoRows) {
			return e
		}
		o := Outbox{ID: "answer:" + digest([]string{j.ID, j.QuestionID}), JobID: j.ID, Payload: answer.Answer, Route: j.Route, RouteEpoch: j.RouteEpoch, State: "pending"}
		if e = putOutbox(tx, o); e != nil {
			return e
		}
		j.State = "answered"
		if e = putJob(tx, j); e != nil {
			return e
		}
		cur.State = "collected"
		if e = putIntent(tx, cur); e != nil {
			return e
		}
		_, e = tx.Exec(`DELETE FROM routine_locks WHERE routine_id=? AND job_id=?`, cur.RoutineID, j.ID)
		return e
	})
	if err != nil {
		return err
	}
	if err = w.fault("after_result_commit"); err != nil {
		return err
	}
	if held {
		return ErrHeld
	}
	return nil
}
