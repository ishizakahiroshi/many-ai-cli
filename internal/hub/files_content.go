package hub

import (
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const filesContentMaxSize = 1 * 1024 * 1024 // 1 MiB

// filesContentResp は /api/files-content のレスポンス。
type filesContentResp struct {
	Path      string    `json:"path"`
	Size      int64     `json:"size"`
	Mtime     time.Time `json:"mtime"`
	Content   string    `json:"content"`
	Truncated bool      `json:"truncated"`
	// ReadOnly は許可ルート外だがチャットの言及により読み取り専用で許可した場合に true。
	ReadOnly bool `json:"readOnly"`
}

// previewableTextExtensions は /api/files-content で許可するテキスト系拡張子（小文字）。
var previewableTextExtensions = map[string]bool{
	".txt": true, ".md": true, ".markdown": true, ".rst": true, ".log": true,
	".json": true, ".jsonl": true, ".yaml": true, ".yml": true, ".toml": true, ".ini": true, ".cfg": true, ".conf": true, ".env": true,
	".csv": true, ".tsv": true, ".xml": true, ".html": true, ".htm": true,
	".css": true, ".scss": true, ".sass": true, ".less": true,
	".js": true, ".mjs": true, ".cjs": true, ".jsx": true, ".ts": true, ".tsx": true, ".vue": true,
	".go": true, ".rs": true, ".py": true, ".rb": true, ".php": true, ".java": true, ".kt": true, ".kts": true,
	".c": true, ".cc": true, ".cpp": true, ".cxx": true, ".h": true, ".hh": true, ".hpp": true, ".cs": true,
	".sh": true, ".bash": true, ".zsh": true, ".fish": true, ".ps1": true, ".psm1": true, ".bat": true, ".cmd": true,
	".sql": true, ".graphql": true, ".gql": true, ".proto": true, ".diff": true, ".patch": true,
	".gitignore": true, ".gitattributes": true, ".editorconfig": true,
}

var previewableTextBasenames = map[string]bool{
	"dockerfile": true,
	"makefile":   true,
	"readme":     true,
	"license":    true,
	"changelog":  true,
	"notice":     true,
}

func isTextFile(absPath string) bool {
	ext := strings.ToLower(filepath.Ext(absPath))
	if previewableTextExtensions[ext] {
		return true
	}
	base := strings.ToLower(filepath.Base(absPath))
	return previewableTextBasenames[base]
}

var previewableImageExtensions = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true,
	".webp": true, ".bmp": true,
}

var previewableVideoExtensions = map[string]bool{
	".mp4": true, ".webm": true, ".ogv": true, ".mov": true, ".m4v": true,
}

func isMediaFile(absPath string) bool {
	ext := strings.ToLower(filepath.Ext(absPath))
	return previewableImageExtensions[ext] || previewableVideoExtensions[ext]
}

// cwdForRequest は ?session=<id> パラメータが指定されている場合、そのセッションの CWD を返す。
// 指定がない場合は Hub 起動時の hubCWD を返す。
// files_list / files_content / files_move / files_rename / files_delete / files_roots の
// 各ハンドラで同一のロジックが重複していたため、ここに集約する。
func (s *Server) cwdForRequest(r *http.Request) string {
	if sidStr := r.URL.Query().Get("session"); sidStr != "" {
		if sid, err := strconv.Atoi(sidStr); err == nil {
			s.sessionsMu.Lock()
			if ses := s.sessions[sid]; ses != nil {
				cwd := ses.CWD
				s.sessionsMu.Unlock()
				return cwd
			}
			s.sessionsMu.Unlock()
		}
	}
	return s.hubCWD
}

// fileReadGrant は resolveAllowedFilePath の判定結果。
type fileReadGrant struct {
	// path は検証済みの絶対パス。
	path string
	// readOnly は許可ルート外のファイルを読み取り専用で許可した場合に true。
	// 書き込み系 API は引き続き cwd / git root 配下しか受け付けないため、
	// UI 側はこのフラグで編集ボタンを抑止する。
	readOnly bool
	// viaMention はチャット言及フォールバック由来（＝論理リモートからの許可）の場合に true。
	// この経路だけダウンロードの type ゲートを維持する（監査 #25）。
	viaMention bool
}

