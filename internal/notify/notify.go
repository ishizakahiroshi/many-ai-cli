// Package notify implements outbound HTTP notification backends (ntfy / webhook).
// Each backend is fire-and-forget: errors are logged but never propagate to
// the caller.  All methods are safe to call concurrently.
package notify

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	neturl "net/url"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/sessionlog"
)

const (
	sendTimeout     = 10 * time.Second
	payloadMaxBytes = 200
	// sentTTL は重複送信防止エントリの保持時間。
	sentTTL = time.Hour
)

// BackendConfig は設定ファイルの backends[] 1 件に対応する。
type BackendConfig struct {
	Type  string `yaml:"type"  json:"type"` // "ntfy" | "webhook"
	URL   string `yaml:"url"   json:"url"`
	Topic string `yaml:"topic" json:"topic"` // ntfy のみ有効
}

// Config は config.yaml の notify: セクション全体に対応する。
type Config struct {
	Backends    []BackendConfig `yaml:"backends" json:"backends"`
	Events      []string        `yaml:"events"   json:"events"` // ["approval"]
	IncludeBody bool            `yaml:"include_body" json:"include_body"`
}

// GenerateRandomTopic は ntfy トピック名用の暗号学的乱数文字列を生成する。
func GenerateRandomTopic() (string, error) {
	buf := make([]byte, 12)
	if _, err := rand.Read(buf); err != nil {
		return "", fmt.Errorf("generate random topic: %w", err)
	}
	return "anyaicli-" + hex.EncodeToString(buf), nil
}

// ApprovalPayload は承認通知の本文。Hub URL / token は含めない。
type ApprovalPayload struct {
	ID         string // 重複送信防止用。空の場合は自動生成
	SessionID  int
	Provider   string
	Title      string
	Body       string
	Risk       string
	ApproveURL string
	RejectURL  string
	OpenURL    string
}

// DonePayload はタスク完了通知の本文。Hub URL / token は含めない。
type DonePayload struct {
	ID        string // 重複送信防止用。空の場合は自動生成
	SessionID int
	Provider  string
	Title     string
	Summary   string
	Kind      string // success | failure | aborted | needs_action
}

// Manager は ntfy/webhook 通知の送信管理を行う。
type Manager struct {
	mu     sync.Mutex
	cfg    Config
	client *http.Client
	logger *slog.Logger
	sent   map[string]time.Time // approvalID → 送信時刻
}

// New は Manager を生成する。
func New(cfg Config, logger *slog.Logger) *Manager {
	if logger == nil {
		logger = slog.Default()
	}
	return &Manager{
		cfg:    cfg,
		client: &http.Client{Timeout: sendTimeout},
		logger: logger,
		sent:   map[string]time.Time{},
	}
}

// UpdateConfig は設定を動的に差し替える（Settings から保存された際に呼ぶ）。
func (m *Manager) UpdateConfig(cfg Config) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.cfg = cfg
}

// SendApproval は承認待ち通知をすべての有効 backend に非同期送信する。
// events に "approval" が含まれていないか backend が空の場合は何もしない。
func (m *Manager) SendApproval(payload ApprovalPayload) {
	m.mu.Lock()
	cfg := m.cfg
	m.pruneSentLocked(time.Now())

	if !hasEvent(cfg.Events, "approval") || len(cfg.Backends) == 0 {
		m.mu.Unlock()
		return
	}
	id := strings.TrimSpace(payload.ID)
	if id == "" {
		id = fmt.Sprintf("approval-%d-%x", payload.SessionID, time.Now().UnixNano())
		payload.ID = id
	}
	if _, ok := m.sent[id]; ok {
		m.mu.Unlock()
		return
	}
	m.sent[id] = time.Now()
	m.mu.Unlock()

	body := fmt.Sprintf("session #%d: 承認要求", payload.SessionID)
	if cfg.IncludeBody {
		body = sessionlog.MaskSecrets(payload.Body)
	}
	body = truncateUTF8(body, payloadMaxBytes)

	for _, backend := range cfg.Backends {
		b := backend
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), sendTimeout)
			defer cancel()
			if err := sendApproval(ctx, m.client, b, payload, body); err != nil {
				m.logger.Warn("notify send failed",
					"type", b.Type,
					"host", notifyHostOf(b.URL),
					"err", notifySanitizeErr(err))
			}
		}()
	}
}

