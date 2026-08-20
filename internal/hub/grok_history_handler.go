package hub

import (
	"bufio"
	"encoding/json"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/sessionlog"
)

const (
	// grokHistoryDefaultLimit は /api/grok-history の limit 省略時のメッセージ件数。
	grokHistoryDefaultLimit = 50
	// grokHistoryMaxLimit は 1 リクエストで返すメッセージ件数の上限。
	grokHistoryMaxLimit = 200
	// grokHistoryLineMax は chat_history.jsonl の 1 行上限。encrypted reasoning を
	// 含む行は数百 KB になり得るため bufio.Scanner の既定 64KB では足りない。
	grokHistoryLineMax = 8 * 1024 * 1024
	// grokHistoryMatchWindow は「セッション開始時刻 と Grok セッション作成時刻の
	// 近さ」でディレクトリを対応付ける際の許容差。wrapper 起動 → grok CLI が
	// セッションディレクトリを作るまでは通常数秒。
	grokHistoryMatchWindow = 10 * time.Minute
)

// grokChatMessage は /api/grok-history が UI に返す 1 メッセージ。
type grokChatMessage struct {
	Role string `json:"role"` // "user" | "assistant"
	Text string `json:"text"`
}

// handleGrokHistory は Grok Build CLI 自身が保存している会話履歴
// （$GROK_HOME/sessions/<cwd>/<session_id>/chat_history.jsonl。未設定時は ~/.grok）を整形して返す。
// Grok は alt screen 上で全画面上書き描画するため xterm のスクロールバックが
// 育たず、生 PTY ログの再生（/api/session-log + 過去ログビューア）では
// 絶対座標フレームの機械置換で読めない出力になる。many-ai-cli 側では
// 何も保存せず、Grok が元々持つ構造化ログを読み取り専用で参照する。
//
//	GET /api/grok-history?session_id=<id>&offset=<msg>&limit=<msg>
//
// offset 省略時または負値は「末尾から limit 件」。レスポンスは JSON:
//
//	{ok, total, offset, messages: [{role, text}]}
func (s *Server) handleGrokHistory(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	q := r.URL.Query()
	sessionID, err := strconv.Atoi(q.Get("session_id"))
	if err != nil || sessionID <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}

	s.sessionsMu.Lock()
	var provider, cwd, startedAt, homeDir, grokHome string
	if ses := s.sessions[sessionID]; ses != nil {
		provider = ses.Provider
		cwd = ses.CWD
		startedAt = ses.StartedAt
		homeDir = ses.HomeDir
		grokHome = ses.GrokHome
	}
	s.sessionsMu.Unlock()
	if provider == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "session not found")
		return
	}
	if provider != "grok" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "not a grok session")
		return
	}
	grokDir := grokHomeDir(grokHome, homeDir)
	if grokDir == "" {
		// UserHomeDir が失敗すると homeDir は空のままとなり、後段の
		// filepath.Join("", ".grok") が相対パス ".grok" となる。プロセス
		// cwd 配下の別プロジェクトの .grok を意図せず読む恐れがあるため、
		// ホームディレクトリを解決できない環境では 404 で早期終了する。
		h, err := os.UserHomeDir()
		if err != nil || strings.TrimSpace(h) == "" {
			writeJSONError(w, http.StatusNotFound, "not_found", "user home directory unavailable")
			return
		}
		grokDir = grokHomeDir("", h)
	}
	startTime, err := time.Parse(time.RFC3339, startedAt)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "session start time unavailable")
		return
	}

	historyPath, ok := findGrokChatHistory(grokDir, cwd, startTime)
	if !ok {
		writeJSONError(w, http.StatusNotFound, "not_found", "grok chat history not found")
		return
	}

	messages, err := readGrokChatHistory(historyPath)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "not_found", errorDetail("grok chat history not readable", err))
		return
	}

	limit := grokHistoryDefaultLimit
	if v := q.Get("limit"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil || n <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid limit")
			return
		}
		limit = min(n, grokHistoryMaxLimit)
	}
	offset := -1
	if v := q.Get("offset"); v != "" {
		n, err := strconv.Atoi(v)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid offset")
			return
		}
		offset = n
	}
	total := len(messages)
	if offset < 0 {
		offset = max(total-limit, 0)
	}
	if offset > total {
		offset = total
	}
	end := min(offset+limit, total)

	writeJSON(w, map[string]any{
		"ok":       true,
		"total":    total,
		"offset":   offset,
		"messages": messages[offset:end],
	})
}

// grokActiveSession は ~/.grok/active_sessions.json の 1 エントリ。
type grokActiveSession struct {
	SessionID string `json:"session_id"`
	CWD       string `json:"cwd"`
	OpenedAt  string `json:"opened_at"`
}

