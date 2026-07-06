package hub

import (
	"context"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"many-ai-cli/internal/wrapper"
)

type approvalRuleMode string

const (
	approvalRuleModeClaudeImport approvalRuleMode = "claude_import"
	approvalRuleModeSharedBlock  approvalRuleMode = "shared_block"
)

type approvalRuleTarget struct {
	Path      string
	Providers []string
	Mode      approvalRuleMode
}

type approvalRuleSessionSnap struct {
	provider string
	cwd      string
}

func (t approvalRuleTarget) wrapperProvider() string {
	if t.Mode == approvalRuleModeClaudeImport {
		return "claude"
	}
	return "codex"
}

func approvalTargetKey(path string) string {
	key := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		key = strings.ToLower(key)
	}
	return key
}

func mergeApprovalRuleTargets(targets []approvalRuleTarget) []approvalRuleTarget {
	byKey := make(map[string]approvalRuleTarget, len(targets))
	order := make([]string, 0, len(targets))
	for _, target := range targets {
		if strings.TrimSpace(target.Path) == "" {
			continue
		}
		target.Path = filepath.Clean(target.Path)
		key := approvalTargetKey(target.Path)
		existing, ok := byKey[key]
		if !ok {
			target.Providers = uniqueProviders(target.Providers)
			byKey[key] = target
			order = append(order, key)
			continue
		}
		existing.Providers = uniqueProviders(append(existing.Providers, target.Providers...))
		byKey[key] = existing
	}
	out := make([]approvalRuleTarget, 0, len(order))
	for _, key := range order {
		out = append(out, byKey[key])
	}
	return out
}

// isAIProvider は provider が AI セッション（承認・chat history・done summary 等が
// 適用される）かどうかを返す。Shell セッションは対象外。
func isAIProvider(provider string) bool {
	switch provider {
	case "claude", "codex", "copilot", "cursor-agent", "opencode", "grok":
		return true
	default:
		return false
	}
}

func uniqueProviders(providers []string) []string {
	seen := make(map[string]struct{}, len(providers))
	for _, provider := range providers {
		provider = strings.TrimSpace(provider)
		if provider == "" {
			continue
		}
		seen[provider] = struct{}{}
	}
	order := []string{"claude", "codex", "copilot", "cursor-agent"}
	out := make([]string, 0, len(seen))
	for _, provider := range order {
		if _, ok := seen[provider]; ok {
			out = append(out, provider)
			delete(seen, provider)
		}
	}
	for _, provider := range providers {
		provider = strings.TrimSpace(provider)
		if _, ok := seen[provider]; ok {
			out = append(out, provider)
			delete(seen, provider)
		}
	}
	return out
}

func instructionRootForCWD(cwd string) string {
	cwd = strings.TrimSpace(cwd)
	if cwd == "" {
		return ""
	}
	ctx, cancel := context.WithTimeout(context.Background(), branchLookupTimeout)
	defer cancel()
	if out, err := runGit(ctx, cwd, "rev-parse", "--show-toplevel"); err == nil {
		if root := strings.TrimSpace(string(out)); root != "" {
			return filepath.Clean(root)
		}
	}
	if abs, err := filepath.Abs(cwd); err == nil {
		return filepath.Clean(abs)
	}
	return filepath.Clean(cwd)
}

func codexAgentsPath() string {
	home, _ := os.UserHomeDir()
	codexHome := os.Getenv("CODEX_HOME")
	if codexHome == "" {
		codexHome = filepath.Join(home, ".codex")
	}
	return filepath.Join(codexHome, "AGENTS.md")
}

func projectAgentsApprovalRuleTarget(provider, cwd string) []approvalRuleTarget {
	root := instructionRootForCWD(cwd)
	if root == "" {
		return nil
	}
	return []approvalRuleTarget{{
		Path:      filepath.Join(root, "AGENTS.md"),
		Providers: []string{provider},
		Mode:      approvalRuleModeSharedBlock,
	}}
}

