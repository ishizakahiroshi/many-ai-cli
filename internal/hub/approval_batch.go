package hub

import (
	"encoding/json"
	"net/http"
	"strings"

	"many-ai-cli/internal/autoapproval"
	"many-ai-cli/internal/proto"
)

// approvalBatchRequest deliberately identifies a normalized signature, never
// an arbitrary command supplied by the browser.
type approvalBatchRequest struct {
	Signature string `json:"signature"`
	Action    string `json:"action"`
	SessionID int    `json:"session_id,omitempty"`
}

type pendingNativeApproval struct {
	id       int
	provider string
	cwd      string
	approval *nativeApproval
	wrapper  *wrapperConn
}

func approvalBatchSignature(provider, cwd string, summary proto.ApprovalSummary) string {
	return strings.ToLower(strings.TrimSpace(provider)) + "\x00" +
		strings.TrimSpace(summary.Command) + "\x00" + string(summary.Risk) + "\x00" + strings.TrimSpace(cwd)
}

func (s *Server) pendingNativeApprovals() []pendingNativeApproval {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	items := make([]pendingNativeApproval, 0)
	for id, ses := range s.sessions {
		if ses == nil || ses.nativeApprovalSig == "" || ses.vt == nil {
			continue
		}
		approval := detectNativeApproval(ses.Provider, ses.vt.Lines())
		if approval == nil || approval.Sig != ses.nativeApprovalSig {
			continue
		}
		items = append(items, pendingNativeApproval{id: id, provider: ses.Provider, cwd: ses.CWD, approval: approval, wrapper: s.wrappers[id]})
	}
	return items
}

func (s *Server) handleApprovalBatch(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req approvalBatchRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil || (strings.TrimSpace(req.Signature) == "" && req.Action != "deny_session") {
		writeJSONError(w, http.StatusBadRequest, "invalid_batch_request", "signature is required")
		return
	}
	items := s.pendingNativeApprovals()
	matched := make([]pendingNativeApproval, 0, len(items))
	for _, item := range items {
		if (req.Action == "deny_session" && item.id == req.SessionID) || approvalBatchSignature(item.provider, item.cwd, item.approval.Summary) == req.Signature {
			matched = append(matched, item)
		}
	}
	if req.Action == "auto_rule" {
		if len(matched) == 0 || matched[0].approval.Summary.Risk != proto.ApprovalRiskLow {
			writeJSONError(w, http.StatusForbidden, "auto_rule_requires_low_risk", "only a pending low-risk approval can be added")
			return
		}
		rule, err := autoapproval.AddRule(matched[0].approval.Summary.Command, matched[0].cwd)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "auto_rule_not_added", err.Error())
			return
		}
		policy, _ := autoapproval.Load()
		s.autoApprovalMu.Lock()
		s.autoApprovalPolicy = policy
		s.autoApprovalMu.Unlock()
		writeJSON(w, map[string]any{"ok": true, "matched": len(matched), "rule": rule})
		return
	}
	if req.Action != "approve" {
		if req.Action == "deny_session" && req.SessionID > 0 {
			applied := 0
			for _, item := range matched {
				if input := oneTapRejectInput(item.approval.Options); input != "" && item.wrapper != nil {
					s.markNativeApprovalConsumed(proto.Message{SessionID: item.id, ApprovalSig: item.approval.Sig, SentText: input})
					s.submitInput(item.wrapper, item.id, input)
					applied++
				}
			}
			writeJSON(w, map[string]any{"ok": true, "matched": len(matched), "applied": applied})
			return
		}
		writeJSONError(w, http.StatusBadRequest, "invalid_batch_action", "unsupported batch action")
		return
	}
	applied := 0
	for _, item := range matched {
		// High-risk approvals are never batch-approved, even if the client was
		// stale or malicious. Mid risk stays manual as well.
		if item.approval.Summary.Risk != proto.ApprovalRiskLow {
			continue
		}
		input := autoApprovalInput(item.approval.Options)
		if input == "" || item.wrapper == nil {
			continue
		}
		s.markNativeApprovalConsumed(proto.Message{SessionID: item.id, ApprovalSig: item.approval.Sig, SentText: input})
		s.submitInput(item.wrapper, item.id, input)
		applied++
	}
	writeJSON(w, map[string]any{"ok": true, "matched": len(matched), "applied": applied})
}
