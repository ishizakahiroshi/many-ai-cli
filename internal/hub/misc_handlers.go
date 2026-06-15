package hub

import (
	"fmt"
	"io"
	"net"
	"net/http"
	neturl "net/url"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/attach"
)

const (
	attachUploadMaxBytes    = 10 * 1024 * 1024
	attachMultipartMaxBytes = attachUploadMaxBytes + 1*1024*1024
)

func (s *Server) handleInfo(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	mode := runtimeMode()
	cfg := s.snapshotCfg()

	userAvatar := cfg.UserPrefs.Avatar
	if userAvatar != "" && !strings.HasPrefix(userAvatar, "http://") && !strings.HasPrefix(userAvatar, "https://") {
		userAvatar = fmt.Sprintf("/api/avatar?token=%s", neturl.QueryEscape(cfg.Token))
	}
	userDisplayName := cfg.UserPrefs.DisplayName
	if userDisplayName == "" {
		if v := os.Getenv("USERNAME"); v != "" {
			userDisplayName = v
		} else {
			userDisplayName = os.Getenv("USER")
		}
	}

	sshSession, hostIP := hostNetInfo()
	// launcher（SSH tunnel モード）が /api/net-hint で登録した接続元情報があれば
	// 補正する。コンテナ内実行の Hub は NIC から内部 IP（例: 172.19.0.2）しか
	// 検出できず、SSH 経由起動の自己判定もできないため。
	s.netHintMu.Lock()
	if s.netHintSSH {
		sshSession = true
	}
	if s.netHintHost != "" {
		hostIP = s.netHintHost
	}
	netHintSSH := s.netHintSSH
	netHintEnvKind := s.netHintEnvKind
	s.netHintMu.Unlock()
	env := resolveEnvMeta(cfg.Hub.EnvKind, mode, sshSession, hostIP, netHintSSH, netHintEnvKind)
	writeJSON(w, map[string]any{
		"cwd":             s.hubCWD,
		"version":         s.version,
		"runtime_mode":    mode,
		"runtime_label":   runtimeLabel(mode),
		"ssh":             sshSession,
		"host_ip":         hostIP,
		"env_kind":        env.Kind,
		"env_label":       env.Label,
		"env_short":       env.Short,
		"env_color":       env.Color,
		"env_title":       env.Title,
		"env_host_label":  env.HostLabel,
		"userAvatar":      userAvatar,
		"userDisplayName": userDisplayName,
	})
}

