package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// requireLoopbackRemote は open 系のホスト操作（既定アプリ/フォルダ/ターミナル起動）を
// loopback 元のみに制限する。設計上これらは localhost 専用のホスト操作 API であり、
// reverse-proxy / Docker 等で非 loopback peer から到達する経路では 403 にする
// （handleOpenDir と同じ防御を共通化）。tailscale serve / SSH tunnel 経由は元が
// loopback になるためここは素通しだが、guard() の PIN ゲート（isLogicallyRemote）で
// 別途保護される。
func (s *Server) requireLoopbackRemote(w http.ResponseWriter, r *http.Request) bool {
	if !isLoopbackRemote(r.RemoteAddr) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "loopback only")
		return false
	}
	return true
}

// openDeniedExtensions は「既定のアプリで開く」で実行に化ける拡張子のブラックリスト。
// ShellExecute("open")（Windows）や cmd.exe /c start はこれらを実行するため、文書/メディアを
// 開く正規 UX を保ちつつ、認証済みユーザーからのホスト上 RCE（.bat/.ps1 を書いてから開く等）を
// 防ぐ。tailscale serve / SSH tunnel 経由（RemoteAddr が loopback 化する）でも効く多層防御。
var openDeniedExtensions = map[string]bool{
	".bat": true, ".cmd": true, ".com": true, ".exe": true, ".scr": true,
	".pif": true, ".hta": true, ".cpl": true, ".msc": true, ".msi": true,
	".reg": true, ".ps1": true, ".psm1": true, ".vbs": true, ".vbe": true,
	".js": true, ".jse": true, ".wsf": true, ".wsh": true, ".lnk": true,
	".scf": true, ".url": true,
}

// checkOpenPathAllowed は open 系ハンドラで path が allowed-roots 配下かを検証する。
// ?session=<id> が指定されていればそのセッションの CWD を判定基準にする（省略時: Hub cwd）。
// 許可外のパスは 403 を返す。
func (s *Server) checkOpenPathAllowed(w http.ResponseWriter, r *http.Request, path string) bool {
	if !filepath.IsAbs(path) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path must be absolute")
		return false
	}
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)
	// 添付ディレクトリ（~/.many-ai-cli/attachments）も許可ルートに含める。
	// many-ai-cli 自身が保存した画像等を「既定のアプリで開く」「フォルダを開く」
	// 対象にできるようにするため（CWD/git root の外にあるため従来は 403 になっていた）。
	attachDir, _ := attachmentsDir()
	allowed, err := isPathUnderAllowedRoots(path, cwd, gitRoot, attachDir)
	if err != nil || !allowed {
		writeJSONError(w, http.StatusForbidden, "forbidden", "path is outside allowed roots")
		return false
	}
	return true
}

func (s *Server) decodeAllowedPath(w http.ResponseWriter, r *http.Request) (string, bool) {
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return "", false
	}
	if body.Path == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path is required")
		return "", false
	}
	if !s.checkOpenPathAllowed(w, r, body.Path) {
		return "", false
	}
	return body.Path, true
}

func (s *Server) handleOpenDefaultFile(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if !s.requireLoopbackRemote(w, r) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	if openDeniedExtensions[strings.ToLower(filepath.Ext(path))] {
		writeJSONError(w, http.StatusForbidden, "forbidden", "executable file type not allowed")
		return
	}
	if err := openFileNative(path); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenFolder(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if !s.requireLoopbackRemote(w, r) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	// path がディレクトリ（許可ルート自身＝Files タブの「フォルダを開く」など）なら
	// それ自体を開く。ファイルのときだけ親フォルダを reveal する。旧実装は常に
	// filepath.Dir(path) を開くため、path が許可ルート自身だと検証していない親
	// （許可境界の 1 階層外）を開いてしまっていた。
	dir := filepath.Dir(path)
	if fi, err := os.Stat(path); err == nil && fi.IsDir() {
		dir = path
	}
	if err := openDirNative(dir); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenTerminal(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if !s.requireLoopbackRemote(w, r) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	s.cfgMu.Lock()
	app := s.cfg.TerminalApp
	s.cfgMu.Unlock()
	if err := openTerminalNative(path, app); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleTerminalApp(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		app := s.cfg.TerminalApp
		s.cfgMu.Unlock()
		writeJSON(w, map[string]string{
			"terminal_app":           app,
			"effective_terminal_app": effectiveTerminalAppDescription(app),
		})
	case http.MethodPost:
		// ターミナル exe の設定変更は loopback 元のみ（任意 executable / UNC パスを
		// リモートから仕込んで handleOpenTerminal で起動させる経路を塞ぐ）。GET（表示）は
		// remote でも可。
		if !s.requireLoopbackRemote(w, r) {
			return
		}
		var body struct {
			TerminalApp string `json:"terminal_app"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		app := strings.TrimSpace(body.TerminalApp)
		s.cfgMu.Lock()
		s.cfg.TerminalApp = app
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]any{
			"ok":                     true,
			"terminal_app":           app,
			"effective_terminal_app": effectiveTerminalAppDescription(app),
		})
	}
}
