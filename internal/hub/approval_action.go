package hub

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/proto"
)

const oneTapApprovalTTL = 120 * time.Second

type oneTapAction string

const (
	oneTapApprove oneTapAction = "approve"
	oneTapReject  oneTapAction = "reject"
)

var (
	errOneTapInvalid  = errors.New("one-tap approval token is invalid")
	errOneTapExpired  = errors.New("one-tap approval token has expired")
	errOneTapConsumed = errors.New("one-tap approval token was already used")
)

// oneTapApprovalClaim is intentionally small because it is delivered through
// external notification services. It never carries the Hub token or HMAC key.
type oneTapApprovalClaim struct {
	Version     int          `json:"v"`
	SessionID   int          `json:"sid"`
	ApprovalID  string       `json:"aid"`
	ApprovalSig string       `json:"sig"`
	SourceEpoch uint64       `json:"ep"`
	Action      oneTapAction `json:"act"`
	ExpiresAt   int64        `json:"exp"`
	Nonce       string       `json:"n"`
}

// oneTapApprovalManager signs purpose-limited, short-lived notification
// actions. Used nonces live only in this Hub process; a restart invalidates all
// previously issued actions because it creates a new secret.
type oneTapApprovalManager struct {
	mu       sync.Mutex
	secret   []byte
	used     map[string]time.Time
	now      func() time.Time
	tokenTTL time.Duration
}

func newOneTapApprovalManager() (*oneTapApprovalManager, error) {
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		return nil, fmt.Errorf("generate one-tap approval secret: %w", err)
	}
	return &oneTapApprovalManager{
		secret:   secret,
		used:     make(map[string]time.Time),
		now:      time.Now,
		tokenTTL: oneTapApprovalTTL,
	}, nil
}

func (m *oneTapApprovalManager) issue(sessionID int, approvalID, approvalSig string, sourceEpoch uint64, action oneTapAction) (string, error) {
	if m == nil || sessionID <= 0 || strings.TrimSpace(approvalID) == "" || strings.TrimSpace(approvalSig) == "" || sourceEpoch == 0 || !validOneTapAction(action) {
		return "", errOneTapInvalid
	}
	nonceBytes := make([]byte, 16)
	if _, err := rand.Read(nonceBytes); err != nil {
		return "", fmt.Errorf("generate one-tap approval nonce: %w", err)
	}
	m.mu.Lock()
	now := m.now()
	ttl := m.tokenTTL
	m.mu.Unlock()
	claim := oneTapApprovalClaim{
		Version: 1, SessionID: sessionID, ApprovalID: approvalID, ApprovalSig: approvalSig,
		SourceEpoch: sourceEpoch, Action: action, ExpiresAt: now.Add(ttl).Unix(), Nonce: hex.EncodeToString(nonceBytes),
	}
	payload, err := json.Marshal(claim)
	if err != nil {
		return "", fmt.Errorf("marshal one-tap approval claim: %w", err)
	}
	encoded := base64.RawURLEncoding.EncodeToString(payload)
	return encoded + "." + base64.RawURLEncoding.EncodeToString(m.mac([]byte(encoded))), nil
}

func (m *oneTapApprovalManager) verify(token string) (oneTapApprovalClaim, error) {
	if m == nil {
		return oneTapApprovalClaim{}, errOneTapInvalid
	}
	parts := strings.Split(token, ".")
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
		return oneTapApprovalClaim{}, errOneTapInvalid
	}
	provided, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || !hmac.Equal(provided, m.mac([]byte(parts[0]))) {
		return oneTapApprovalClaim{}, errOneTapInvalid
	}
	payload, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil {
		return oneTapApprovalClaim{}, errOneTapInvalid
	}
	var claim oneTapApprovalClaim
	if err := json.Unmarshal(payload, &claim); err != nil || claim.Version != 1 || claim.SessionID <= 0 || strings.TrimSpace(claim.ApprovalID) == "" || strings.TrimSpace(claim.ApprovalSig) == "" || claim.SourceEpoch == 0 || strings.TrimSpace(claim.Nonce) == "" || !validOneTapAction(claim.Action) {
		return oneTapApprovalClaim{}, errOneTapInvalid
	}
	m.mu.Lock()
	now := m.now()
	m.mu.Unlock()
	if claim.ExpiresAt <= now.Unix() {
		return oneTapApprovalClaim{}, errOneTapExpired
	}
	return claim, nil
}