// resolveAllowedFilePath は ?path= を検証して読み取り許可の判定結果を返す。
//
// 判定は 3 段:
//  1. cwd / git root / attachments / orchestration 配下なら無条件に許可（readOnly=false）
//  2. 許可ルート外は秘密情報 denylist（isSecretReadDenied）で拒否
//  3. 直 loopback は許可ルート制限なしで読み取り専用許可、
//     論理リモートはユーザーがチャットで言及したパスのみ読み取り専用許可
//
// files-content / files-download / files-asset の GET 専用。
// 書き込み系 API では使わないこと（isPathUnderAllowedRoots を直接呼ぶ）。
func (s *Server) resolveAllowedFilePath(r *http.Request) (fileReadGrant, error) {
	pathParam := r.URL.Query().Get("path")
	if pathParam == "" {
		return fileReadGrant{}, httpError{status: http.StatusBadRequest, msg: "path parameter is required"}
	}
	if !filepath.IsAbs(pathParam) {
		return fileReadGrant{}, httpError{status: http.StatusBadRequest, msg: "path must be an absolute path"}
	}

	cwd := s.cwdForRequest(r)

	gitRoot := findGitRoot(cwd)
	// 添付ディレクトリ（~/.many-ai-cli/attachments）も許可ルートに含める。
	// 送信履歴から many-ai-cli 自身が保存した添付（画像・CSV・テキスト等）を
	// プレビュー / ダウンロードできるようにするため（CWD/git root の外にあるため
	// 従来は 403 になっていた）。open 系ハンドラと同じ扱い（read-only の GET 専用）。
	attachDir, _ := attachmentsDir()
	// orchestration board（~/.many-ai-cli/orchestration/<id>/board.md）も同様に許可する。
	// conductor カードの「board を開く」導線は cwd/git root の外を指すため。
	orchDir, _ := orchestrationDir()
	allowed, err := isPathUnderAllowedRoots(pathParam, cwd, gitRoot, attachDir, orchDir)
	if err == nil && allowed {
		return fileReadGrant{path: pathParam}, nil
	}

	// ここから先は許可ルート外＝「中身をブラウザへ返してよいか」の判断になるため
	// 秘密情報 denylist を適用する。
	if isSecretReadDenied(pathParam) {
		return fileReadGrant{}, httpError{status: http.StatusForbidden, msg: "forbidden: path is denied as a secret-like file"}
	}

	// 直 loopback（Hub ホストのブラウザ）は許可ルートで制限しない。根拠は filesScopeRestricted。
	if !s.filesScopeRestricted(r) {
		return fileReadGrant{path: pathParam, readOnly: true}, nil
	}

	// 論理リモート（tailscale / trusted_networks / スマホ）は従来どおり、
	// ユーザー自身がチャットで言及したパスのみ読み取り専用で許可する。
	if s.isPathMentionedInSession(r, pathParam, cwd) {
		return fileReadGrant{path: pathParam, readOnly: true, viaMention: true}, nil
	}
	return fileReadGrant{}, httpError{status: http.StatusForbidden, msg: "forbidden: path is outside allowed roots"}
}

// isPathMentionedInSession は ?session= のチャット（sessionstore）に
// absPath の言及があるかをサーバ側で照合する。クライアント申告は信用しない。
func (s *Server) isPathMentionedInSession(r *http.Request, absPath, cwd string) bool {
	if s.sessionStore == nil {
		return false
	}
	sid, err := strconv.Atoi(r.URL.Query().Get("session"))
	if err != nil || sid <= 0 {
		return false
	}
	ok, err := s.sessionStore.MessagesMentionText(sid, pathMentionVariants(absPath, cwd))
	return err == nil && ok
}

// pathMentionVariants は履歴照合に使うパス表記の変種を返す。
// ターミナル出力では \ と / が混在し、cwd からの相対表記で言及されることもあるため、
// 絶対パス（両セパレータ）+ 相対パス（両セパレータ）を照合対象にする。
func pathMentionVariants(absPath, cwd string) []string {
	addBothSeps := func(out []string, p string) []string {
		if p == "" {
			return out
		}
		out = append(out, p)
		if fwd := strings.ReplaceAll(p, "\\", "/"); fwd != p {
			out = append(out, fwd)
		} else if back := strings.ReplaceAll(p, "/", "\\"); back != p {
			out = append(out, back)
		}
		return out
	}
	variants := addBothSeps(nil, absPath)
	if cwd != "" {
		if rel, err := filepath.Rel(cwd, absPath); err == nil && rel != "" && rel != "." {
			variants = addBothSeps(variants, rel)
		}
	}
	return variants
}

type httpError struct {
	status int
	msg    string
}

func (e httpError) Error() string { return e.msg }

