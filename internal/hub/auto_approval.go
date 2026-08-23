package hub

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"time"

	"gopkg.in/natefinch/lumberjack.v2"

	"many-ai-cli/internal/autoapproval"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const autoApprovalHistoryLimit = 100

type autoApprovalCandidate struct {
	At        time.Time             `json:"at"`
	SessionID int                   `json:"session_id"`
	Provider  string                `json:"provider"`
	CWD       string                `json:"cwd"`
	Summary   proto.ApprovalSummary `json:"summary"`
	Decision  autoapproval.Decision `json:"decision"`
}

type autoApprovalAuditRecord struct {
	Timestamp string `json:"timestamp"`
	SessionID int    `json:"session_id"`
	Provider  string `json:"provider"`
	RuleID    string `json:"rule_id"`
	Command   string `json:"command"`
	Risk      string `json:"risk"`
}

func (s *Server) autoApprovalEnabled() bool {
	s.cfgMu.Lock()
	enabled := s.cfg.UserPrefs.Approval.AutoApprovalEnabled
	s.cfgMu.Unlock()
	return enabled
}

func (s *Server) evaluateAutoApproval(id int, approval *nativeApproval) autoapproval.Decision {
	if approval == nil {
		return autoapproval.Decision{Reason: "承認情報がありません"}
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	cwd, provider := "", ""
	if ses != nil {
		cwd, provider = ses.CWD, ses.Provider
	}
	s.sessionsMu.Unlock()
	decision := s.evaluateAutoApprovalPolicy(approval.Summary.Command, cwd, approval.Summary.Risk)
	s.autoApprovalMu.Lock()
	candidate := autoApprovalCandidate{At: time.Now(), SessionID: id, Provider: provider, CWD: cwd, Summary: approval.Summary, Decision: decision}
	s.autoApprovalHistory = append(s.autoApprovalHistory, candidate)
	if len(s.autoApprovalHistory) > autoApprovalHistoryLimit {
		s.autoApprovalHistory = append([]autoApprovalCandidate(nil), s.autoApprovalHistory[len(s.autoApprovalHistory)-autoApprovalHistoryLimit:]...)
	}
	s.autoApprovalMu.Unlock()
	if !s.autoApprovalEnabled() {
		return autoapproval.Decision{Reason: "設定で自動承認がオフです"}
	}
	return decision
}

func (s *Server) evaluateAutoApprovalPolicy(command, cwd string, risk proto.ApprovalRiskTier) autoapproval.Decision {
	s.autoApprovalMu.Lock()
	policy := s.autoApprovalPolicy
	if policy == nil {
		policy = &autoapproval.Policy{}
	}
	decision := policy.Evaluate(command, cwd, risk)
	s.autoApprovalMu.Unlock()
	return decision
}

func autoApprovalInput(options []proto.ApprovalOption) string {
	for _, option := range options {
		if input := approvalOptionInput(option, true); input != "" {
			return input
		}
	}
	return ""
}

func normalizeApprovalOptionLabel(label string) string {
	label = strings.ToLower(strings.TrimSpace(label))
	label = strings.ReplaceAll(label, "’", "'")
	label = strings.Join(strings.Fields(label), " ")
	for _, suffix := range []string{
		" (y)", " (p)", " (n)", " (esc)", " (escape)", " (!)", " (?)", " (#)", " (recommended)",
	} {
		if strings.HasSuffix(label, suffix) {
			label = strings.TrimSpace(strings.TrimSuffix(label, suffix))
			break
		}
	}
	return label
}

func isExplicitApprovalNegativeLabel(label string) bool {
	label = normalizeApprovalOptionLabel(label)
	if label == "" {
		return false
	}
	for _, exact := range []string{
		"no", "deny", "deny once", "reject", "cancel", "skip", "abort", "decline",
		"do not allow", "don't allow", "do not run", "don't run",
		"拒否", "許可しない", "中止",
	} {
		if label == exact {
			return true
		}
	}
	for _, prefix := range []string{"no ", "deny ", "reject ", "cancel ", "skip ", "abort ", "decline "} {
		if strings.HasPrefix(label, prefix) {
			return true
		}
	}
	return false
}

func isExplicitApprovalPositiveLabel(label string) bool {
	label = normalizeApprovalOptionLabel(label)
	if label == "" || isExplicitApprovalNegativeLabel(label) {
		return false
	}
	// Persistent/session-wide choices are intentionally not automatic. The
	// whitelist rule authorizes one low-risk operation, not a permission change.
	for _, marker := range []string{"always", "don't ask", "dont ask", "all similar", "this session", "session"} {
		if strings.Contains(label, marker) {
			return false
		}
	}
	switch label {
	case "yes", "yes, allow once", "allow", "allow once", "approve", "run", "run command", "run (once)", "continue", "proceed", "yes, proceed", "yes proceed", "許可", "実行", "続行":
		return true
	default:
		return false
	}
}

func approvalOptionInput(option proto.ApprovalOption, positive bool) string {
	if positive {
		if !isExplicitApprovalPositiveLabel(option.Label) {
			return ""
		}
	} else if !isExplicitApprovalNegativeLabel(option.Label) {
		return ""
	}
	if option.SendText != "" {
		return option.SendText
	}
	if option.IsCurrent {
		return "\r"
	}
	if option.Num > 0 {
		return fmt.Sprintf("%d\r", option.Num)
	}
	return ""
}

func (s *Server) maybeAutoApprove(id int, approval *nativeApproval) bool {
	decision := s.evaluateAutoApproval(id, approval)
	if !decision.Allowed {
		return false
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	expectedCandidateKey := ""
	expectedSourceEpoch := uint64(0)
	if ses != nil {
		expectedCandidateKey = ses.nativeApprovalCandidateKey
		expectedSourceEpoch = ses.nativeApprovalSourceEpoch
	}
	s.sessionsMu.Unlock()
	if ses == nil {
		return false
	}
	if expectedCandidateKey == "" {
		expectedCandidateKey = approval.Sig
	}
	if expectedSourceEpoch == 0 {
		s.sessionsMu.Lock()
		if current := s.sessions[id]; current == ses {
			expectedSourceEpoch = ensureApprovalSourceEpochLocked(current)
		}
		s.sessionsMu.Unlock()
	}
	result, err := s.sendNativeApprovalAction(nativeApprovalActionRequest{
		sessionID:            id,
		approvalSig:          approval.Sig,
		expectedCandidateKey: expectedCandidateKey,
		expectedSourceEpoch:  expectedSourceEpoch,
		action:               oneTapApprove,
		lowRiskOnly:          true,
		ruleID:               decision.RuleID,
	})
	if err != nil {
		s.logger.Warn("auto approval skipped", "session_id", id, "rule_id", decision.RuleID, "err", err)
		return false
	}
	if !s.commitNativeApprovalAction(result) {
		s.logger.Warn("auto approval skipped: approval changed after send", "session_id", id, "rule_id", decision.RuleID)
		return false
	}
	s.writeAutoApprovalAudit(autoApprovalAuditRecord{Timestamp: time.Now().Format(time.RFC3339), SessionID: id, Provider: result.provider, RuleID: decision.RuleID, Command: sessionlog.MaskSecrets(result.approval.Summary.Command), Risk: string(result.approval.Summary.Risk)})
	summary := result.approval.Summary
	s.broadcast(proto.Message{Type: "auto_approval_applied", SessionID: id, Provider: result.provider, ApprovalSig: result.approvalSig, ApprovalSummary: &summary, Text: decision.RuleID})
	return true
}

func (s *Server) writeAutoApprovalAudit(record autoApprovalAuditRecord) {
	s.cfgMu.Lock()
	logDir, logCfg := s.cfg.Hub.LogDir, s.cfg.Log
	s.cfgMu.Unlock()
	data, err := json.Marshal(record)
	if err != nil {
		return
	}
	roller := &lumberjack.Logger{Filename: filepath.Join(logDir, "auto-approval.jsonl"), MaxSize: logCfg.MaxSizeMB, MaxBackups: logCfg.MaxBackups, Compress: logCfg.Compress}
	if _, err := roller.Write(append(data, '\n')); err != nil {
		s.logger.Warn("auto approval audit write failed", "err", err)
	}
	_ = roller.Close()
}

func (s *Server) handleAutoApprovalStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	path, _ := autoapproval.Path()
	s.autoApprovalMu.Lock()
	policy := s.autoApprovalPolicy
	warnings := []string(nil)
	rules := 0
	if policy != nil {
		warnings = append(warnings, policy.Warnings...)
		rules = len(policy.Rules)
	}
	s.autoApprovalMu.Unlock()
	writeJSON(w, map[string]any{"enabled": s.autoApprovalEnabled(), "path": path, "active_rules": rules, "warnings": warnings})
}

func (s *Server) handleAutoApprovalSimulation(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.autoApprovalMu.Lock()
	policy := s.autoApprovalPolicy
	items := append([]autoApprovalCandidate(nil), s.autoApprovalHistory...)
	s.autoApprovalMu.Unlock()
	if policy == nil {
		policy = &autoapproval.Policy{}
	}
	n := 100
	if r.URL.Query().Get("n") != "" {
		_, _ = fmt.Sscanf(r.URL.Query().Get("n"), "%d", &n)
	}
	if n > 0 && n < len(items) {
		items = items[len(items)-n:]
	}
	matched := 0
	for i := range items {
		items[i].Decision = policy.Evaluate(items[i].Summary.Command, items[i].CWD, items[i].Summary.Risk)
		if items[i].Decision.Allowed {
			matched++
		}
	}
	writeJSON(w, map[string]any{"total": len(items), "matched": matched, "items": items})
}
