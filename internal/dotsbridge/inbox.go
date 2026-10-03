package dotsbridge

import (
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// Ingest commits the receipt before returning Ack. Lost ACKs may safely retry.
func (s *Store) Ingest(e Envelope, now time.Time) (Receipt, error) {
	receipt := Receipt{}
	var semanticErr error
	err := s.transaction("inbox", func(tx *sql.Tx) error {
		j, err := getJob(tx, e.JobID)
		if err != nil {
			return err
		}
		disposition := "accepted"
		if !validRoute(e.Source) || !validID(e.SourceEventID) || !validID(e.LogicalMessageID) || e.CampaignID != j.Campaign || e.CaseID != j.Case || e.Sender != "fake-dot" || e.Thread != j.thread(e.Source) || !e.ExpiresAt.Equal(j.ExpiresAt) || len(e.Payload) > 4096 || e.Sequence < 1 || e.RouteEpoch < 1 || e.RouteEpoch > j.RouteEpoch {
			disposition = "quarantined: envelope"
		}
		if e.Origin == "bridge" {
			disposition = "ignored: self reply"
		} else if e.Origin != "dot" {
			disposition = "quarantined: origin"
		}
		hash := e.Hash()
		if e.PayloadHash != hash {
			disposition = "quarantined: payload hash"
		}
		if disposition != "accepted" {
			if err = quarantine(tx, j.ID, disposition, []byte(encode(e))); err != nil {
				return err
			}
			receipt = Receipt{true, disposition}
			return nil
		}
		var oldHash, oldJob, oldLogical string
		err = tx.QueryRow(`SELECT hash,job_id,logical_id FROM inbox WHERE source=? AND event_id=?`, e.Source, e.SourceEventID).Scan(&oldHash, &oldJob, &oldLogical)
		if err == nil {
			if oldHash != hash || oldJob != e.JobID || oldLogical != e.LogicalMessageID {
				semanticErr = ErrConflict
				if err = quarantine(tx, j.ID, "source event conflict", []byte(encode(e))); err != nil {
					return err
				}
				return hold(tx, j, "source event payload conflict")
			}
			receipt = Receipt{true, "duplicate delivery"}
			return nil
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		err = tx.QueryRow(`SELECT hash FROM logical_events WHERE job_id=? AND logical_id=?`, j.ID, e.LogicalMessageID).Scan(&oldHash)
		if err == nil {
			disposition = "duplicate logical"
			if oldHash != hash {
				disposition = "conflict"
				semanticErr = ErrConflict
				if err = hold(tx, j, "logical message payload conflict"); err != nil {
					return err
				}
			}
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		} else {
			if !now.Before(j.ExpiresAt) {
				disposition = "held: expired"
			} else if err = stopped(tx); err != nil {
				if !errors.Is(err, ErrStopped) {
					return err
				}
				disposition = "held: stopped"
			} else if j.State == "needs_reconcile" {
				disposition = "held: reconciliation"
			} else if e.ExecutionEpoch != j.ExecutionEpoch {
				disposition = "rejected: execution epoch"
			} else if e.Sequence <= j.Sequence {
				disposition = "rejected: old sequence"
			} else {
				switch e.Phase {
				case "question":
					if j.State != "created" || !validID(e.QuestionID) {
						disposition = "rejected: question prerequisite"
					} else {
						j.QuestionID = e.QuestionID
						j.State = "question"
					}
				case "blocked":
					if (j.State != "question" && j.State != "blocked" && j.State != "running") || e.QuestionID != j.QuestionID {
						disposition = "rejected: blocked prerequisite"
					} else {
						if j.State != "running" {
							j.State = "blocked"
						}
					}
				case "completed":
					o, oe := getOutbox(tx, j.ID)
					if oe != nil && !errors.Is(oe, sql.ErrNoRows) {
						return oe
					}
					if oe != nil || o.State != "delivered" || j.State != "answered" || e.QuestionID != j.QuestionID {
						disposition = "rejected: completion prerequisite"
					} else {
						j.State = "completed"
					}
				default:
					disposition = "ignored: phase"
				}
			}
			if disposition == "accepted" {
				j.Sequence = e.Sequence
				if err = putJob(tx, j); err != nil {
					return err
				}
				if _, err = tx.Exec(`INSERT INTO logical_events VALUES(?,?,?)`, j.ID, e.LogicalMessageID, hash); err != nil {
					return err
				}
			}
		}
		if disposition != "accepted" && disposition != "duplicate logical" {
			if err = quarantine(tx, j.ID, disposition, []byte(encode(e))); err != nil {
				return err
			}
		}
		_, err = tx.Exec(`INSERT INTO inbox VALUES(?,?,?,?,?,?,?)`, e.Source, e.SourceEventID, j.ID, e.LogicalMessageID, hash, disposition, encode(e))
		receipt = Receipt{true, disposition}
		return err
	})
	if err != nil {
		return Receipt{}, fmt.Errorf("inbox commit: %w", err)
	}
	return receipt, semanticErr
}