func providerApprovalRuleTargets(provider, cwd string) []approvalRuleTarget {
	home, _ := os.UserHomeDir()
	switch provider {
	case "claude":
		return []approvalRuleTarget{{
			Path:      filepath.Join(home, ".claude", "CLAUDE.md"),
			Providers: []string{"claude"},
			Mode:      approvalRuleModeClaudeImport,
		}}
	case "codex":
		return []approvalRuleTarget{{
			Path:      codexAgentsPath(),
			Providers: []string{"codex"},
			Mode:      approvalRuleModeSharedBlock,
		}}
	case "copilot", "cursor-agent", "grok":
		// grok (Grok Build) は CLAUDE.md / AGENTS.md を両方ネイティブに読む
		// （Claude Code 互換 harness）。copilot / cursor-agent と同じく
		// プロジェクト直下 AGENTS.md へ共有ブロックを注入する。
		return projectAgentsApprovalRuleTarget(provider, cwd)
	default:
		return nil
	}
}

func legacyApprovalRuleTargets(provider, cwd string) []approvalRuleTarget {
	if provider != "codex" {
		return nil
	}
	return projectAgentsApprovalRuleTarget("codex", cwd)
}

func (s *Server) approvalRulesEnabled() bool {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Approval.Enabled
}

func (s *Server) activeApprovalRuleTargets() []approvalRuleTarget {
	snaps := s.activeApprovalRuleSessionSnaps()

	targets := make([]approvalRuleTarget, 0, len(snaps))
	for _, snap := range snaps {
		targets = append(targets, providerApprovalRuleTargets(snap.provider, snap.cwd)...)
	}
	return mergeApprovalRuleTargets(targets)
}

func (s *Server) activeLegacyApprovalRuleTargets() []approvalRuleTarget {
	snaps := s.activeApprovalRuleSessionSnaps()

	targets := make([]approvalRuleTarget, 0, len(snaps))
	for _, snap := range snaps {
		targets = append(targets, legacyApprovalRuleTargets(snap.provider, snap.cwd)...)
	}
	return mergeApprovalRuleTargets(targets)
}

func (s *Server) activeApprovalRuleSessionSnaps() []approvalRuleSessionSnap {
	s.sessionsMu.Lock()
	snaps := make([]approvalRuleSessionSnap, 0, len(s.sessions))
	for id, ses := range s.sessions {
		if ses == nil || s.wrappers[id] == nil {
			continue
		}
		if ses.State == "completed" || ses.State == "error" || ses.State == "disconnected" {
			continue
		}
		// Shell session は approval rule injection / cleanup の対象外
		if !isAIProvider(ses.Provider) {
			continue
		}
		snaps = append(snaps, approvalRuleSessionSnap{provider: ses.Provider, cwd: ses.CWD})
	}
	s.sessionsMu.Unlock()
	return snaps
}

func (s *Server) rememberApprovalTargets(targets []approvalRuleTarget) {
	if len(targets) == 0 {
		return
	}
	s.approvalRulesMu.Lock()
	defer s.approvalRulesMu.Unlock()
	if s.approvalRuleTargets == nil {
		s.approvalRuleTargets = map[string]approvalRuleTarget{}
	}
	for _, target := range mergeApprovalRuleTargets(targets) {
		key := approvalTargetKey(target.Path)
		if existing, ok := s.approvalRuleTargets[key]; ok {
			existing.Providers = uniqueProviders(append(existing.Providers, target.Providers...))
			s.approvalRuleTargets[key] = existing
			continue
		}
		s.approvalRuleTargets[key] = target
	}
}

func (s *Server) knownApprovalTargets() []approvalRuleTarget {
	s.approvalRulesMu.Lock()
	defer s.approvalRulesMu.Unlock()
	targets := make([]approvalRuleTarget, 0, len(s.approvalRuleTargets))
	for _, target := range s.approvalRuleTargets {
		targets = append(targets, target)
	}
	return mergeApprovalRuleTargets(targets)
}