func sendApproval(ctx context.Context, client *http.Client, backend BackendConfig, payload ApprovalPayload, body string) error {
	if strings.EqualFold(strings.TrimSpace(backend.Type), "ntfy") {
		return sendNtfyApproval(ctx, client, backend, payload, body)
	}
	return send(ctx, client, backend, payload.Title, body)
}

// SendDone はタスク完了通知をすべての有効 backend に非同期送信する。
// events に "done" が含まれていないか backend が空の場合は何もしない。
func (m *Manager) SendDone(payload DonePayload) {
	m.mu.Lock()
	cfg := m.cfg
	m.pruneSentLocked(time.Now())

	if !hasEvent(cfg.Events, "done") || len(cfg.Backends) == 0 {
		m.mu.Unlock()
		return
	}
	id := strings.TrimSpace(payload.ID)
	if id == "" {
		id = fmt.Sprintf("done-%d-%x", payload.SessionID, time.Now().UnixNano())
		payload.ID = id
	}
	if _, ok := m.sent[id]; ok {
		m.mu.Unlock()
		return
	}
	m.sent[id] = time.Now()
	m.mu.Unlock()

	body := truncateUTF8(payload.Summary, payloadMaxBytes)
	title := doneNotificationTitle(payload.Title, payload.Kind)

	for _, backend := range cfg.Backends {
		b := backend
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), sendTimeout)
			defer cancel()
			if err := sendDone(ctx, m.client, b, title, body, payload.Kind); err != nil {
				m.logger.Warn("notify done send failed",
					"type", b.Type,
					"host", notifyHostOf(b.URL),
					"err", notifySanitizeErr(err))
			}
		}()
	}
}

func doneNotificationTitle(title, kind string) string {
	label := map[string]string{"success": "成功", "failure": "失敗", "aborted": "中断", "needs_action": "要判断"}[strings.ToLower(strings.TrimSpace(kind))]
	if label == "" {
		label = "完了"
	}
	return "[" + label + "] " + title
}

func sendDone(ctx context.Context, client *http.Client, backend BackendConfig, title, body, kind string) error {
	if strings.EqualFold(strings.TrimSpace(backend.Type), "ntfy") {
		return sendNtfyDone(ctx, client, backend, title, body, kind)
	}
	if strings.EqualFold(strings.TrimSpace(backend.Type), "webhook") {
		return sendWebhookDone(ctx, client, backend, title, body, kind)
	}
	return fmt.Errorf("unknown backend type: %q", backend.Type)
}

func sendNtfyDone(ctx context.Context, client *http.Client, backend BackendConfig, title, body, kind string) error {
	topicURL := strings.TrimRight(backend.URL, "/") + "/" + strings.TrimSpace(backend.Topic)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, topicURL, strings.NewReader(body))
	if err != nil {
		return fmt.Errorf("ntfy: build request: %w", err)
	}
	tags, priority := "white_check_mark", "default"
	switch strings.ToLower(strings.TrimSpace(kind)) {
	case "failure":
		tags, priority = "x", "high"
	case "aborted":
		tags, priority = "pause_button", "default"
	case "needs_action":
		tags, priority = "warning", "high"
	}
	req.Header.Set("Title", title)
	req.Header.Set("Priority", priority)
	req.Header.Set("Tags", tags)
	req.Header.Set("Content-Type", "text/plain; charset=utf-8")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("ntfy: send: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("ntfy: unexpected status %d", resp.StatusCode)
	}
	return nil
}

