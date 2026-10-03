package dotsbridge

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// HubClient mirrors existing routine POST/list/detail semantics. Implementations
// must not treat request aliases as a canonical request match. No live adapter exists.
type HubClient interface {
	List(context.Context, string) ([]Run, error)
	Post(context.Context, string, string) (Run, error)
	Get(context.Context, string) (Run, error)
}

// ConsumerVerifier is a LOCAL broker boundary, not an added Hub HTTP endpoint.
// A consumer's claimed IDs/nonce alone must never satisfy this interface.
type ConsumerVerifier interface {
	VerifyConsumer(context.Context, ConsumerCapability) (Run, error)
}

// ConsumerCapability is opaque. Only the fake trusted issuer can mint it here.
type ConsumerCapability struct{ key string }
type Worker struct {
	Store            *Store
	Hub              HubClient
	Consumers        ConsumerVerifier
	Routine          string
	ExpectedInstance string
	Fault            func(string) error
}

func (w *Worker) fault(point string) error {
	if w.Fault != nil {
		return w.Fault(point)
	}
	return nil
}
func (w *Worker) Launch(ctx context.Context, l Lease, now time.Time) (Intent, error) {
	if w.Hub == nil || w.Consumers == nil || !validID(w.Routine) || w.ExpectedInstance == "" {
		return Intent{}, ErrHeld
	}
	// Persisted intents never lead to another POST, even after a crash before POST.
	err := w.Store.transaction("read", func(tx *sql.Tx) error { var e error; _, e = getIntent(tx, l.JobID); return e })
	if err == nil {
		return w.Reconcile(ctx, l, now)
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return Intent{}, err
	}
	runs, err := w.Hub.List(ctx, w.Routine)
	if err != nil {
		return Intent{}, ErrHeld
	}
	for _, r := range runs {
		if r.active() {
			return Intent{}, ErrHeld
		}
	}
	nonce, err := opaque()
	if err != nil {
		return Intent{}, err
	}
	var intent Intent
	err = w.Store.transaction("launch_intent", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		j, err := getJob(tx, l.JobID)
		if err != nil {
			return err
		}
		if !j.exact(l, now) {
			return ErrLease
		}
		if err = j.live(now); err != nil {
			return err
		}
		if j.State != "question" && j.State != "blocked" {
			return ErrHeld
		}
		if _, err = getIntent(tx, j.ID); err == nil {
			return ErrHeld
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if _, err = tx.Exec(`INSERT INTO routine_locks VALUES(?,?)`, w.Routine, j.ID); err != nil {
			return ErrHeld
		}
		r, err := tx.Exec(`UPDATE budgets SET launches=launches+1 WHERE campaign=? AND candidate=? AND launches<2`, j.Campaign, j.Candidate)
		if err != nil {
			return err
		}
		n, _ := r.RowsAffected()
		if n != 1 {
			return ErrBudget
		}
		intent = Intent{JobID: j.ID, QuestionID: j.QuestionID, ExecutionEpoch: j.ExecutionEpoch, RequestID: requestID(j), RoutineID: w.Routine, State: "staged", Nonce: nonce, Input: "合成メモの色を青と回答してください"}
		if err = putIntent(tx, intent); err != nil {
			return err
		}
		j.State = "launching"
		return putJob(tx, j)
	})
	if err != nil {
		return Intent{}, err
	}
	if err = w.fault("after_intent_before_post"); err != nil {
		return intent, err
	}
	// Recheck the current ledger immediately before POST. Lease loss and DB outage fail closed.
	err = w.Store.transaction("pre_post", func(tx *sql.Tx) error {
		if e := stopped(tx); e != nil {
			return e
		}
		j, e := getJob(tx, l.JobID)
		if e != nil {
			return e
		}
		if !j.exact(l, now) {
			return ErrLease
		}
		return j.live(now)
	})
	if err != nil {
		return intent, err
	}
	run, postErr := w.Hub.Post(ctx, w.Routine, intent.RequestID)
	if err = w.fault("after_post_before_bind"); err != nil {
		return intent, err
	}
	if postErr != nil {
		return w.Reconcile(ctx, l, now)
	}
	return w.bind(l, intent, run, now)
}
func (w *Worker) bind(l Lease, i Intent, r Run, now time.Time) (Intent, error) {
	var semanticErr error
	err := w.Store.transaction("bind", func(tx *sql.Tx) error {
		j, err := getJob(tx, l.JobID)
		if err != nil {
			return err
		}
		current, err := getIntent(tx, j.ID)
		if err != nil {
			return err
		}
		i = current
		if !j.exact(l, now) {
			return ErrLease
		}
		if err = j.live(now); err != nil {
			return err
		}
		if err = stopped(tx); err != nil {
			return err
		}
		if !r.valid() || r.RoutineID != i.RoutineID || r.RequestID != i.RequestID || r.HubInstanceID != w.ExpectedInstance || (i.Run.ID != "" && i.Run.identity() != r.identity()) {
			semanticErr = ErrHeld
			if err = quarantine(tx, j.ID, "Hub identity/alias mismatch", []byte(encode(r))); err != nil {
				return err
			}
			return hold(tx, j, "Hub identity/alias mismatch")
		}
		i.Run = r
		i.State = "bound"
		if err = putIntent(tx, i); err != nil {
			return err
		}
		j.State = "running"
		return putJob(tx, j)
	})
	if err != nil {
		return i, err
	}
	return i, semanticErr
}
func (w *Worker) Reconcile(ctx context.Context, l Lease, now time.Time) (Intent, error) {
	var i Intent
	err := w.Store.transaction("read", func(tx *sql.Tx) error {
		j, e := getJob(tx, l.JobID)
		if e != nil {
			return e
		}
		if !j.exact(l, now) {
			return ErrLease
		}
		i, e = getIntent(tx, j.ID)
		return e
	})
	if err != nil {
		return i, err
	}
	// A committed result is terminal for launch reconciliation. Re-entering a
	// worker after result commit must not roll answered back to running.
	if i.State == "collected" {
		return i, nil
	}
	runs, err := w.Hub.List(ctx, i.RoutineID)
	if err == nil {
		for _, r := range runs {
			if r.RequestID == i.RequestID && r.RoutineID == i.RoutineID {
				detail, e := w.Hub.Get(ctx, r.ID)
				if e == nil {
					return w.bind(l, i, detail, now)
				}
			}
		}
	}
	err = w.Store.transaction("unknown_run", func(tx *sql.Tx) error {
		j, e := getJob(tx, l.JobID)
		if e != nil {
			return e
		}
		if !j.exact(l, now) {
			return ErrLease
		}
		return hold(tx, j, "unknown launch outcome; never automatically re-POST")
	})
	if err != nil {
		return i, err
	}
	return i, ErrHeld
}