func (s *Server) forgetApprovalTargets(targets []approvalRuleTarget) {
	if len(targets) == 0 {
		return
	}
	s.approvalRulesMu.Lock()
	defer s.approvalRulesMu.Unlock()
	for _, target := range targets {
		delete(s.approvalRuleTargets, approvalTargetKey(target.Path))
	}
}

func (s *Server) injectApprovalTargets(targets []approvalRuleTarget) {
	targets = mergeApprovalRuleTargets(targets)
	if len(targets) == 0 {
		return
	}
	if err := wrapper.SyncRulesFile(); err != nil {
		s.logger.Warn("sync rules file failed", "err", err)
		return
	}
	var injected []approvalRuleTarget
	for _, target := range targets {
		provider := target.wrapperProvider()
		if err := wrapper.InjectRules(provider, target.Path); err != nil {
			s.logger.Warn("inject rules failed", "providers", strings.Join(target.Providers, ","), "path", target.Path, "err", err)
			continue
		}
		s.logger.Debug("inject rules ok", "providers", strings.Join(target.Providers, ","), "path", target.Path)
		injected = append(injected, target)
	}
	s.rememberApprovalTargets(injected)
}

func (s *Server) removeApprovalTargets(targets []approvalRuleTarget) {
	targets = mergeApprovalRuleTargets(targets)
	if len(targets) == 0 {
		return
	}
	var removed []approvalRuleTarget
	for _, target := range targets {
		provider := target.wrapperProvider()
		if err := wrapper.RemoveRules(provider, target.Path); err != nil {
			s.logger.Warn("remove rules failed", "providers", strings.Join(target.Providers, ","), "path", target.Path, "err", err)
			continue
		}
		removed = append(removed, target)
	}
	s.forgetApprovalTargets(removed)
}

func (s *Server) injectApprovalRules() {
	s.injectApprovalTargets(s.activeApprovalRuleTargets())
	s.removeInactiveApprovalRules(s.activeLegacyApprovalRuleTargets())
}

func (s *Server) removeApprovalRules() {
	targets := append(s.knownApprovalTargets(), s.activeApprovalRuleTargets()...)
	targets = append(targets, s.activeLegacyApprovalRuleTargets()...)
	s.removeApprovalTargets(targets)
}

func (s *Server) removeInactiveApprovalRules(candidates []approvalRuleTarget) {
	candidates = append(candidates, s.knownApprovalTargets()...)
	if len(candidates) == 0 {
		return
	}
	active := s.activeApprovalRuleTargets()
	activeKeys := make(map[string]struct{}, len(active))
	for _, target := range active {
		activeKeys[approvalTargetKey(target.Path)] = struct{}{}
	}
	var removable []approvalRuleTarget
	for _, target := range mergeApprovalRuleTargets(candidates) {
		if _, ok := activeKeys[approvalTargetKey(target.Path)]; ok {
			continue
		}
		removable = append(removable, target)
	}
	s.removeApprovalTargets(removable)
}

func (s *Server) handleApprovalStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.cfgMu.Lock()
	enabled := s.cfg.Approval.Enabled
	firstLaunchShown := s.cfg.Approval.FirstLaunchShown
	s.cfgMu.Unlock()
	writeJSON(w, map[string]bool{
		"enabled":            enabled,
		"first_launch_shown": firstLaunchShown,
	})
}

func (s *Server) handleApprovalEnable(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.cfgMu.Lock()
	s.cfg.Approval.Enabled = true
	s.cfg.Approval.FirstLaunchShown = true
	s.cfgMu.Unlock()
	s.injectApprovalRules()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	writeJSON(w, map[string]bool{"ok": true})
}

func (s *Server) handleApprovalDisable(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.cfgMu.Lock()
	s.cfg.Approval.Enabled = false
	s.cfgMu.Unlock()
	s.removeApprovalRules()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	writeJSON(w, map[string]bool{"ok": true})
}

// handleApprovalDismiss は初回トーストを「後で」で閉じた際に first_launch_shown をマークする
func (s *Server) handleApprovalDismiss(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.cfgMu.Lock()
	s.cfg.Approval.FirstLaunchShown = true
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	w.WriteHeader(http.StatusNoContent)
}