// consume marks a verified claim as used. It must be called only after the
// live approval binding has been checked under the session lock.
func (m *oneTapApprovalManager) consume(claim oneTapApprovalClaim) error {
	if m == nil {
		return errOneTapInvalid
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	now := m.now()
	for nonce, expires := range m.used {
		if !expires.After(now) {
			delete(m.used, nonce)
		}
	}
	if claim.ExpiresAt <= now.Unix() {
		return errOneTapExpired
	}
	if _, exists := m.used[claim.Nonce]; exists {
		return errOneTapConsumed
	}
	m.used[claim.Nonce] = time.Unix(claim.ExpiresAt, 0)
	return nil
}

func (m *oneTapApprovalManager) mac(payload []byte) []byte {
	m.mu.Lock()
	secret := append([]byte(nil), m.secret...)
	m.mu.Unlock()
	h := hmac.New(sha256.New, secret)
	_, _ = h.Write(payload)
	return h.Sum(nil)
}

func validOneTapAction(action oneTapAction) bool {
	return action == oneTapApprove || action == oneTapReject
}

// handleOneTapApproval is deliberately separate from guard: its only
// credential is the short-lived, action-scoped token carried in the URL path.
// It still applies the normal Host and same-origin checks to prevent the token
// endpoint from becoming a DNS-rebinding or CSRF primitive.
func (s *Server) handleOneTapApproval(w http.ResponseWriter, r *http.Request) {
	token := strings.TrimPrefix(r.URL.Path, "/api/approval-action/")
	if token == "" || strings.Contains(token, "/") {
		writeJSONError(w, http.StatusUnauthorized, "invalid_action_token", "invalid action token")
		return
	}
	claim, err := s.oneTapApprovals.verify(token)
	if err != nil {
		s.writeOneTapApprovalError(w, err)
		return
	}
	if !requireMethod(w, r, http.MethodPost) || !s.requireAllowedHubHost(w, r) || !s.requireAllowedRequestOrigin(w, r) {
		return
	}
	if err := s.applyOneTapApproval(claim); err != nil {
		s.writeOneTapApprovalError(w, err)
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session_id": claim.SessionID, "action": claim.Action})
}

func (s *Server) writeOneTapApprovalError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, errOneTapConsumed):
		writeJSONError(w, http.StatusConflict, "action_already_used", "action already used")
	case errors.Is(err, errOneTapExpired), errors.Is(err, errOneTapInvalid):
		writeJSONError(w, http.StatusUnauthorized, "invalid_action_token", "invalid or expired action token")
	case errors.Is(err, errOneTapHighRisk):
		writeJSONError(w, http.StatusForbidden, "high_risk_requires_in_app_confirmation", "open Hub to confirm this high-risk approval")
	case errors.Is(err, errOneTapNoApproval):
		writeJSONError(w, http.StatusConflict, "approval_not_pending", "approval is no longer pending")
	default:
		writeJSONError(w, http.StatusConflict, "action_not_applied", "approval action was not applied")
	}
}

var (
	errOneTapHighRisk            = errors.New("high-risk approval requires in-app confirmation")
	errOneTapNoApproval          = errors.New("approval is not pending")
	errOneTapNoInput             = errors.New("approval action input is unavailable")
	errApprovalActionRuleChanged = errors.New("approval rule no longer matches")
)

type nativeApprovalActionRequest struct {
	sessionID            int
	approvalSig          string
	expectedCandidateKey string
	expectedSourceEpoch  uint64
	expectedWrapper      *wrapperConn
	action               oneTapAction
	lowRiskOnly          bool
	ruleID               string
}

type nativeApprovalActionResult struct {
	sessionID    int
	approvalSig  string
	candidateKey string
	sourceEpoch  uint64
	provider     string
	wrapper      *wrapperConn
	approval     nativeApproval
	input        string
}

