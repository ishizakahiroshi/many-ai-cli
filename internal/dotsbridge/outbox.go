package dotsbridge

import (
	"database/sql"
	"errors"
	"time"
)

// BeginDelivery durably records an attempt before the transport may send.
func (s *Store) BeginDelivery(job string, route Route, epoch int64, now time.Time) (Outbox, error) {
	var out Outbox
	token, err := opaque()
	if err != nil {
		return Outbox{}, err
	}
	err = s.transaction("delivery_intent", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		j, err := getJob(tx, job)
		if err != nil {
			return err
		}
		if err = j.live(now); err != nil {
			return err
		}
		o, err := getOutbox(tx, job)
		if err != nil {
			return err
		}
		if j.Route != route || j.RouteEpoch != epoch || o.Route != route || o.RouteEpoch != epoch || o.State != "pending" || o.Attempts >= 3 || now.Before(o.NotBefore) {
			return ErrHeld
		}
		o.State = "sending"
		o.Attempts++
		o.AttemptID = token
		out = o
		return putOutbox(tx, o)
	})
	if err != nil {
		return Outbox{}, err
	}
	return out, nil
}

// FinishDelivery requires the entire saved attempt, not only an ID or owner name.
func (s *Store) FinishDelivery(attempt Outbox, result DeliveryOutcome, retryAfter time.Duration, now time.Time) error {
	if result != Delivered && result != NotSent && result != Uncertain {
		return ErrHeld
	}
	return s.transaction("delivery_ack", func(tx *sql.Tx) error {
		j, err := getJob(tx, attempt.JobID)
		if err != nil {
			return err
		}
		o, err := getOutbox(tx, j.ID)
		if err != nil {
			return err
		}
		if o != attempt || o.State != "sending" || attempt.AttemptID == "" {
			return ErrLease
		}
		if result == Delivered {
			o.State = "delivered"
		} else if result == Uncertain {
			o.State = "needs_reconcile"
			if err = hold(tx, j, "delivery outcome unknown; do not resend on another route"); err != nil {
				return err
			}
		} else {
			if o.Attempts >= 3 || !now.Before(j.ExpiresAt) {
				o.State = "needs_reconcile"
				if err = hold(tx, j, "delivery retry budget/deadline reached"); err != nil {
					return err
				}
			} else {
				o.State = "pending"
				backoff := time.Second * time.Duration(1<<uint(o.Attempts-1))
				if retryAfter > backoff {
					backoff = retryAfter
				}
				o.NotBefore = now.Add(backoff)
			}
		}
		return putOutbox(tx, o)
	})
}

// RecoverDelivery converts durable in-flight attempts into uncertainty, never retry.
func (s *Store) RecoverDelivery(job string) error {
	return s.transaction("delivery_recovery", func(tx *sql.Tx) error {
		j, err := getJob(tx, job)
		if err != nil {
			return err
		}
		o, err := getOutbox(tx, job)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		if o.State != "sending" {
			return nil
		}
		o.State = "needs_reconcile"
		if err = putOutbox(tx, o); err != nil {
			return err
		}
		return hold(tx, j, "interrupted delivery; reconcile physical effect")
	})
}
