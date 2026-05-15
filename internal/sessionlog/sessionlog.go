package sessionlog

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
	"unicode"
)

type Metadata struct {
	SessionID int
	Provider  string
	CWD       string
	StartedAt time.Time
}

var (
	invalidFileChars = regexp.MustCompile(`[<>:"/\\|?*]`)
	spaceRun         = regexp.MustCompile(`\s+`)
	oscRE            = regexp.MustCompile(`\x1b\][^\x07]*(?:\x07|\x1b\\)`)
	ansiRE           = regexp.MustCompile(`\x1b\[[0-?]*[ -/]*[@-~]`)
	ansiSimpleRE     = regexp.MustCompile(`\x1b[@-_]`)
)

func BaseName(meta Metadata) string {
	provider := SanitizeFilePart(strings.ToLower(strings.TrimSpace(meta.Provider)))
	if provider == "" {
		provider = "unknown"
	}
	t := meta.StartedAt
	if t.IsZero() {
		t = time.Now()
	}
	ts := t.Format("2006-01-02_150405")
	folder := SanitizeFilePart(filepath.Base(meta.CWD))
	return fmt.Sprintf("%s_%s_%s_s%d", provider, ts, folder, meta.SessionID)
}

func Paths(logDir string, meta Metadata) (rawLogPath string, jsonlPath string) {
	base := BaseName(meta)
	dir := filepath.Join(logDir, "sessions")
	return filepath.Join(dir, base+".log"), filepath.Join(dir, base+".jsonl")
}

// TranscriptPath は jsonlPath（...jsonl）に対応するクリーンテキスト（...txt）のパスを返す。
// 拡張子が .jsonl でない場合は jsonlPath にそのまま .txt を付け足す（呼び出し側の責任）。
func TranscriptPath(jsonlPath string) string {
	if strings.HasSuffix(jsonlPath, ".jsonl") {
		return strings.TrimSuffix(jsonlPath, ".jsonl") + ".txt"
	}
	return jsonlPath + ".txt"
}

func SanitizeFilePart(s string) string {
	s = strings.TrimSpace(s)
	s = invalidFileChars.ReplaceAllString(s, "_")
	s = strings.Map(func(r rune) rune {
		if r < 0x20 || r == 0x7f || unicode.IsControl(r) {
			return '_'
		}
		return r
	}, s)
	s = spaceRun.ReplaceAllString(s, "_")
	s = strings.Trim(s, ". ")
	if s == "" {
		return "no-project"
	}
	if len(s) > 80 {
		s = s[:80]
	}
	return s
}

func StripANSI(s string) string {
	s = oscRE.ReplaceAllString(s, "")
	s = ansiRE.ReplaceAllString(s, "")
	return ansiSimpleRE.ReplaceAllString(s, "")
}

func EncodeBase64(data []byte) string {
	return base64.StdEncoding.EncodeToString(data)
}

type Writer struct {
	mu sync.Mutex
	f  *os.File
}

func NewJSONLWriter(path string) (*Writer, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	f, err := os.Create(path)
	if err != nil {
		return nil, err
	}
	return &Writer{f: f}, nil
}

func NewJSONLWriterAppend(path string) (*Writer, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return nil, err
	}
	return &Writer{f: f}, nil
}

func (w *Writer) Event(event any) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.f == nil {
		return os.ErrClosed
	}
	b, err := json.Marshal(event)
	if err != nil {
		return err
	}
	if _, err := w.f.Write(append(b, '\n')); err != nil {
		return err
	}
	return nil
}

func (w *Writer) Close() error {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.f == nil {
		return nil
	}
	err := w.f.Close()
	w.f = nil
	return err
}
