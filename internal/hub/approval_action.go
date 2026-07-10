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

func (m *oneTapApprovalManager) issue(sessionID int, approvalID, approvalSig string, action oneTapAction) (string, error) {
	if m == nil || sessionID <= 0 || strings.TrimSpace(approvalID) == "" || strings.TrimSpace(approvalSig) == "" || !validOneTapAction(action) {
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
		Action: action, ExpiresAt: now.Add(ttl).Unix(), Nonce: hex.EncodeToString(nonceBytes),
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
	if err := json.Unmarshal(payload, &claim); err != nil || claim.Version != 1 || claim.SessionID <= 0 || strings.TrimSpace(claim.ApprovalID) == "" || strings.TrimSpace(claim.ApprovalSig) == "" || strings.TrimSpace(claim.Nonce) == "" || !validOneTapAction(claim.Action) {
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
	errOneTapHighRisk   = errors.New("high-risk approval requires in-app confirmation")
	errOneTapNoApproval = errors.New("approval is not pending")
	errOneTapNoInput    = errors.New("approval action input is unavailable")
)

func (s *Server) applyOneTapApproval(claim oneTapApprovalClaim) error {
	s.sessionsMu.Lock()
	ses := s.sessions[claim.SessionID]
	s.sessionsMu.Unlock()
	if ses == nil || ses.inputMu == nil {
		return errOneTapNoApproval
	}
	ses.inputMu.Lock()
	var (
		wc       *wrapperConn
		input    string
		provider string
		clearMsg *proto.Message
	)
	s.sessionsMu.Lock()
	if ses.nativeApprovalSig != claim.ApprovalSig || claim.ApprovalID != claim.ApprovalSig {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapNoApproval
	}
	if ses.vt == nil {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapNoApproval
	}
	approval := detectNativeApproval(ses.Provider, ses.vt.Lines())
	if approval == nil || approval.Sig != claim.ApprovalSig {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapNoApproval
	}
	if claim.Action == oneTapApprove && approval.Summary.Risk == proto.ApprovalRiskHigh {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapHighRisk
	}
	if claim.Action == oneTapApprove {
		input = oneTapApproveInput(approval.Options)
	} else {
		input = oneTapRejectInput(approval.Options)
	}
	if input == "" {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapNoInput
	}
	if err := s.oneTapApprovals.consume(claim); err != nil {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return err
	}
	wc = s.wrappers[claim.SessionID]
	if wc == nil {
		s.sessionsMu.Unlock()
		ses.inputMu.Unlock()
		return errOneTapNoInput
	}
	provider = ses.Provider
	ses.nativeApprovalConsumed = claim.ApprovalSig
	ses.nativeApprovalConsumedAt = time.Now()
	ses.nativeApprovalSig = ""
	ses.nativeApprovalClearMisses = 0
	clearMsg = &proto.Message{Type: "approval_cleared", SessionID: claim.SessionID, Provider: provider, ApprovalSig: claim.ApprovalSig, ApprovalSource: approvalSourceGoVT}
	s.sessionsMu.Unlock()
	// Keep the state transition and PTY write serialized with normal user input.
	if rem := s.trySendInput(wc, claim.SessionID, input); rem != "" {
		ses.inputMu.Unlock()
		return errOneTapNoInput
	}
	ses.inputMu.Unlock()
	if s.sessionStore != nil {
		s.sessionStore.StoreApprovalConsumed(claim.SessionID, claim.ApprovalSig, input, time.Now())
	}
	s.broadcast(*clearMsg)
	return nil
}

func oneTapApproveInput(options []proto.ApprovalOption) string {
	return autoApprovalInput(options)
}

func oneTapRejectInput(options []proto.ApprovalOption) string {
	for _, option := range options {
		label := strings.ToLower(strings.TrimSpace(option.Label))
		negative := strings.Contains(label, "reject") || strings.Contains(label, "deny") || strings.Contains(label, "cancel") || strings.Contains(label, "skip") || strings.Contains(label, "no") || strings.Contains(option.Label, "拒否") || strings.Contains(option.Label, "許可しない") || strings.Contains(option.Label, "中止")
		if !negative {
			continue
		}
		if option.SendText != "" {
			return option.SendText
		}
		if option.IsCurrent {
			return "\r"
		}
	}
	return ""
}
