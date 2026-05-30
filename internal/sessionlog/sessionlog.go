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

// secretPatterns は PTY 出力から既知の秘密文字列を伏字化するパターン一覧。
// ヒューリスティックなので過剰マスクは許容し、取りこぼしは残る前提。
var secretPatterns = []*regexp.Regexp{
	// 汎用 API キー変数への代入 / 出力: ANTHROPIC_API_KEY=sk-... など
	regexp.MustCompile(`(?i)([A-Z_]*API_KEY=)\S+`),
	// Anthropic / OpenAI トークン: sk-ant-... / sk-...
	regexp.MustCompile(`sk-(?:ant-)?[A-Za-z0-9_\-]{20,}`),
	// GitHub Personal Access Token: ghp_ / github_pat_
	regexp.MustCompile(`(?:ghp_|github_pat_)[A-Za-z0-9_]{20,}`),
	// Bearer トークン
	regexp.MustCompile(`(?i)(Bearer )\S{8,}`),
	// AWS アクセスキー
	regexp.MustCompile(`(?:AKIA|ASIA|AROA)[A-Z0-9]{16}`),
}

// MaskSecrets は s 中の既知の秘密パターンを "***" に置換して返す。
// ANSI エスケープを含む生 PTY 文字列にそのまま適用してよい（可視文字列範囲に限定されるため誤爆は最小限）。
func MaskSecrets(s string) string {
	for _, re := range secretPatterns {
		// Bearer など prefix を持つパターンはキャプチャグループ 1 を残す
		if re.NumSubexp() >= 1 {
			s = re.ReplaceAllStringFunc(s, func(m string) string {
				sub := re.FindStringSubmatchIndex(m)
				if len(sub) >= 4 && sub[2] >= 0 {
					prefix := m[:sub[3]-sub[2]] // group[1] の文字数分
					return prefix + "***"
				}
				return "***"
			})
		} else {
			s = re.ReplaceAllString(s, "***")
		}
	}
	return s
}

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
	mu        sync.Mutex
	f         *os.File
	written   int64
	maxBytes  int64
	truncated bool // true = log_truncated マーカーを書き済み
}

const (
	// PrivateDirMode / PrivateFileMode are used for PTY/session logs because
	// raw terminal output can contain credentials or local project details.
	PrivateDirMode  os.FileMode = 0o700
	PrivateFileMode os.FileMode = 0o600
)

// NewJSONLWriter は新規ファイルを作成して Writer を返す。
// maxBytes > 0 の場合、累積書き込みバイト数がその値に達すると以降の書き込みを no-op にする。
func NewJSONLWriter(path string, maxBytes int64) (*Writer, error) {
	if err := os.MkdirAll(filepath.Dir(path), PrivateDirMode); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, PrivateFileMode)
	if err != nil {
		return nil, err
	}
	return &Writer{f: f, maxBytes: maxBytes}, nil
}

// NewJSONLWriterAppend は既存ファイルに追記する Writer を返す。
// maxBytes > 0 の場合、累積書き込みバイト数がその値に達すると以降の書き込みを no-op にする。
// 追記時は既存ファイルサイズをカウンタの初期値とする。
func NewJSONLWriterAppend(path string, maxBytes int64) (*Writer, error) {
	if err := os.MkdirAll(filepath.Dir(path), PrivateDirMode); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, PrivateFileMode)
	if err != nil {
		return nil, err
	}
	var written int64
	if info, err := f.Stat(); err == nil {
		written = info.Size()
	}
	return &Writer{f: f, written: written, maxBytes: maxBytes}, nil
}

// writeLine は JSON marshal 済みのイベントを改行付きでファイルに書く。
// 書き込み失敗時はファイルハンドルを閉じ、以降の書き込みを無効化する（フェイルセーフ）。
// mu は呼び出し側で保持していること。
func (w *Writer) writeLine(line []byte) error {
	if _, err := w.f.Write(line); err != nil {
		// 書き込み失敗 → ハンドルを閉じて以降を停止
		_ = w.f.Close()
		w.f = nil
		return err
	}
	w.written += int64(len(line))
	return nil
}

// Event は event を JSONL 形式でファイルに書く。
// サイズ上限到達時は log_truncated マーカーを一度だけ書いてから no-op になる。
// ただし type:"session_end" のイベントは上限後も例外的に書き込む。
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
	line := append(b, '\n')

	if w.maxBytes > 0 && w.written >= w.maxBytes {
		// 上限到達済み: truncated マーカーを一度だけ書く
		if !w.truncated {
			w.truncated = true
			marker, _ := json.Marshal(map[string]any{
				"type": "log_truncated",
				"at":   time.Now().UTC().Format(time.RFC3339),
			})
			markerLine := append(marker, '\n')
			_ = w.writeLine(markerLine) // エラーは無視（ベストエフォート）
		}
		// session_end は上限後も例外書き込み
		if isSessionEndEvent(event) {
			return w.writeLine(line)
		}
		return nil
	}

	return w.writeLine(line)
}

// isSessionEndEvent は event が session_end タイプか判定する。
func isSessionEndEvent(event any) bool {
	type typer interface{ getType() string }
	// map[string]any の場合
	if m, ok := event.(map[string]any); ok {
		return m["type"] == "session_end"
	}
	// json.Marshal して確認（重い処理だが session_end は低頻度）
	b, err := json.Marshal(event)
	if err != nil {
		return false
	}
	var m map[string]any
	if err := json.Unmarshal(b, &m); err != nil {
		return false
	}
	return m["type"] == "session_end"
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
