package hub

import (
	"net/http"
	"os"
	"path/filepath"
)

// filesRootsCandidate は /api/files-roots の候補エントリ。
type filesRootsCandidate struct {
	Name    string `json:"name"`
	AbsPath string `json:"absPath"`
	Exists  bool   `json:"exists"`
}

// filesRootsResp は /api/files-roots のレスポンス。
type filesRootsResp struct {
	GitRoot    string                `json:"gitRoot"`
	Candidates []filesRootsCandidate `json:"candidates"`
}

// handleFilesRoots は GET /api/files-roots を処理する。
// ?session=<id>&token=<token> 必須（session は省略可: Hub cwd を使用）。
func (s *Server) handleFilesRoots(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)

	// git ルートを検出（ピュア Go 実装: .git ディレクトリを親方向に探索）
	gitRoot := findGitRoot(cwd)

	// git ルート直下の docs/ を候補として追加
	candidates := buildFilesCandidates(gitRoot)

	resp := filesRootsResp{
		GitRoot:    gitRoot,
		Candidates: candidates,
	}

	writeJSON(w, resp)
}

// buildFilesCandidates は gitRoot 直下の標準的なファイルディレクトリを候補として返す。
// 候補: docs/（存在チェック付き）
func buildFilesCandidates(gitRoot string) []filesRootsCandidate {
	candidates := []filesRootsCandidate{}

	docsPath := filepath.Join(gitRoot, "docs")
	info, err := os.Stat(docsPath)
	exists := err == nil && info.IsDir()
	candidates = append(candidates, filesRootsCandidate{
		Name:    "docs",
		AbsPath: docsPath,
		Exists:  exists,
	})

	return candidates
}
