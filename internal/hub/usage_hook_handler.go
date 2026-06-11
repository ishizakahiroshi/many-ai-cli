package hub

import (
	"fmt"
	"os"

	"any-ai-cli/internal/wrapper"
)

// usageHookSessionSnap は usage フック注入の参照カウント用スナップショット。
type usageHookSessionSnap struct {
	provider  string
	cwd       string
	sessionID int
}

// ---------------------------------------------------------------------------
// Server-level helpers
// ---------------------------------------------------------------------------

// tokenStatusbarEnabled は UserPrefs.TokenStatusbar.IsEnabled() を cfgMu で保護して返す。
func (s *Server) tokenStatusbarEnabled() bool {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.UserPrefs.TokenStatusbar.IsEnabled()
}

// hubBaseURL は Hub の base URL ("http://127.0.0.1:<port>") を返す。
func (s *Server) hubBaseURL() string {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return fmt.Sprintf("http://127.0.0.1:%d", s.cfg.Hub.Port)
}

// hubToken は Hub のアクセストークンを返す。
func (s *Server) hubToken() string {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Token
}

// resolveExePath は any-ai-cli バイナリのフルパスを返す。
// 取得に失敗した場合は "any-ai-cli" を返す（PATH 解決任せ）。
func resolveExePath() string {
	exe, err := os.Executable()
	if err != nil {
		return "any-ai-cli"
	}
	return exe
}

// ---------------------------------------------------------------------------
// inject / remove オーケストレーション
// ---------------------------------------------------------------------------

// activeUsageHookSnaps は active セッション（completed/error/disconnected 以外）の
// スナップショットを返す。
func (s *Server) activeUsageHookSnaps() []usageHookSessionSnap {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	snaps := make([]usageHookSessionSnap, 0, len(s.sessions))
	for id, ses := range s.sessions {
		if ses == nil {
			continue
		}
		if ses.State == "completed" || ses.State == "error" || ses.State == "disconnected" {
			continue
		}
		snaps = append(snaps, usageHookSessionSnap{
			provider:  ses.Provider,
			cwd:       ses.CWD,
			sessionID: id,
		})
	}
	return snaps
}

// injectUsageHooks は active セッション全体に対して usage フックを注入する。
// tokenStatusbar.enabled=false の場合は何もしない。
func (s *Server) injectUsageHooks() {
	if !s.tokenStatusbarEnabled() {
		return
	}
	snaps := s.activeUsageHookSnaps()
	if len(snaps) == 0 {
		return
	}
	hubURL := s.hubBaseURL()
	token := s.hubToken()
	exe := resolveExePath()

	for _, snap := range snaps {
		p := wrapper.UsageHookParams{
			HubURL:    hubURL,
			Token:     token,
			SessionID: snap.sessionID,
			ExePath:   exe,
		}
		switch snap.provider {
		case "claude":
			// claude は Hub 側で settings.local.json を書き換えない。
			// wrapper が起動時に `claude --settings <temp>` で statusLine を渡す
			// （共有ファイル衝突を避ける根本対策。usage_hooks.go 参照）。
		case "codex":
			if err := wrapper.InjectCodexStopHook(p); err != nil {
				s.logger.Warn("inject codex stop hook failed", "session_id", snap.sessionID, "err", err)
			} else {
				s.logger.Debug("inject codex stop hook ok", "session_id", snap.sessionID)
			}
		// copilot / cursor-agent は注入しない（トークン源なし）
		}
	}
}

// removeUsageHooks は active セッションが存在しなくなった provider の usage フックを除去する。
// approval_handler.go の removeInactiveApprovalRules と同じ思想。
func (s *Server) removeInactiveUsageHooks(endedProvider, endedCWD string) {
	// claude は共有ファイルを書き換えず wrapper 所有の temp settings を使うため、
	// Hub 側で除去する対象は無い（temp は wrapper 終了時に自身で削除する）。
	if endedProvider != "codex" {
		return
	}

	// codex は ~/.codex/config.toml への global 注入。active な codex が残って
	// いれば除去しない。
	for _, snap := range s.activeUsageHookSnaps() {
		if snap.provider == "codex" {
			return
		}
	}
	if err := wrapper.RemoveCodexStopHook(); err != nil {
		s.logger.Warn("remove codex stop hook failed", "err", err)
	} else {
		s.logger.Debug("remove codex stop hook ok")
	}
}

// removeAllUsageHooks は全 usage フックを除去する（Hub シャットダウン時用）。
func (s *Server) removeAllUsageHooks() {
	// Claude: active セッション全 cwd + remembered cwd を除去対象にする。
	// 本実装では active セッションのみを確認する（Hub シャットダウン時は全セッションが
	// active でなくなってから呼ばれるため、snaps は空になる。代わりに全 known CWD
	// をスキャンして除去する方法もあるが、ここでは active セッションを一括処理する
	// シンプルな実装にする。シャットダウン前に wrapperLoop が session_end を送るため
	// removeInactiveUsageHooks が個別に除去する）。

	// Codex は global なので、active セッションが 0 になった時点で除去済みのはず。
	// 念のためここでも試みる。
	if err := wrapper.RemoveCodexStopHook(); err != nil {
		s.logger.Warn("removeAllUsageHooks: remove codex failed", "err", err)
	}
}