// grokHomeDir は Grok Build CLI のデータディレクトリを返す。
// grokHome（$GROK_HOME）が空でなければそれが ~/.grok を置き換える。
// Codex の CodexHome / $CODEX_HOME と同じ契約。設定済みの grokHome から
// 既定の ~/.grok へは落とさない（別プロファイルの履歴を誤って拾わないため）。
func grokHomeDir(grokHome, userHome string) string {
	if g := strings.TrimSpace(grokHome); g != "" {
		return g
	}
	if h := strings.TrimSpace(userHome); h != "" {
		return filepath.Join(h, ".grok")
	}
	return ""
}

// findGrokChatHistory は many-ai-cli セッション（cwd + 開始時刻）に対応する
// Grok セッションディレクトリを特定し、chat_history.jsonl のパスを返す。
//
// 対応付けは 2 段:
//  1. active_sessions.json（実行中の Grok が自己申告する session_id/cwd/opened_at）
//     から cwd 一致 かつ opened_at が開始時刻に最も近いものを取る。
//     ここに載る PID は grok プロセス自身のもので wrapper の PID とは別物のため、
//     PID 突合ではなく時刻近傍で対応付ける。
//  2. 終了済みで 1. に無い場合、セッションディレクトリ名（UUIDv7・先頭 48bit が
//     生成時刻 ms）を復号して開始時刻に最も近いものを取る。
//
// どちらも grokHistoryMatchWindow を超える差は不一致として扱う（同一 cwd で
// many-ai-cli 外の Grok セッションを誤って拾わないため）。
func findGrokChatHistory(grokDir, cwd string, startedAt time.Time) (string, bool) {
	cwdDir, ok := findGrokCwdDir(filepath.Join(grokDir, "sessions"), cwd)
	if !ok {
		return "", false
	}

	type candidate struct {
		path  string
		delta time.Duration
	}
	pick := func(sessionID string, delta time.Duration) (candidate, bool) {
		p := filepath.Join(cwdDir, sessionID, "chat_history.jsonl")
		if info, err := os.Stat(p); err == nil && !info.IsDir() {
			return candidate{path: p, delta: delta}, true
		}
		return candidate{}, false
	}
	var candidates []candidate
	seen := map[string]int{}
	addCandidate := func(c candidate) {
		if idx, exists := seen[c.path]; exists {
			if c.delta < candidates[idx].delta {
				candidates[idx].delta = c.delta
			}
			return
		}
		seen[c.path] = len(candidates)
		candidates = append(candidates, c)
	}

	// 1. active_sessions.json（実行中セッション）
	var fallbackActive *candidate
	if data, err := os.ReadFile(filepath.Join(grokDir, "active_sessions.json")); err == nil {
		var actives []grokActiveSession
		if json.Unmarshal(data, &actives) == nil {
			for _, a := range actives {
				if !grokPathsEquivalent(a.CWD, cwd) {
					continue
				}
				opened, err := time.Parse(time.RFC3339Nano, a.OpenedAt)
				if err != nil {
					continue
				}
				delta := opened.Sub(startedAt).Abs()
				if delta > grokHistoryMatchWindow {
					continue
				}
				if c, ok := pick(a.SessionID, delta); ok {
					addCandidate(c)
					if fallbackActive == nil || c.delta <= fallbackActive.delta {
						cp := c
						fallbackActive = &cp
					}
				}
			}
		}
	}

	// 2. ディレクトリ名（UUIDv7）の埋め込み時刻で近傍一致
	entries, err := os.ReadDir(cwdDir)
	var fallbackUUID *candidate
	if err == nil {
		for _, e := range entries {
			if !e.IsDir() {
				continue
			}
			ts, ok := uuidV7Time(e.Name())
			if !ok {
				continue
			}
			delta := ts.Sub(startedAt).Abs()
			if delta > grokHistoryMatchWindow {
				continue
			}
			if c, ok := pick(e.Name(), delta); ok {
				addCandidate(c)
				if fallbackUUID == nil || c.delta <= fallbackUUID.delta {
					cp := c
					fallbackUUID = &cp
				}
			}
		}
	}

	// 空 stub より、表示可能な user/assistant メッセージを含む候補を優先する。
	// 非空候補同士ではメッセージ数を比較せず、従来どおり開始時刻の近さだけで選ぶ。
	var bestNonEmpty *candidate
	for _, c := range candidates {
		messages, err := readGrokChatHistory(c.path)
		if err != nil || len(messages) == 0 {
			continue
		}
		if bestNonEmpty == nil || c.delta < bestNonEmpty.delta {
			cp := c
			bestNonEmpty = &cp
		}
	}
	if bestNonEmpty != nil {
		return bestNonEmpty.path, true
	}

	// 全候補が空なら従来の優先順位へ戻す:
	// active_sessions の時刻近傍候補を先に取り、無ければ UUIDv7 候補を使う。
	if fallbackActive != nil {
		return fallbackActive.path, true
	}
	if fallbackUUID != nil {
		return fallbackUUID.path, true
	}
	return "", false
}

