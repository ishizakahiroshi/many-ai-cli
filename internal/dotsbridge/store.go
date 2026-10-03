package dotsbridge

import (
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"
)

//go:embed schema.sql
var schema string

type Store struct {
	db   *sql.DB
	fail func(string) error
}

// Open opens a dedicated bridge ledger. It refuses databases owned by another application.
func Open(path string) (*Store, error) {
	if path == "" || path == ":memory:" {
		return nil, fmt.Errorf("a dedicated ledger file is required")
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	if info, e := os.Lstat(abs); e == nil && !info.Mode().IsRegular() {
		return nil, fmt.Errorf("ledger must be a regular dedicated file")
	}
	f, err := os.OpenFile(abs, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	if err = f.Close(); err != nil {
		return nil, err
	}
	u := sqliteFileURL(filepath.ToSlash(abs))
	db, err := sql.Open("sqlite", u.String()+"?_pragma=busy_timeout(5000)&_pragma=foreign_keys(1)&_pragma=synchronous(FULL)&_txlock=immediate")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	s := &Store{db: db}
	var count, appID int
	if err = db.QueryRow(`SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'`).Scan(&count); err != nil {
		db.Close()
		return nil, fmt.Errorf("not a dedicated bridge ledger")
	}
	if err = db.QueryRow(`PRAGMA application_id`).Scan(&appID); err != nil || (count > 0 && appID != 1127367236) || (appID != 0 && appID != 1127367236) {
		db.Close()
		return nil, fmt.Errorf("not a dedicated bridge ledger")
	}
	if _, err = db.Exec(`PRAGMA application_id=1127367236`); err != nil {
		db.Close()
		return nil, err
	}
	if _, err = db.Exec(schema); err != nil {
		db.Close()
		return nil, err
	}
	var version int
	if err = db.QueryRow(`SELECT schema_version FROM bridge_meta WHERE id=1`).Scan(&version); err != nil || version != 1 {
		db.Close()
		return nil, fmt.Errorf("unsupported bridge schema")
	}
	return s, nil
}

// SQLite file URIs require a slash before a Windows drive, otherwise net/url
// renders the drive as an authority (file://C:/...), which SQLite rejects.
func sqliteFileURL(path string) url.URL {
	if len(path) > 1 && path[1] == ':' {
		path = "/" + path
	}
	return url.URL{Scheme: "file", Path: path}
}

func (s *Store) Close() error { return s.db.Close() }
func (s *Store) transaction(point string, fn func(*sql.Tx) error) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err = fn(tx); err != nil {
		return err
	}
	if s.fail != nil {
		if err = s.fail(point); err != nil {
			return err
		}
	}
	return tx.Commit()
}
func encode(v any) string { b, _ := json.Marshal(v); return string(b) }
func getJob(tx *sql.Tx, id string) (Job, error) {
	var j Job
	var body string
	err := tx.QueryRow(`SELECT body FROM jobs WHERE id=?`, id).Scan(&body)
	if err == nil {
		err = json.Unmarshal([]byte(body), &j)
	}
	return j, err
}
func putJob(tx *sql.Tx, j Job) error {
	_, err := tx.Exec(`INSERT INTO jobs(id,body) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body`, j.ID, encode(j))
	return err
}
func getIntent(tx *sql.Tx, id string) (Intent, error) {
	var i Intent
	var body string
	err := tx.QueryRow(`SELECT body FROM launch_intents WHERE job_id=?`, id).Scan(&body)
	if err == nil {
		err = json.Unmarshal([]byte(body), &i)
	}
	return i, err
}
func putIntent(tx *sql.Tx, i Intent) error {
	_, err := tx.Exec(`INSERT INTO launch_intents(job_id,request_id,body) VALUES(?,?,?) ON CONFLICT(job_id) DO UPDATE SET body=excluded.body`, i.JobID, i.RequestID, encode(i))
	return err
}
func getOutbox(tx *sql.Tx, id string) (Outbox, error) {
	var o Outbox
	var b string
	err := tx.QueryRow(`SELECT body FROM outbox WHERE job_id=?`, id).Scan(&b)
	if err == nil {
		err = json.Unmarshal([]byte(b), &o)
	}
	return o, err
}
func putOutbox(tx *sql.Tx, o Outbox) error {
	_, err := tx.Exec(`INSERT INTO outbox(id,job_id,body) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body`, o.ID, o.JobID, encode(o))
	return err
}
func stopped(tx *sql.Tx) error {
	var n int
	if err := tx.QueryRow(`SELECT stopped FROM bridge_meta WHERE id=1`).Scan(&n); err != nil {
		return err
	}
	if n != 0 {
		return ErrStopped
	}
	return nil
}
func quarantine(tx *sql.Tx, job, reason string, data []byte) error {
	sample := data
	if len(sample) > MaxResultBytes {
		sample = sample[:MaxResultBytes]
	}
	_, err := tx.Exec(`INSERT INTO quarantine(body) VALUES(?)`, encode(Quarantine{job, reason, digest(string(data)), string(sample)}))
	return err
}
func hold(tx *sql.Tx, j Job, reason string) error {
	j.State = "needs_reconcile"
	j.Reason = reason
	return putJob(tx, j)
}
func (s *Store) Create(campaign string, route Route, caseID string, now time.Time) (Job, error) {
	j, err := newJob(campaign, route, caseID, now)
	if err != nil {
		return j, err
	}
	err = s.transaction("create", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		old, err := getJob(tx, j.ID)
		if err == nil {
			j = old
			return nil
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if _, err = tx.Exec(`INSERT OR IGNORE INTO budgets VALUES(?,?,0,0)`, campaign, route); err != nil {
			return err
		}
		r, err := tx.Exec(`UPDATE budgets SET jobs=jobs+1 WHERE campaign=? AND candidate=? AND jobs<2`, campaign, route)
		if err != nil {
			return err
		}
		n, _ := r.RowsAffected()
		if n != 1 {
			return ErrBudget
		}
		return putJob(tx, j)
	})
	return j, err
}
func (s *Store) Job(id string) (Job, error) {
	var j Job
	err := s.transaction("read", func(tx *sql.Tx) error { var e error; j, e = getJob(tx, id); return e })
	return j, err
}
func (s *Store) Claim(id, owner string, now time.Time) (Lease, error) {
	var l Lease
	if !validID(owner) {
		return l, ErrLease
	}
	nonce, err := opaque()
	if err != nil {
		return l, err
	}
	err = s.transaction("claim", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		j, err := getJob(tx, id)
		if err != nil {
			return err
		}
		if err = j.live(now); err != nil {
			return err
		}
		if j.State != "question" && j.State != "blocked" {
			return ErrHeld
		}
		if j.Lease.ID != "" {
			return ErrLease
		}
		l = Lease{j.ID, owner, nonce, j.ExecutionEpoch, j.ExpiresAt}
		j.Lease = l
		return putJob(tx, j)
	})
	return l, err
}
func (s *Store) SwitchRoute(id string, route Route, now time.Time) error {
	if !validRoute(route) {
		return ErrHeld
	}
	return s.transaction("route", func(tx *sql.Tx) error {
		if err := stopped(tx); err != nil {
			return err
		}
		j, err := getJob(tx, id)
		if err != nil {
			return err
		}
		if err = j.live(now); err != nil {
			return err
		}
		if j.Route == route {
			return nil
		}
		o, err := getOutbox(tx, id)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if err == nil && (o.State == "sending" || o.State == "needs_reconcile") {
			return ErrHeld
		}
		j.Route = route
		j.RouteEpoch++
		if err == nil && o.State != "delivered" {
			o.Route = route
			o.RouteEpoch = j.RouteEpoch
			if err = putOutbox(tx, o); err != nil {
				return err
			}
		}
		return putJob(tx, j)
	})
}
func (s *Store) Stop(now time.Time) error {
	return s.transaction("stop", func(tx *sql.Tx) error {
		if _, err := tx.Exec(`UPDATE bridge_meta SET stopped=1 WHERE id=1`); err != nil {
			return err
		}
		rows, err := tx.Query(`SELECT body FROM jobs`)
		if err != nil {
			return err
		}
		var list []Job
		for rows.Next() {
			var body string
			var j Job
			if err = rows.Scan(&body); err != nil {
				rows.Close()
				return err
			}
			if err = json.Unmarshal([]byte(body), &j); err != nil {
				rows.Close()
				return err
			}
			list = append(list, j)
		}
		err = rows.Close()
		if err != nil {
			return err
		}
		for _, j := range list {
			if j.State != "completed" {
				if err = hold(tx, j, "bridge_stopped; running AI is not killed"); err != nil {
					return err
				}
			}
		}
		return nil
	})
}
func (s *Store) Status() ([]Job, error) { e, err := s.Export(); return e.Jobs, err }
func (s *Store) Export() (Export, error) {
	e := Export{Jobs: []Job{}, Outbox: []Outbox{}, Quarantine: []Quarantine{}, Budgets: []Budget{}, Launches: []Intent{}}
	err := s.transaction("read", func(tx *sql.Tx) error {
		for _, table := range []string{"jobs", "outbox", "quarantine", "launch_intents"} {
			rows, err := tx.Query(`SELECT body FROM ` + table + ` ORDER BY rowid`)
			if err != nil {
				return err
			}
			for rows.Next() {
				var b string
				if err = rows.Scan(&b); err != nil {
					rows.Close()
					return err
				}
				switch table {
				case "jobs":
					var j Job
					err = json.Unmarshal([]byte(b), &j)
					j.Lease.ID = ""
					e.Jobs = append(e.Jobs, j)
				case "outbox":
					var o Outbox
					err = json.Unmarshal([]byte(b), &o)
					o.AttemptID = ""
					e.Outbox = append(e.Outbox, o)
				case "quarantine":
					var q Quarantine
					err = json.Unmarshal([]byte(b), &q)
					q.Sample = "" // Raw bounded evidence remains local; do not export dispatch capabilities.
					e.Quarantine = append(e.Quarantine, q)
				case "launch_intents":
					var i Intent
					err = json.Unmarshal([]byte(b), &i)
					i.Nonce = ""
					i.Input = ""
					e.Launches = append(e.Launches, i)
				}
				if err != nil {
					rows.Close()
					return err
				}
			}
			if err = rows.Err(); err != nil {
				rows.Close()
				return err
			}
			rows.Close()
		}
		rows, err := tx.Query(`SELECT campaign,candidate,jobs,launches FROM budgets ORDER BY campaign,candidate`)
		if err != nil {
			return err
		}
		for rows.Next() {
			var b Budget
			if err = rows.Scan(&b.Campaign, &b.Candidate, &b.Jobs, &b.Launches); err != nil {
				rows.Close()
				return err
			}
			e.Budgets = append(e.Budgets, b)
		}
		if err = rows.Err(); err != nil {
			rows.Close()
			return err
		}
		rows.Close()
		var n int
		err = tx.QueryRow(`SELECT stopped FROM bridge_meta WHERE id=1`).Scan(&n)
		e.Stopped = n != 0
		return err
	})
	return e, err
}