// handleFilesContent は GET /api/files-content を処理する。
// ?path=<absPath>&token=<token> 必須。
// ?session=<id> で検証スコープを指定（省略時: Hub cwd）。
func (s *Server) handleFilesContent(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	grant, err := s.resolveAllowedFilePath(r)
	if err != nil {
		if he, ok := err.(httpError); ok {
			writeJSONError(w, he.status, httpErrorCode(he.status), he.msg)
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	pathParam, readOnly := grant.path, grant.readOnly

	if !isTextFile(pathParam) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "not a previewable text file")
		return
	}

	// ファイル情報取得
	info, err := os.Stat(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		if os.IsNotExist(err) {
			writeJSONError(w, http.StatusNotFound, "not_found", "not found")
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	if info.IsDir() {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path is a directory")
		return
	}

	// ファイル読み込み（上限 1 MiB）
	f, err := os.Open(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", "cannot open file")
		return
	}
	defer f.Close()

	buf, err := io.ReadAll(io.LimitReader(f, filesContentMaxSize+1))
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_failed", "cannot read file")
		return
	}
	truncated := len(buf) > filesContentMaxSize
	if truncated {
		buf = buf[:filesContentMaxSize]
	}

	resp := filesContentResp{
		Path:      pathParam,
		Size:      info.Size(),
		Mtime:     info.ModTime(),
		Content:   string(buf),
		Truncated: truncated,
		ReadOnly:  readOnly,
	}

	writeJSON(w, resp)
}

func contentDispositionAttachment(filename string) string {
	fallback := asciiFilenameFallback(filename)
	return fmt.Sprintf(`attachment; filename="%s"; filename*=UTF-8''%s`, fallback, url.PathEscape(filename))
}

func asciiFilenameFallback(filename string) string {
	var b strings.Builder
	for _, r := range filename {
		switch {
		case r >= 0x20 && r <= 0x7e && r != '"' && r != '\\' && r != '/' && r != ';':
			b.WriteRune(r)
		case r == '\t':
			b.WriteByte('_')
		case r > 0x7e:
			b.WriteByte('_')
		default:
			b.WriteByte('_')
		}
	}
	fallback := strings.TrimSpace(b.String())
	if fallback == "" || fallback == "." || fallback == ".." {
		return "download"
	}
	return fallback
}

// handleFilesDownload は GET /api/files-download を処理する。
// ?path=<absPath>&token=<token> 必須。
// ?session=<id> で検証スコープを指定（省略時: Hub cwd）。
func (s *Server) handleFilesDownload(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	grant, err := s.resolveAllowedFilePath(r)
	if err != nil {
		if he, ok := err.(httpError); ok {
			writeJSONError(w, he.status, httpErrorCode(he.status), he.msg)
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	pathParam := grant.path

	// viaMention==true はチャット言及フォールバック由来（＝論理リモートからの許可）。
	// この経路では content/asset と同じく type ゲートを課し、テキスト/メディア以外の
	// 任意バイナリ（資格情報・鍵ファイル等）が言及されただけで全文配信されるのを防ぐ（監査 #25）。
	// 許可ルート内と、直 loopback の許可ルート外読み取り（秘密情報 denylist で保護済み）は
	// 従来どおり拡張子無制限でダウンロードできる。
	if grant.viaMention && !isTextFile(pathParam) && !isMediaFile(pathParam) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "not a downloadable file outside allowed roots")
		return
	}

	info, err := os.Stat(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		if os.IsNotExist(err) {
			writeJSONError(w, http.StatusNotFound, "not_found", "not found")
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	if info.IsDir() {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path is a directory")
		return
	}

	f, err := os.Open(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", "cannot open file")
		return
	}
	defer f.Close()

	filename := filepath.Base(pathParam)
	contentType := mime.TypeByExtension(strings.ToLower(filepath.Ext(pathParam)))
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	w.Header().Set("Content-Disposition", contentDispositionAttachment(filename))
	w.Header().Set("Content-Type", contentType)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	http.ServeContent(w, r, filename, info.ModTime(), f)
}

// handleFilesAsset は GET /api/files-asset を処理する。
// Files タブ内のメディアプレビュー用。許可ルート内の画像/動画ファイルだけを配信する。
func (s *Server) handleFilesAsset(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	grant, err := s.resolveAllowedFilePath(r)
	if err != nil {
		if he, ok := err.(httpError); ok {
			writeJSONError(w, he.status, httpErrorCode(he.status), he.msg)
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	pathParam := grant.path
	if !isMediaFile(pathParam) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "not a previewable media file")
		return
	}

	info, err := os.Stat(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		if os.IsNotExist(err) {
			writeJSONError(w, http.StatusNotFound, "not_found", "not found")
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "internal_error", "internal server error")
		return
	}
	if info.IsDir() {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path is a directory")
		return
	}

	f, err := os.Open(pathParam) // #nosec G703 -- resolveAllowedFilePath で許可ルート配下を検証済み
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", "cannot open file")
		return
	}
	defer f.Close()

	if ct := mime.TypeByExtension(strings.ToLower(filepath.Ext(pathParam))); ct != "" {
		w.Header().Set("Content-Type", ct)
	}
	w.Header().Set("X-Content-Type-Options", "nosniff")
	http.ServeContent(w, r, filepath.Base(pathParam), info.ModTime(), f)
}