func sendWebhookDone(ctx context.Context, client *http.Client, backend BackendConfig, title, body, kind string) error {
	payload := map[string]string{"title": title, "body": body, "kind": kind}
	b, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("webhook: marshal: %w", err)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, backend.URL, bytes.NewReader(b))
	if err != nil {
		return fmt.Errorf("webhook: build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("webhook: send: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("webhook: unexpected status %d", resp.StatusCode)
	}
	return nil
}

// SendSecurity はセキュリティ警告（SEC-C 新規デバイス接続など）を全 backend に送る。
// 重要度が高いため events フィルタは無視し、backend が設定されていれば常に送る。
// 同一 title+body は sentTTL 内では 1 回だけ送る（連投抑制）。
func (m *Manager) SendSecurity(title, body string) {
	m.mu.Lock()
	cfg := m.cfg
	m.pruneSentLocked(time.Now())
	if len(cfg.Backends) == 0 {
		m.mu.Unlock()
		return
	}
	id := "security-" + title + "-" + body
	if _, ok := m.sent[id]; ok {
		m.mu.Unlock()
		return
	}
	m.sent[id] = time.Now()
	m.mu.Unlock()

	out := truncateUTF8(body, payloadMaxBytes)
	for _, backend := range cfg.Backends {
		b := backend
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), sendTimeout)
			defer cancel()
			if err := send(ctx, m.client, b, title, out); err != nil {
				m.logger.Warn("notify security send failed",
					"type", b.Type,
					"host", notifyHostOf(b.URL),
					"err", notifySanitizeErr(err))
			}
		}()
	}
}

// SendTest は Settings のテスト送信ボタン用。指定 backend に即時送信して err を返す。
func (m *Manager) SendTest(ctx context.Context, backend BackendConfig, title, body string) error {
	return send(ctx, m.client, backend, title, body)
}

// send は 1 件の backend に HTTP POST する。
func send(ctx context.Context, client *http.Client, backend BackendConfig, title, body string) error {
	switch strings.ToLower(strings.TrimSpace(backend.Type)) {
	case "ntfy":
		return sendNtfy(ctx, client, backend, title, body)
	case "webhook":
		return sendWebhook(ctx, client, backend, title, body)
	default:
		return fmt.Errorf("unknown backend type: %q", backend.Type)
	}
}

// sendNtfy は ntfy.sh / self-hosted ntfy へ POST する。
// https://docs.ntfy.sh/publish/
func sendNtfy(ctx context.Context, client *http.Client, backend BackendConfig, title, body string) error {
	topicURL := strings.TrimRight(backend.URL, "/") + "/" + strings.TrimSpace(backend.Topic)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, topicURL, strings.NewReader(body))
	if err != nil {
		return fmt.Errorf("ntfy: build request: %w", err)
	}
	req.Header.Set("Title", title)
	req.Header.Set("Priority", "high")
	req.Header.Set("Tags", "bell")
	req.Header.Set("Content-Type", "text/plain; charset=utf-8")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("ntfy: send: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("ntfy: unexpected status %d", resp.StatusCode)
	}
	return nil
}

// sendNtfyApproval adds notification actions only for absolute HTTPS/HTTP
// targets supplied by the Hub. The Hub omits them when no mobile-reachable
// secure URL is configured, rather than accidentally pointing a phone at its
// own localhost. The short-lived action token is contained only in the action
// URL; the Hub token is never present.
func sendNtfyApproval(ctx context.Context, client *http.Client, backend BackendConfig, payload ApprovalPayload, body string) error {
	topicURL := strings.TrimRight(backend.URL, "/") + "/" + strings.TrimSpace(backend.Topic)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, topicURL, strings.NewReader(body))
	if err != nil {
		return fmt.Errorf("ntfy: build request: %w", err)
	}
	req.Header.Set("Title", payload.Title)
	req.Header.Set("Priority", "high")
	req.Header.Set("Tags", "bell")
	req.Header.Set("Content-Type", "text/plain; charset=utf-8")
	if actions := ntfyApprovalActions(payload); actions != "" {
		req.Header.Set("Actions", actions)
	}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("ntfy: send: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("ntfy: unexpected status %d", resp.StatusCode)
	}
	return nil
}

