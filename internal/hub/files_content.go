package hub

import (
	"encoding/json"
	"net/http"
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

// handleFilesContent は GET /api/files-content を処理する。
// ?path=<absPath>&token=<token> 必須。
// ?session=<id> で検証スコープを指定（省略時: Hub cwd）。
func (s *Server) handleFilesContent(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	pathParam := r.URL.Query().Get("path")
	if pathParam == "" {
		http.Error(w, "path parameter is required", http.StatusBadRequest)
		return
	}
	if !filepath.IsAbs(pathParam) {
		http.Error(w, "path must be an absolute path", http.StatusBadRequest)
		return
	}

	if !isTextFile(pathParam) {
		http.Error(w, "forbidden: not a previewable text file", http.StatusForbidden)
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.hubCWD
	if sidStr := r.URL.Query().Get("session"); sidStr != "" {
		if sid, err := strconv.Atoi(sidStr); err == nil {
			s.mu.Lock()
			if ses := s.sessions[sid]; ses != nil {
				cwd = ses.CWD
			}
			s.mu.Unlock()
		}
	}

	// 許可ルートの確定
	gitRoot := findGitRoot(cwd)
	allowed, err := isPathUnderAllowedRoots(pathParam, cwd, gitRoot)
	if err != nil || !allowed {
		http.Error(w, "forbidden: path is outside allowed roots", http.StatusForbidden)
		return
	}

	// ファイル情報取得
	info, err := os.Stat(pathParam)
	if err != nil {
		if os.IsNotExist(err) {
			http.Error(w, "not found", http.StatusNotFound)
			return
		}
		http.Error(w, "internal server error", http.StatusInternalServerError)
		return
	}
	if info.IsDir() {
		http.Error(w, "path is a directory", http.StatusBadRequest)
		return
	}

	// ファイル読み込み（上限 1 MiB）
	f, err := os.Open(pathParam)
	if err != nil {
		http.Error(w, "cannot open file", http.StatusInternalServerError)
		return
	}
	defer f.Close()

	buf := make([]byte, filesContentMaxSize+1)
	n, _ := f.Read(buf)
	truncated := false
	if n > filesContentMaxSize {
		n = filesContentMaxSize
		truncated = true
	}

	resp := filesContentResp{
		Path:      pathParam,
		Size:      info.Size(),
		Mtime:     info.ModTime(),
		Content:   string(buf[:n]),
		Truncated: truncated,
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}