// findGrokCwdDir は ~/.grok/sessions/ 直下から cwd に対応するディレクトリを探す。
// ディレクトリ名は cwd をパーセントエンコードしたもの（例:
// C%3A%5Cdev%5C...）だが、エンコード方式の細部に依存しないよう
// 「各エントリ名をデコードして cwd と突き合わせる」方向で照合する。
func findGrokCwdDir(sessionsRoot, cwd string) (string, bool) {
	entries, err := os.ReadDir(sessionsRoot)
	if err != nil {
		return "", false
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		decoded, err := url.PathUnescape(e.Name())
		if err != nil {
			continue
		}
		if grokPathsEquivalent(decoded, cwd) {
			return filepath.Join(sessionsRoot, e.Name()), true
		}
	}
	return "", false
}

// grokPathsEquivalent はパス表記ゆれ（区切り文字・大文字小文字）を吸収して比較する。
// Windows のドライブレターやフォルダ名は大文字小文字を区別しないため EqualFold で照合する。
func grokPathsEquivalent(a, b string) bool {
	norm := func(p string) string {
		return filepath.Clean(strings.ReplaceAll(p, "/", string(filepath.Separator)))
	}
	return strings.EqualFold(norm(a), norm(b))
}

// uuidV7Time は UUIDv7 文字列の先頭 48bit（Unix ミリ秒）を時刻として取り出す。
func uuidV7Time(id string) (time.Time, bool) {
	hex := strings.ReplaceAll(id, "-", "")
	if len(hex) != 32 {
		return time.Time{}, false
	}
	// version ニブル（13 文字目）が 7 でなければ UUIDv7 ではない
	if hex[12] != '7' {
		return time.Time{}, false
	}
	ms, err := strconv.ParseUint(hex[:12], 16, 64)
	if err != nil {
		return time.Time{}, false
	}
	return time.UnixMilli(int64(ms)), true
}

// grokChatLine は chat_history.jsonl の 1 行。content は文字列と
// パーツ配列（{type:"text",text:...} / {type:"image",...}）の両形がある。
type grokChatLine struct {
	Type            string          `json:"type"`
	SyntheticReason string          `json:"synthetic_reason"`
	Content         json.RawMessage `json:"content"`
}

type grokContentPart struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// readGrokChatHistory は chat_history.jsonl から表示対象メッセージを抽出する。
//   - assistant: 本文をそのまま採用
//   - user: synthetic_reason 無し かつ <user_query> タグを含む行のみ、タグ内を採用
//     （<user_info> / <image_files> 等の環境情報ブロックは表示しない）
//   - reasoning / tool_result / system 等は返さない（encrypted_content を UI へ流さない）
//
// 返却前に既知の秘密文字列を伏字化する（/api/session-log と同じ規約）。
func readGrokChatHistory(path string) ([]grokChatMessage, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	var messages []grokChatMessage
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), grokHistoryLineMax)
	for sc.Scan() {
		line := sc.Bytes()
		if len(line) == 0 {
			continue
		}
		var ev grokChatLine
		if json.Unmarshal(line, &ev) != nil {
			continue
		}
		text := grokContentText(ev.Content)
		switch ev.Type {
		case "assistant":
			if t := strings.TrimSpace(text); t != "" {
				messages = append(messages, grokChatMessage{Role: "assistant", Text: sessionlog.MaskSecrets(t)})
			}
		case "user":
			if ev.SyntheticReason != "" {
				continue
			}
			query, ok := extractGrokUserQuery(text)
			if !ok {
				continue
			}
			messages = append(messages, grokChatMessage{Role: "user", Text: sessionlog.MaskSecrets(query)})
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	return messages, nil
}

// grokContentText は content フィールド（文字列 or パーツ配列）からテキストを取り出す。
func grokContentText(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	var parts []grokContentPart
	if json.Unmarshal(raw, &parts) != nil {
		return ""
	}
	var b strings.Builder
	for _, p := range parts {
		if p.Type == "text" && p.Text != "" {
			if b.Len() > 0 {
				b.WriteString("\n")
			}
			b.WriteString(p.Text)
		}
	}
	return b.String()
}

// extractGrokUserQuery は <user_query>...</user_query> の中身を取り出す。
// タグが無い user 行は環境情報ブロック（<user_info> 等）なので表示対象外。
func extractGrokUserQuery(text string) (string, bool) {
	const openTag = "<user_query>"
	const closeTag = "</user_query>"
	start := strings.Index(text, openTag)
	if start < 0 {
		return "", false
	}
	rest := text[start+len(openTag):]
	end := strings.Index(rest, closeTag)
	if end < 0 {
		end = len(rest)
	}
	q := strings.TrimSpace(rest[:end])
	if q == "" {
		return "", false
	}
	return q, true
}
