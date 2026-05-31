package hub

import (
	"context"
	"time"

	"any-ai-cli/internal/config"
	"any-ai-cli/internal/proto"
)

// approvalPatternsRemoteSync は Hub 起動直後に呼ばれ、各 provider の公式 md を
// リモート fetch して <provider>.official.json を更新する。差分があれば
// approval_patterns_updated イベントを全 UI に broadcast する。
//
// 失敗時（タイムアウト・5xx・パースエラー）は official.json を触らず、既存ファイル
// （初回ならハードコード defaultApprovalPatterns で初期化済み）が使われ続ける。
//
// 仕様: 24h TTL（次回 Hub 再起動まで再取得しない）。手動更新 API はとりあえず提供しない。
func (s *Server) approvalPatternsRemoteSync(ctx context.Context) {
	s.cfgMu.Lock()
	sources := config.EffectiveApprovalPatternSources(s.cfg.ApprovalPatternSources)
	profiles := s.cfg.ApprovalProfiles
	s.cfgMu.Unlock()

	type fetchResult struct {
		provider string
		patterns []string
		err      error
	}

	results := make(chan fetchResult, len(KnownApprovalProviders()))
	for _, provider := range KnownApprovalProviders() {
		p := provider
		s.safeGo("approval_patterns_fetch_"+p, func() {
			url := approvalSourceFor(sources, p)
			if url == "" {
				results <- fetchResult{provider: p, err: nil, patterns: nil}
				return
			}
			pats, err := fetchAndParseApprovalPatterns(url)
			results <- fetchResult{provider: p, patterns: pats, err: err}
		})
	}

	timeout := time.NewTimer(30 * time.Second)
	defer timeout.Stop()

	var changed []string
	pending := len(KnownApprovalProviders())
	for pending > 0 {
		select {
		case <-ctx.Done():
			return
		case <-timeout.C:
			s.logger.Warn("approval patterns remote sync timeout", "pending", pending)
			pending = 0
		case r := <-results:
			pending--
			if r.err != nil {
				s.logger.Warn("approval patterns fetch failed", "provider", r.provider, "err", r.err)
				continue
			}
			if len(r.patterns) == 0 {
				// 空 md は破損とみなし、上書きしない
				continue
			}
			current, err := ReadApprovalPatternsByProfile(r.provider, config.ApprovalProfileOfficial)
			if err != nil {
				s.logger.Warn("approval patterns read failed", "provider", r.provider, "err", err)
				continue
			}
			if stringSliceEqual(current, r.patterns) {
				continue
			}
			if err := WriteOfficialApprovalPatterns(r.provider, r.patterns); err != nil {
				s.logger.Warn("approval patterns write failed", "provider", r.provider, "err", err)
				continue
			}
			changed = append(changed, r.provider)
		}
	}

	if len(changed) == 0 {
		return
	}
	// 公式が更新され、かつそれをアクティブプロファイルとして使っている provider が
	// あれば <provider>.json ミラーを更新する。
	mirrorNeeded := false
	effective := config.EffectiveApprovalProfiles(profiles)
	for _, p := range changed {
		if effective.For(p) == config.ApprovalProfileOfficial {
			mirrorNeeded = true
			break
		}
	}
	if mirrorNeeded {
		if err := RefreshActiveMirrors(profiles); err != nil {
			s.logger.Warn("refresh active mirrors failed", "err", err)
		}
	}
	s.broadcast(proto.Message{Type: "approval_patterns_updated", Providers: changed})
}

func approvalSourceFor(src config.ApprovalPatternSources, provider string) string {
	switch provider {
	case "claude":
		return src.Claude
	case "codex":
		return src.Codex
	case "copilot":
		return src.Copilot
	case "cursor-agent":
		return src.CursorAgent
	case "common":
		return src.Common
	}
	return ""
}

func stringSliceEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