// sendNativeApprovalAction validates the live native prompt while holding the
// session's input lock, then sends only that validated option. Callers commit
// the consumed state after this returns successfully.
func (s *Server) sendNativeApprovalAction(req nativeApprovalActionRequest) (nativeApprovalActionResult, error) {
	if req.sessionID <= 0 || strings.TrimSpace(req.approvalSig) == "" || !validOneTapAction(req.action) {
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	s.sessionsMu.Lock()
	ses := s.sessions[req.sessionID]
	var inputMu *sync.Mutex
	if ses != nil {
		inputMu = ses.inputMu
	}
	s.sessionsMu.Unlock()
	if ses == nil || inputMu == nil {
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}

	inputMu.Lock()
	defer inputMu.Unlock()

	s.sessionsMu.Lock()
	if s.sessions[req.sessionID] != ses || ses.vt == nil || ses.nativeApprovalSig != req.approvalSig {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	approval := detectNativeApproval(ses.Provider, ses.vt.Lines())
	if approval == nil || approval.Sig != req.approvalSig {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	candidateKey := approvalCandidateKeyWithContext(ses.Provider, approval.Kind, approval.Question, approval.Context, approval.Options)
	sourceEpoch := ensureApprovalSourceEpochLocked(ses)
	if req.expectedCandidateKey != "" && candidateKey != req.expectedCandidateKey {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	if req.expectedSourceEpoch != 0 && sourceEpoch != req.expectedSourceEpoch {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	if req.action == oneTapApprove && approval.Summary.Risk == proto.ApprovalRiskHigh {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapHighRisk
	}
	if req.action == oneTapApprove && req.lowRiskOnly && approval.Summary.Risk != proto.ApprovalRiskLow {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	wc := s.wrappers[req.sessionID]
	if wc == nil || (req.expectedWrapper != nil && wc != req.expectedWrapper) {
		s.sessionsMu.Unlock()
		return nativeApprovalActionResult{}, errOneTapNoInput
	}
	command, cwd, risk := approval.Summary.Command, ses.CWD, approval.Summary.Risk
	provider := ses.Provider
	s.sessionsMu.Unlock()

	if req.ruleID != "" {
		if !s.autoApprovalEnabled() {
			return nativeApprovalActionResult{}, errApprovalActionRuleChanged
		}
		decision := s.evaluateAutoApprovalPolicy(command, cwd, risk)
		if !decision.Allowed || decision.RuleID != req.ruleID {
			return nativeApprovalActionResult{}, errApprovalActionRuleChanged
		}
	}
	if !s.nativeApprovalActionStillCurrent(req.sessionID, ses, req.approvalSig, candidateKey, sourceEpoch, wc) {
		return nativeApprovalActionResult{}, errOneTapNoApproval
	}
	var input string
	if req.action == oneTapApprove {
		input = autoApprovalInput(approval.Options)
	} else {
		input = oneTapRejectInput(approval.Options)
	}
	if input == "" {
		return nativeApprovalActionResult{}, errOneTapNoInput
	}
	result := nativeApprovalActionResult{
		sessionID: req.sessionID, approvalSig: approval.Sig, candidateKey: candidateKey,
		sourceEpoch: sourceEpoch, provider: provider, wrapper: wc, approval: *approval, input: input,
	}
	if rem := s.trySendInputToWrapper(req.sessionID, result.wrapper, result.input); rem != "" {
		return nativeApprovalActionResult{}, errOneTapNoInput
	}
	return result, nil
}

func (s *Server) nativeApprovalActionStillCurrent(sessionID int, expectedSession *session, approvalSig, candidateKey string, sourceEpoch uint64, expectedWrapper *wrapperConn) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	return ses != nil && ses == expectedSession && ses.nativeApprovalSig == approvalSig &&
		(ses.nativeApprovalCandidateKey == "" || ses.nativeApprovalCandidateKey == candidateKey) &&
		(ses.nativeApprovalSourceEpoch == 0 || ses.nativeApprovalSourceEpoch == sourceEpoch) && s.wrappers[sessionID] == expectedWrapper
}

// commitNativeApprovalAction changes native approval state only after the
// corresponding PTY input was accepted by the wrapper. The final check is
// atomic with the state transition so a newer prompt cannot be consumed.
func (s *Server) commitNativeApprovalAction(result nativeApprovalActionResult) bool {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[result.sessionID]
	if ses == nil || ses.nativeApprovalSig != result.approvalSig ||
		(ses.nativeApprovalCandidateKey != "" && ses.nativeApprovalCandidateKey != result.candidateKey) ||
		(ses.nativeApprovalSourceEpoch != 0 && ses.nativeApprovalSourceEpoch != result.sourceEpoch) || s.wrappers[result.sessionID] != result.wrapper {
		s.sessionsMu.Unlock()
		return false
	}
	sourceEpoch := markApprovalConsumedLocked(ses, result.candidateKey, result.approvalSig)
	provider := ses.Provider
	ses.nativeApprovalSig = ""
	ses.nativeApprovalCandidateKey = ""
	ses.nativeApprovalCandidateShape = ""
	ses.nativeApprovalSourceEpoch = 0
	ses.nativeApprovalClearMisses = 0
	clearMsg := proto.Message{
		Type: "approval_cleared", SessionID: result.sessionID, Provider: provider,
		ApprovalSig: result.approvalSig, ApprovalCandidateKey: result.candidateKey,
		ApprovalCandidateShape: ses.approvalConsumedCandidateShape,
		ApprovalSourceEpoch:    sourceEpoch, ApprovalSource: approvalSourceGoVT,
	}
	s.sessionsMu.Unlock()
	if s.sessionStore != nil {
		s.sessionStore.StoreApprovalConsumed(result.sessionID, result.approvalSig, result.input, now)
	}
	s.broadcast(clearMsg)
	return true
}

func (s *Server) applyOneTapApproval(claim oneTapApprovalClaim) error {
	if claim.ApprovalID != claim.ApprovalSig {
		return errOneTapNoApproval
	}
	result, err := s.sendNativeApprovalAction(nativeApprovalActionRequest{
		sessionID: claim.SessionID, approvalSig: claim.ApprovalSig,
		expectedSourceEpoch: claim.SourceEpoch,
		action:              claim.Action,
	})
	if err != nil {
		return err
	}
	if err := s.oneTapApprovals.consume(claim); err != nil {
		return err
	}
	if !s.commitNativeApprovalAction(result) {
		return errOneTapNoApproval
	}
	return nil
}

func oneTapRejectInput(options []proto.ApprovalOption) string {
	for _, option := range options {
		if input := approvalOptionInput(option, false); input != "" {
			return input
		}
	}
	return ""
}
