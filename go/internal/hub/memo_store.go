package hub

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"time"

	"many-ai-cli/internal/config"
)

// memo は 1 件の作業メモ。plan_memo-panel.md の案 C（プロジェクトに紐づくメモ）。
// project はメモを書いた時点の選択中セッションの cwd を Hub 側で git root に
// 正規化した値で、クライアントから直接受け取らない（projectForSession 経由のみ）。
type memo struct {
	ID        string `json:"id"`
	Text      string `json:"text"`
	Project   string `json:"project"`
	Done      bool   `json:"done"`
	CreatedAt string `json:"created_at"`
	UpdatedAt string `json:"updated_at"`
	DoneAt    string `json:"done_at,omitempty"`
	// Images は memo-images/ 配下の画像名（パスではない）。memo_images.go 参照。
	Images []string `json:"images,omitempty"`
}

type memoFile struct {
	Version int    `json:"version"`
	Memos   []memo `json:"memos"`
}

const (
	memoTextMaxLen = 4000
	memoMaxCount   = 500
)

type memoManager struct {
	mu        sync.Mutex
	data      memoFile
	path      string
	imagesDir string
	loadErr   error
	write     func(string, memoFile) error
}

func newMemoManager(path string) *memoManager {
	m := &memoManager{
		path:      path,
		imagesDir: filepath.Join(filepath.Dir(path), "memo-images"),
		write:     writeMemoFile,
		data:      memoFile{Version: 1, Memos: []memo{}},
	}
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return m
	}
	if err != nil {
		m.loadErr = err
		return m
	}
	if err = json.Unmarshal(b, &m.data); err != nil {
		m.loadErr = err
		return m
	}
	if m.data.Version != 1 {
		m.loadErr = errors.New("unsupported memo store version")
	}
	return m
}

func (s *Server) initMemos() {
	dir, err := config.Dir()
	if err != nil {
		s.memos = &memoManager{loadErr: err}
		return
	}
	s.memos = newMemoManager(filepath.Join(dir, "memos.json"))
	if s.memos.loadErr != nil {
		s.logger.Warn("memo store unavailable", "err", s.memos.loadErr)
		return
	}
	if n := s.memos.cleanOrphanImages(time.Now()); n > 0 {
		s.logger.Info("removed unattached memo images", "count", n)
	}
}

// commitLocked publishes memory only after the complete new file has been saved.
func (m *memoManager) commitLocked(next memoFile) error {
	if m.loadErr != nil {
		return m.loadErr
	}
	if err := m.write(m.path, next); err != nil {
		return err
	}
	m.data = next
	return nil
}

func (m *memoManager) copyLocked() memoFile {
	return memoFile{Version: 1, Memos: append([]memo{}, m.data.Memos...)}
}

func writeMemoFile(path string, data memoFile) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	b, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), "memos-*.tmp")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err = f.Write(b); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Sync(); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	if err = os.Chmod(f.Name(), 0o600); err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}

func memoID() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(b[:]), nil
}

func memoTime(t time.Time) string { return t.UTC().Format(time.RFC3339Nano) }

// projectForSession はメモを書いた時点の選択中セッション（sessionID）の cwd を
// git root に正規化して project として返す。sessionID が 0（未指定）、または
// そのセッションが見つからないときは空文字（「未分類」）を返す。
// project はここを経由してのみ決まり、POST /api/memos の body から直接は受け取らない。
func (s *Server) projectForSession(sessionID int) string {
	if sessionID == 0 {
		return ""
	}
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	var cwd string
	if ses != nil {
		cwd = ses.CWD
	}
	s.sessionsMu.Unlock()
	if cwd == "" {
		return ""
	}
	return findGitRoot(cwd)
}

// memoMention はメモの本文とその project だけを持つ最小のビュー。C2（ビューアの
// 読み取り許可を広げる）が、論理リモートからの files-content 等の要求で「保存済み
// メモの本文に言及されたパス」を読み取り専用で許可する判定に使う。id / done 等の
// フィールドはその判定に不要なので持たない。
type memoMention struct {
	Text    string
	Project string
}

// memoMentions は保存済みメモ全件を memoMention のスライスで返す。
// s.memos が未初期化（loadErr 等）のときは nil を返す（呼び出し側は「言及なし」と
// 同じ扱いになる。エラーを伝播させない＝C2 の読み取り許可判定は fail-closed）。
func (s *Server) memoMentions() []memoMention {
	if s.memos == nil {
		return nil
	}
	s.memos.mu.Lock()
	defer s.memos.mu.Unlock()
	out := make([]memoMention, 0, len(s.memos.data.Memos))
	for _, item := range s.memos.data.Memos {
		out = append(out, memoMention{Text: item.Text, Project: item.Project})
	}
	return out
}
