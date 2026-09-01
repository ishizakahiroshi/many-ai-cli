package hub

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

// approval_patterns_custom.go wires custom_providers: 's approval_pattern_source
// into the same asset path the browser already loads live pattern data from
// (plan_custom-provider-extension-triage.md C5). Earlier investigation had
// concluded ApprovalPatternSources-fetched files reach nothing but the
// Settings UI; that was wrong (see approval_patterns.go's doc history) — the
// browser's providerApprovalTriggers already fetches /approval-patterns/<id>.json
// for the 7 built-in names, so a custom provider only needed the same file to
// exist under its own id.
//
// Unlike the built-in official/custom two-profile system, a custom provider
// has no "official" patterns many-ai-cli could know about, so there is only
// one tier here: whatever approval_pattern_source resolves to becomes the
// active file directly. An entry with no source configured is skipped
// entirely — doctor's customProviderApprovalPatternNotice already tells the
// user that case is a no-op.

func customApprovalPatternAssetPath(id string) string {
	return filepath.Join(approvalPatternsDir(), id+".json")
}

func readCustomApprovalPatternsMirror(id string) []string {
	data, err := os.ReadFile(customApprovalPatternAssetPath(id))
	if err != nil {
		return nil
	}
	var list []string
	if err := json.Unmarshal(data, &list); err != nil {
		return nil
	}
	return list
}

// syncCustomApprovalPatterns fetches (or reads, for a local-file source) each
// configured approval_pattern_source and writes it to the same
// ~/.many-ai-cli/approval-patterns/ directory the built-in mirrors live in,
// under the custom provider's own id. Called once at Hub startup, mirroring
// approvalPatternsRemoteSync's timing and failure handling (best-effort: a
// fetch failure just leaves any existing file in place, or leaves the asset
// 404ing if none was ever written).
func (s *Server) syncCustomApprovalPatterns(ctx context.Context) {
	s.cfgMu.Lock()
	customProviders := config.EffectiveCustomProviders(s.cfg.CustomProviders)
	s.cfgMu.Unlock()

	var targets []config.CustomProvider
	for _, p := range customProviders {
		if strings.TrimSpace(p.ApprovalPatternSource) != "" {
			targets = append(targets, p)
		}
	}
	if len(targets) == 0 {
		return
	}

	type fetchResult struct {
		id       string
		patterns []string
		err      error
	}
	results := make(chan fetchResult, len(targets))
	for _, p := range targets {
		p := p
		s.safeGo("custom_approval_patterns_fetch_"+p.ID, func() {
			pats, err := fetchAndParseApprovalPatterns(p.ApprovalPatternSource)
			results <- fetchResult{id: p.ID, patterns: pats, err: err}
		})
	}

	timeout := time.NewTimer(30 * time.Second)
	defer timeout.Stop()

	var changed []string
	pending := len(targets)
	for pending > 0 {
		select {
		case <-ctx.Done():
			return
		case <-timeout.C:
			s.logger.Warn("custom approval patterns sync timeout", "pending", pending)
			pending = 0
		case r := <-results:
			pending--
			if r.err != nil {
				s.logger.Warn("custom approval patterns fetch failed", "id", r.id, "err", r.err)
				continue
			}
			if len(r.patterns) == 0 {
				// 空ファイル・パース結果ゼロは破損とみなし、既存ファイルを上書きしない
				// （built-in の approvalPatternsRemoteSync と同じ扱い）。
				continue
			}
			if stringSliceEqual(readCustomApprovalPatternsMirror(r.id), r.patterns) {
				continue
			}
			if err := writePatternFile(customApprovalPatternAssetPath(r.id), r.patterns); err != nil {
				s.logger.Warn("custom approval patterns write failed", "id", r.id, "err", err)
				continue
			}
			changed = append(changed, r.id)
		}
	}

	if len(changed) == 0 {
		return
	}
	s.broadcast(proto.Message{Type: "approval_patterns_updated", Providers: changed})
}

// validCustomApprovalPatternAssetName is validApprovalPatternAssetName's
// counterpart for custom provider ids: same shape checks (no path
// separators, no leading dot, .json suffix), but membership is checked
// against config.EffectiveCustomProviders instead of the fixed built-in list.
func (s *Server) validCustomApprovalPatternAssetName(name string) bool {
	name = strings.TrimSpace(name)
	if name == "" || strings.HasPrefix(name, ".") || strings.ContainsAny(name, `/\`) {
		return false
	}
	if !strings.HasSuffix(name, ".json") {
		return false
	}
	base := strings.TrimSuffix(name, ".json")
	s.cfgMu.Lock()
	ok := s.cfg.IsCustomProviderID(base)
	s.cfgMu.Unlock()
	return ok
}
