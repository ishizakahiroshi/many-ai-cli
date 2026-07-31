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
	s.autoApprovalMu.Lock()
	policy := s.autoApprovalPolicy
	if policy == nil {
		policy = &autoapproval.Policy{}
	}
	decision := policy.Evaluate(approval.Summary.Command, cwd, approval.Summary.Risk)
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

func autoApprovalInput(options []proto.ApprovalOption) string {
	for _, option := range options {
		label := strings.ToLower(strings.TrimSpace(option.Label))
		positive := strings.Contains(label, "allow once") || strings.Contains(label, "allow") || strings.Contains(label, "approve") || strings.Contains(label, "run") || strings.Contains(label, "continue") || strings.Contains(label, "yes") || strings.Contains(option.Label, "許可") || strings.Contains(option.Label, "実行") || strings.Contains(option.Label, "続行")
		if !positive {
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

func (s *Server) maybeAutoApprove(id int, approval *nativeApproval) bool {
	decision := s.evaluateAutoApproval(id, approval)
	if !decision.Allowed {
		return false
	}
	s.sessionsMu.Lock()
	wc := s.wrappers[id]
	ses := s.sessions[id]
	s.sessionsMu.Unlock()
	if wc == nil || ses == nil {
		return false
	}
	input := autoApprovalInput(approval.Options)
	if input == "" {
		s.logger.Warn("auto approval skipped: allow option not recognized", "session_id", id, "rule_id", decision.RuleID)
		return false
	}
	// Consume before writing input so redraws cannot send the same approval twice.
	s.markNativeApprovalConsumed(proto.Message{SessionID: id, ApprovalSig: approval.Sig, SentText: input})
	s.submitInput(wc, id, input)
	s.writeAutoApprovalAudit(autoApprovalAuditRecord{Timestamp: time.Now().Format(time.RFC3339), SessionID: id, Provider: ses.Provider, RuleID: decision.RuleID, Command: sessionlog.MaskSecrets(approval.Summary.Command), Risk: string(approval.Summary.Risk)})
	s.broadcast(proto.Message{Type: "auto_approval_applied", SessionID: id, Provider: ses.Provider, ApprovalSig: approval.Sig, ApprovalSummary: &approval.Summary, Text: decision.RuleID})
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