// handleNetHint は launcher（SSH tunnel モード）から接続元情報を受け取り保持する。
// tunnel モードでは既起動の Hub に MANY_AI_CLI_HOST_LABEL を注入できないため、
// トンネル確立後に launcher が POST し、/api/info のバッジ表示情報を補正する。
func (s *Server) handleNetHint(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		SSH       bool   `json:"ssh"`
		HostLabel string `json:"host_label"`
		EnvKind   string `json:"env_kind"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	s.netHintMu.Lock()
	s.netHintSSH = body.SSH
	s.netHintHost = body.HostLabel
	s.netHintEnvKind = body.EnvKind
	s.netHintMu.Unlock()
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleAvatar(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.cfgMu.Lock()
	path := s.cfg.UserPrefs.Avatar
	s.cfgMu.Unlock()
	if path == "" || strings.HasPrefix(path, "http://") || strings.HasPrefix(path, "https://") {
		http.NotFound(w, r)
		return
	}
	// avatar はアップロード専用パス（~/.many-ai-cli/user_avatar.bin）の配信のみ許可する。
	// 任意ローカルパスを許すと config.yaml（Token / RemotePINHash / AuthCookieSecret 等を
	// 含む）など ~/.many-ai-cli 配下の機密ファイルを read できてしまうため、固定パスとの
	// 完全一致のみ通す（任意ファイル read プリミティブ化を防ぐ。AuthCookieSecret 漏洩は
	// PIN cookie 偽造による PIN 境界の恒久バイパスにつながる）。
	uploadPath, err := avatarUploadPath()
	if err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	if pathExistsCandidateKey(path) != pathExistsCandidateKey(uploadPath) {
		http.NotFound(w, r)
		return
	}
	data, err := os.ReadFile(uploadPath)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	ct := http.DetectContentType(data)
	w.Header().Set("Content-Type", ct)
	w.Header().Set("Cache-Control", "max-age=3600")
	_, _ = w.Write(data)
}

func (s *Server) handleAttach(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, attachMultipartMaxBytes)
	if err := r.ParseMultipartForm(attachUploadMaxBytes); err != nil { // #nosec G120 -- 直前の MaxBytesReader で読み取り上限を設定済み
		writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("bad request", err))
		return
	}
	if r.MultipartForm != nil {
		defer r.MultipartForm.RemoveAll()
	}
	sessionID, err := strconv.Atoi(r.FormValue("session_id"))
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}
	file, header, err := r.FormFile("file")
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "missing file")
		return
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, attachUploadMaxBytes+1))
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_failed", errorDetail("read error", err))
		return
	}
	if len(data) > attachUploadMaxBytes {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "file too large")
		return
	}
	s.sessionsMu.Lock()
	var provider string
	if ses := s.sessions[sessionID]; ses != nil {
		provider = ses.Provider
	}
	s.sessionsMu.Unlock()
	if provider == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "session not found")
		return
	}
	attachDir, err := attachmentsDir()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", "home dir error")
		return
	}
	savedPath, inject, err := attach.Save(attachDir, sessionID, provider, data, header.Filename)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save error", err))
		return
	}
	s.logger.Info("attach saved via HTTP", "session_id", sessionID, "path", savedPath)
	s.writeHistory(sessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "attach",
		"session_id": sessionID,
		"path":       savedPath,
		"filename":   header.Filename,
		"provider":   provider,
	})
	writeJSON(w, map[string]any{
		"ok":         true,
		"inject":     inject,
		"saved_path": savedPath,
		"filename":   header.Filename,
	})
}

func (s *Server) handlePickDirectory(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	path, err := pickDirectoryNative()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "pick_failed", errorDetail("pick error", err))
		return
	}
	if path == "" {
		writeJSON(w, map[string]any{"ok": false})
		return
	}
	writeJSON(w, map[string]any{"ok": true, "path": path})
}

func (s *Server) handlePickFile(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	filterExe := r.URL.Query().Get("filter") == "exe"
	path, err := pickFileNative(filterExe)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "pick_failed", errorDetail("pick error", err))
		return
	}
	if path == "" {
		writeJSON(w, map[string]any{"ok": false})
		return
	}
	writeJSON(w, map[string]any{"ok": true, "path": path})
}

// handlePathExists は UI の cwd 入力欄/履歴ドロップダウン向けに、複数パスが
// 「実在するディレクトリ」かをまとめて判定して返す。
// POST {"paths": ["C:\\dev\\foo", ...]} → {"results": {"C:\\dev\\foo": true, ...}}
//
// Spawn 時の Cmd.Dir に渡すと Windows では存在しないディレクトリで
// CreateProcess が ERROR_DIRECTORY を返して分かりにくいので、事前に弾く用途。
//
// 要求された各パスをそのまま stat する（履歴に無い、ユーザーが新規に打ち込んだ
// パスこそ検証が必要なため、許可リストでの絞り込みはしない）。spawn 自体が
// 任意 cwd を受け付けてエラーで存在有無を返すので、ここで実在判定を返しても
// 漏れる情報は同じ。バインドは 127.0.0.1 固定＋トークン認証で保護される。
func (s *Server) handlePathExists(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Paths []string `json:"paths"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	results := make(map[string]bool, len(body.Paths))
	for _, p := range body.Paths {
		if p == "" {
			continue
		}
		info, err := os.Stat(p)
		results[p] = err == nil && info.IsDir()
	}
	writeJSON(w, map[string]any{"results": results})
}

