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
//
// セキュリティ: launcher は常に同一ホスト（直 loopback）から POST する設計のため、
// 論理リモート（tailscale serve 等）からの POST は受け付けない（環境バッジ spoofing
// 防止）。host_label は制御文字を除去し長さを制限、env_kind は許可値のみ採用する。
func (s *Server) handleNetHint(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.isLogicallyRemote(r) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "net-hint is only accepted from local loopback callers")
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
	// HostLabel: 制御文字を除去し最大長 128 rune に切る（UI バッジへ流すため）。
	cleanLabel := sanitizeHostLabel(body.HostLabel)
	// EnvKind: 既知値のみ採用。未知値は空文字に正規化（resolveEnvMeta 側でデフォルト動作）。
	cleanKind := normalizeEnvKind(body.EnvKind)
	s.netHintMu.Lock()
	s.netHintSSH = body.SSH
	s.netHintHost = cleanLabel
	s.netHintEnvKind = cleanKind
	s.netHintMu.Unlock()
	writeJSON(w, map[string]any{"ok": true})
}

// sanitizeHostLabel は host_label を UI 表示に安全な形に丸める
// （制御文字・改行除去、長さ 128 rune 上限）。
func sanitizeHostLabel(s string) string {
	if s == "" {
		return ""
	}
	const maxRunes = 128
	out := make([]rune, 0, len(s))
	for _, r := range s {
		if r < 0x20 || r == 0x7f {
			continue
		}
		out = append(out, r)
		if len(out) >= maxRunes {
			break
		}
	}
	return strings.TrimSpace(string(out))
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
// 直 loopback（直接ブラウザでローカル Hub に接続）は任意パスを許可する（cwd 入力 UX）。
// 論理リモート（tailscale serve / SSH tunnel 経由）は、ファイルシステム enumeration の
// 攻撃面を狭めるため、許可ルート（Hub cwd / 既存セッション cwd / project_favorites /
// cwd_history / cwd_favorites / HOME）の祖先または子孫のパスに限定する。
// 隠しフォルダ（先頭ドット）は除外、上限 500 件で打ち切る。シンボリックリンクは
// follow しない（リモートでの symlink-hop による enumeration 拡張を防ぐ）。
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
	if s.isLogicallyRemote(r) && !s.listSubdirsAllowedRemote(clean) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "path is outside allowed roots for remote callers")
		return
	}
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
			// シンボリックリンクは follow しない（リモートからの symlink-hop による
			// allowed roots 外への enumeration 経路を断つ）。直 loopback の cwd 入力
			// 補完では subdir 補完が落ちるが、UX 劣化は許容範囲。
			continue
		}
		subdirs = append(subdirs, name)
		if len(subdirs) >= maxSubdirs {
			break
		}
	}
	writeJSON(w, map[string]any{"ok": true, "path": clean, "subdirs": subdirs})
}

// listSubdirsAllowedRemote はリモート caller の指定パスが許可ルートの祖先 or 子孫かを返す。
// 許可ルート集合: Hub cwd / 既存セッション cwd / UserPrefs.{CwdHistory,CwdFavorites,ProjectFavorites} / HOME。
// 子孫許可は cwd 入力時の subdir 補完、祖先許可は「上のディレクトリへ移動」UX のため。
func (s *Server) listSubdirsAllowedRemote(p string) bool {
	if p == "" {
		return false
	}
	roots := make(map[string]struct{}, 16)
	addRoot := func(v string) {
		v = strings.TrimSpace(v)
		if v == "" {
			return
		}
		if !filepath.IsAbs(v) {
			return
		}
		roots[filepath.Clean(v)] = struct{}{}
	}
	addRoot(s.hubCWD)
	if home, err := os.UserHomeDir(); err == nil {
		addRoot(home)
	}
	s.sessionsMu.Lock()
	for _, ses := range s.sessions {
		if ses != nil {
			addRoot(ses.CWD)
		}
	}
	s.sessionsMu.Unlock()
	cfg := s.snapshotCfg()
	for _, v := range cfg.UserPrefs.ProjectFavorites {
		addRoot(v)
	}
	for _, v := range cfg.UserPrefs.CwdHistory {
		addRoot(v)
	}
	for _, v := range cfg.UserPrefs.CwdFavorites {
		addRoot(v)
	}
	for root := range roots {
		if pathsEqualForListSubdirs(p, root) {
			return true
		}
		if isAncestorOrEqual(root, p) {
			// p は root の子孫
			return true
		}
		if isAncestorOrEqual(p, root) {
			// p は root の祖先（「上に行く」UX）
			return true
		}
	}
	return false
}

// pathsEqualForListSubdirs は Windows のみ大小文字を無視して比較する。
func pathsEqualForListSubdirs(a, b string) bool {
	a = filepath.Clean(a)
	b = filepath.Clean(b)
	if a == b {
		return true
	}
	if runtime.GOOS == "windows" {
		return strings.EqualFold(a, b)
	}
	return false
}

// isAncestorOrEqual は child が parent と等しい、または parent 配下にあるかを返す。
func isAncestorOrEqual(parent, child string) bool {
	parent = filepath.Clean(parent)
	child = filepath.Clean(child)
	if pathsEqualForListSubdirs(parent, child) {
		return true
	}
	rel, err := filepath.Rel(parent, child)
	if err != nil {
		return false
	}
	rel = filepath.ToSlash(rel)
	if rel == "." {
		return true
	}
	if strings.HasPrefix(rel, "../") || rel == ".." {
		return false
	}
	return true
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