func ntfyApprovalActions(payload ApprovalPayload) string {
	actions := make([]string, 0, 3)
	if isSafeActionURL(payload.ApproveURL) && strings.ToLower(strings.TrimSpace(payload.Risk)) != "high" {
		actions = append(actions, "http, Approve, "+payload.ApproveURL+", method=POST")
	}
	if isSafeActionURL(payload.RejectURL) {
		actions = append(actions, "http, Reject, "+payload.RejectURL+", method=POST")
	}
	if isSafeActionURL(payload.OpenURL) {
		actions = append(actions, "view, Open, "+payload.OpenURL)
	}
	return strings.Join(actions, "; ")
}

func isSafeActionURL(raw string) bool {
	u, err := neturl.Parse(strings.TrimSpace(raw))
	return err == nil && u != nil && (u.Scheme == "https" || u.Scheme == "http") && u.Host != "" && u.User == nil && u.Fragment == ""
}

// sendWebhook は汎用 webhook へ JSON POST する。
func sendWebhook(ctx context.Context, client *http.Client, backend BackendConfig, title, body string) error {
	payload := map[string]string{
		"title": title,
		"body":  body,
	}
	b, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("webhook: marshal: %w", err)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, backend.URL, bytes.NewReader(b))
	if err != nil {
		return fmt.Errorf("webhook: build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("webhook: send: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("webhook: unexpected status %d", resp.StatusCode)
	}
	return nil
}

// notifySanitizeErr は送信失敗 err をログ出力用に整形する。
// http.Client.Do が返す *url.Error は対象 URL 全体（ntfy トピック秘密や
// webhook の ?token=... を含む）を Error() に埋め込むため、生のまま
// hub.log へ出すと通知チャンネルの秘密が平文で残る。ここで URL を剥がし、
// transport 由来のメッセージのみに縮約する。host は呼び出し側が backend.URL
// から別途取り出す（notifyHostOf）。
func notifySanitizeErr(err error) string {
	if err == nil {
		return ""
	}
	var ue *neturl.Error
	if errors.As(err, &ue) {
		// ue.Err は URL を含まない transport / context 由来のエラー。
		// ue.Op（"Post" 等）と合わせて種別のみを残す。
		op := strings.TrimSpace(ue.Op)
		if inner := ue.Err; inner != nil {
			if op != "" {
				return op + ": " + inner.Error()
			}
			return inner.Error()
		}
		if op != "" {
			return op + ": request error"
		}
		return "request error"
	}
	// *url.Error 以外（build request / marshal / status N 等）は URL を
	// 含まない自前 fmt.Errorf 文言なのでそのまま返してよい。
	return err.Error()
}

// notifyHostOf は backend URL から host:port のみを取り出す。
// パース不能・空のときは "unknown" を返す（URL 全体は決して返さない）。
func notifyHostOf(rawURL string) string {
	u, err := neturl.Parse(strings.TrimSpace(rawURL))
	if err != nil || u == nil || u.Host == "" {
		return "unknown"
	}
	return u.Host
}

func hasEvent(events []string, target string) bool {
	for _, e := range events {
		if strings.EqualFold(strings.TrimSpace(e), target) {
			return true
		}
	}
	return false
}

func (m *Manager) pruneSentLocked(now time.Time) {
	for k, ts := range m.sent {
		if now.Sub(ts) > sentTTL {
			delete(m.sent, k)
		}
	}
}

func truncateUTF8(s string, max int) string {
	if max <= 0 {
		return ""
	}
	if len(s) <= max {
		return s
	}
	cut := 0
	for i := range s {
		if i > max {
			break
		}
		cut = i
	}
	if cut <= 0 {
		return ""
	}
	return strings.TrimSpace(s[:cut])
}