// handleListSubdirs は cwd 入力欄の補完用に、指定パス直下のサブディレクトリ名一覧を返す。
// POST {"path": "C:\\dev\\github\\public\\"} → {"ok": true, "path": "...", "subdirs": ["a", "b", ...]}
//
// バインドは 127.0.0.1 固定 + トークン認証で保護。隠しフォルダ（先頭ドット）は除外、
// 上限 500 件で打ち切る（巨大ディレクトリでのフロント側カクつき防止）。
func (s *Server) handleListSubdirs(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	p := strings.TrimSpace(body.Path)
	if p == "" {
		writeJSON(w, map[string]any{"ok": false, "subdirs": []string{}})
		return
	}
	if !filepath.IsAbs(p) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path must be absolute")
		return
	}
	clean := filepath.Clean(p)
	info, err := os.Stat(clean)
	if err != nil || !info.IsDir() {
		writeJSON(w, map[string]any{"ok": false, "path": clean, "subdirs": []string{}})
		return
	}
	entries, err := os.ReadDir(clean)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_failed", errorDetail("readdir failed", err))
		return
	}
	const maxSubdirs = 500
	subdirs := make([]string, 0, len(entries))
	for _, e := range entries {
		name := e.Name()
		if name == "" || name[0] == '.' {
			continue
		}
		if !e.IsDir() {
			// シンボリックリンク経由のディレクトリも拾う
			if e.Type()&os.ModeSymlink == 0 {
				continue
			}
			if fi, err := os.Stat(filepath.Join(clean, name)); err != nil || !fi.IsDir() {
				continue
			}
		}
		subdirs = append(subdirs, name)
		if len(subdirs) >= maxSubdirs {
			break
		}
	}
	writeJSON(w, map[string]any{"ok": true, "path": clean, "subdirs": subdirs})
}

func pathExistsCandidateKey(path string) string {
	key := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		key = strings.ToLower(key)
	}
	return key
}

// handleOpenDir opens a directory or reveals a file in the OS file manager.
//
// Security:
//   - token required
//   - request must come from a loopback address (defense-in-depth on top of the
//     127.0.0.1 bind that NewServer already enforces)
//   - kind "log"/"attach": only the configured log_dir or attach_dir is permitted;
//     arbitrary paths are rejected so an XSS in the UI cannot turn this into "open any folder"
//   - kind "path": arbitrary absolute paths are permitted; risk is accepted because
//     token auth + loopback-only binding limits exposure, and the operation is "reveal
//     in folder" (not "execute"), which has limited blast radius
func (s *Server) handleOpenDir(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		writeJSONError(w, http.StatusForbidden, "forbidden", "loopback remote address required")
		return
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		writeJSONError(w, http.StatusForbidden, "forbidden", "loopback only")
		return
	}
	var body struct {
		Kind string `json:"kind"` // "log", "attach", or "path"
		Path string `json:"path"` // kind=="path" のみ使用
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Kind == "path" {
		if !filepath.IsAbs(body.Path) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "path must be absolute")
			return
		}
		if !s.checkOpenPathAllowed(w, r, body.Path) {
			return
		}
		if err := openRevealNative(body.Path); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "open_failed", errorDetail("open failed", err))
			return
		}
		writeJSON(w, map[string]any{"ok": true, "path": body.Path})
		return
	}
	var target string
	switch body.Kind {
	case "log":
		s.cfgMu.Lock()
		target = s.cfg.Hub.LogDir
		s.cfgMu.Unlock()
	case "attach":
		dir, err := attachmentsDir()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "home_dir_error", "home dir unavailable")
			return
		}
		target = dir
	default:
		writeJSONError(w, http.StatusBadRequest, "bad_request", "unknown kind")
		return
	}
	if target == "" {
		writeJSONError(w, http.StatusInternalServerError, "not_configured", "target dir not configured")
		return
	}
	if err := os.MkdirAll(target, 0o700); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "mkdir_failed", errorDetail("mkdir failed", err))
		return
	}
	if err := os.Chmod(target, 0o700); err != nil { // #nosec G302 -- ディレクトリには実行ビットが必要（0700 は所有者限定で適切）
		writeJSONError(w, http.StatusInternalServerError, "chmod_failed", errorDetail("chmod failed", err))
		return
	}
	if err := openDirNative(target); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", errorDetail("open failed", err))
		return
	}
	writeJSON(w, map[string]any{"ok": true, "path": target})
}