// FenceAfterTerminal is explicit operator reconciliation, never lease-timeout recovery.
// It only fences a verified terminal run and keeps the job held, without starting another run.
func (w *Worker) FenceAfterTerminal(ctx context.Context, l Lease, newOwner string, now time.Time) (Lease, error) {
	var next Lease
	if !validID(newOwner) {
		return next, ErrLease
	}
	var i Intent
	err := w.Store.transaction("read", func(tx *sql.Tx) error { var e error; i, e = getIntent(tx, l.JobID); return e })
	if err != nil {
		return next, err
	}
	r, err := w.Hub.Get(ctx, i.Run.ID)
	if err != nil || r.identity() != i.Run.identity() || r.active() || r.Status != "finished" {
		return next, ErrHeld
	}
	token, err := opaque()
	if err != nil {
		return next, err
	}
	err = w.Store.transaction("fence", func(tx *sql.Tx) error {
		j, e := getJob(tx, l.JobID)
		if e != nil {
			return e
		}
		if j.Lease != l {
			return ErrLease
		}
		j.ExecutionEpoch++
		next = Lease{j.ID, newOwner, token, j.ExecutionEpoch, j.ExpiresAt}
		j.Lease = next
		return hold(tx, j, "terminal run ownership explicitly fenced; reconcile retained result")
	})
	return next, err
}
func (w *Worker) check() error {
	if w.Store == nil || w.Hub == nil || w.Consumers == nil {
		return fmt.Errorf("offline worker is not configured")
	}
	return nil
}
