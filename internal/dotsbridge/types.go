// Package dotsbridge is an offline-only, synthetic C2 bridge prototype.
// It never starts processes, reads credentials, or connects to a live service.
package dotsbridge

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"time"
)

type Route string

const (
	RouteM Route = "mcp"
	RouteS Route = "socket"
)
const MaxResultBytes = 8192

var (
	ErrConflict = errors.New("identity payload conflict; needs reconciliation")
	ErrLease    = errors.New("execution lease is not current")
	ErrHeld     = errors.New("operation held for reconciliation")
	ErrExpired  = errors.New("job deadline reached")
	ErrStopped  = errors.New("bridge is stopped")
	ErrBudget   = errors.New("cumulative candidate budget exhausted")
	ErrConsumer = errors.New("consumer identity is not verified for this dispatch")
)
var idPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$`)

func validID(s string) bool   { return idPattern.MatchString(s) }
func validRoute(r Route) bool { return r == RouteM || r == RouteS }
func opaque() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}
func digest(v any) string {
	b, _ := json.Marshal(v)
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}

type Envelope struct {
	CampaignID       string    `json:"campaign_id"`
	CaseID           string    `json:"case_id"`
	JobID            string    `json:"job_id"`
	LogicalMessageID string    `json:"logical_message_id"`
	Phase            string    `json:"phase"`
	QuestionID       string    `json:"question_id"`
	Sequence         int64     `json:"sequence"`
	Source           Route     `json:"source"`
	SourceEventID    string    `json:"source_event_id"`
	Sender           string    `json:"sender"`
	Origin           string    `json:"origin"`
	Thread           string    `json:"thread"`
	RouteEpoch       int64     `json:"route_epoch"`
	ExecutionEpoch   int64     `json:"execution_epoch"`
	Payload          string    `json:"payload"`
	PayloadHash      string    `json:"payload_hash"`
	ReceivedAt       time.Time `json:"received_at"`
	ExpiresAt        time.Time `json:"expires_at"`
}

func (e Envelope) Hash() string {
	return digest(struct {
		Campaign, Case, Job, Logical, Phase, Question string
		Sequence, Execution                           int64
		Payload                                       string
	}{e.CampaignID, e.CaseID, e.JobID, e.LogicalMessageID, e.Phase, e.QuestionID, e.Sequence, e.ExecutionEpoch, e.Payload})
}

type Lease struct {
	JobID string    `json:"job_id"`
	Owner string    `json:"owner"`
	ID    string    `json:"id"`
	Epoch int64     `json:"epoch"`
	Until time.Time `json:"until"`
}
type Job struct {
	ID             string    `json:"job_id"`
	Campaign       string    `json:"campaign_id"`
	Case           string    `json:"case_id"`
	Candidate      Route     `json:"candidate"`
	Route          Route     `json:"route"`
	RouteEpoch     int64     `json:"route_epoch"`
	ExecutionEpoch int64     `json:"execution_epoch"`
	State          string    `json:"state"`
	Reason         string    `json:"reason,omitempty"`
	QuestionID     string    `json:"question_id,omitempty"`
	Sequence       int64     `json:"sequence"`
	CreatedAt      time.Time `json:"created_at"`
	ExpiresAt      time.Time `json:"expires_at"`
	Lease          Lease     `json:"lease"`
}

func (j Job) thread(r Route) string { return "synthetic:" + string(r) + ":" + j.ID }
func (j Job) exact(l Lease, now time.Time) bool {
	return j.Lease == l && l.ID != "" && l.Owner != "" && l.Epoch == j.ExecutionEpoch && now.Before(l.Until)
}
func (j Job) live(now time.Time) error {
	if !now.Before(j.ExpiresAt) {
		return ErrExpired
	}
	if j.State == "needs_reconcile" || j.State == "completed" {
		return ErrHeld
	}
	return nil
}

type Run struct {
	ID              string `json:"id"`
	RoutineID       string `json:"routine_id"`
	RequestID       string `json:"request_id"`
	HubInstanceID   string `json:"hub_instance_id"`
	SessionID       int    `json:"session_id"`
	Status          string `json:"status"`
	ResultAvailable bool   `json:"result_available"`
}

func (r Run) active() bool {
	return r.Status == "starting" || r.Status == "running" || r.Status == "waiting"
}
func (r Run) identity() runIdentity {
	return runIdentity{r.ID, r.RoutineID, r.RequestID, r.HubInstanceID, r.SessionID}
}

type runIdentity struct {
	RunID, RoutineID, RequestID, Instance string
	SessionID                             int
}

func (r Run) valid() bool {
	return r.ID != "" && r.RoutineID != "" && r.RequestID != "" && r.HubInstanceID != "" && r.SessionID > 0
}

type Intent struct {
	JobID          string `json:"job_id"`
	QuestionID     string `json:"question_id"`
	ExecutionEpoch int64  `json:"execution_epoch"`
	RequestID      string `json:"request_id"`
	RoutineID      string `json:"routine_id"`
	Run            Run    `json:"run"`
	State          string `json:"state"`
	Nonce          string `json:"nonce"`
	Consumed       bool   `json:"consumed"`
	Input          string `json:"input"`
}
type Input struct {
	JobID          string `json:"job_id"`
	QuestionID     string `json:"question_id"`
	ExecutionEpoch int64  `json:"execution_epoch"`
	RequestID      string `json:"request_id"`
	RunID          string `json:"run_id"`
	HubInstanceID  string `json:"hub_instance_id"`
	SessionID      int    `json:"session_id"`
	DispatchNonce  string `json:"dispatch_nonce"`
	Task           string `json:"task"`
}
type Answer struct {
	Input
	Answer string `json:"answer"`
}

type Outbox struct {
	ID         string    `json:"logical_message_id"`
	JobID      string    `json:"job_id"`
	Payload    string    `json:"payload"`
	Route      Route     `json:"route"`
	RouteEpoch int64     `json:"route_epoch"`
	State      string    `json:"state"`
	Attempts   int       `json:"attempts"`
	AttemptID  string    `json:"attempt_id,omitempty"`
	NotBefore  time.Time `json:"not_before"`
}
type DeliveryOutcome string

const (
	Delivered DeliveryOutcome = "delivered"
	NotSent   DeliveryOutcome = "not_sent"
	Uncertain DeliveryOutcome = "uncertain"
)

type Receipt struct {
	Ack         bool   `json:"ack"`
	Disposition string `json:"disposition"`
}
type Quarantine struct {
	JobID  string `json:"job_id"`
	Reason string `json:"reason"`
	Hash   string `json:"hash"`
	Sample string `json:"sample"`
}
type Budget struct {
	Campaign  string `json:"campaign"`
	Candidate Route  `json:"candidate"`
	Jobs      int    `json:"jobs"`
	Launches  int    `json:"launches"`
}
type Export struct {
	Jobs       []Job        `json:"jobs"`
	Outbox     []Outbox     `json:"outbox"`
	Quarantine []Quarantine `json:"quarantine"`
	Budgets    []Budget     `json:"budgets"`
	Launches   []Intent     `json:"launches"`
	Stopped    bool         `json:"stopped"`
}

func requestID(j Job) string { return "c2-" + digest([]any{j.ID, j.QuestionID, j.ExecutionEpoch}) }
func newJob(campaign string, route Route, caseID string, now time.Time) (Job, error) {
	if !validID(campaign) || !validRoute(route) || (caseID != "color" && caseID != "recovery") {
		return Job{}, fmt.Errorf("invalid synthetic campaign, candidate or case")
	}
	id := campaign + ":" + string(route) + ":" + caseID
	if !validID(id) {
		return Job{}, fmt.Errorf("synthetic job ID too long")
	}
	return Job{ID: id, Campaign: campaign, Case: caseID, Candidate: route, Route: route, RouteEpoch: 1, ExecutionEpoch: 1, State: "created", CreatedAt: now, ExpiresAt: now.Add(10 * time.Minute)}, nil
}
