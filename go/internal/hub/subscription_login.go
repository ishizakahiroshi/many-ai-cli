package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/sessionlog"
)

// subscriptionLoginDirName is the dedicated workspace for vendor-CLI login
// sessions. Login must not run in Hub's process cwd: that is often the
// directory that holds the running exe, and the session list groups by the
// last path component, which then shows up as a leftover project (observed
// as repo-root `dist/` when Hub was started from the build output).
const subscriptionLoginDirName = "subscription-login"

func ensureSubscriptionLoginRoot(configDir string) (string, error) {
	configDir = strings.TrimSpace(configDir)
	if configDir == "" {
		return "", fmt.Errorf("config directory is empty")
	}
	root := filepath.Join(filepath.Clean(configDir), subscriptionLoginDirName)
	if err := os.MkdirAll(root, sessionlog.PrivateDirMode); err != nil {
		return "", fmt.Errorf("create login workspace: %w", err)
	}
	_ = os.Chmod(root, sessionlog.PrivateDirMode)
	return root, nil
}

// dismissSubscriptionLoginSession removes a finished login session from the
// live map. Ordinary sessions are left alone: their disconnected cards stay
// until the user closes them (or the 24h auto-dismiss). Login sessions are
// advertised as short-lived and must not remain as a project group.
func (s *Server) dismissSubscriptionLoginSession(sessionID int) {
	if sessionID <= 0 {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	login := ses != nil && ses.SubscriptionLogin
	s.sessionsMu.Unlock()
	if !login {
		return
	}
	_ = s.handleDismiss(protoMessageSessionDismiss(sessionID))
}
