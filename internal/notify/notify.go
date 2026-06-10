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
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"time"
)

const (
	sendTimeout      = 10 * time.Second
	payloadMaxBytes  = 200
	// sentTTL は重複送信防止エントリの保持時間。
	sentTTL = time.Hour
)

// BackendConfig は設定ファイルの backends[] 1 件に対応する。
type BackendConfig struct {
	Type  string `yaml:"type"  json:"type"`  // "ntfy" | "webhook"
	URL   string `yaml:"url"   json:"url"`
	Topic string `yaml:"topic" json:"topic"` // ntfy のみ有効
}

// Config は config.yaml の notify: セクション全体に対応する。
type Config struct {
	Backends []BackendConfig `yaml:"backends" json:"backends"`
	Events   []string        `yaml:"events"   json:"events"` // ["approval"]
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
	ID        string // 重複送信防止用。空の場合は自動生成
	SessionID int
	Provider  string
	Title     string
	Body      string
}

// DonePayload はタスク完了通知の本文。Hub URL / token は含めない。
type DonePayload struct {
	ID        string // 重複送信防止用。空の場合は自動生成
	SessionID int
	Provider  string
	Title     string
	Summary   string
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

	body := truncateUTF8(payload.Body, payloadMaxBytes)

	for _, backend := range cfg.Backends {
		b := backend
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), sendTimeout)
			defer cancel()
			if err := send(ctx, m.client, b, payload.Title, body); err != nil {
				m.logger.Warn("notify send failed",
					"type", b.Type,
					"err", err)
			}
		}()
	}
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

	for _, backend := range cfg.Backends {
		b := backend
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), sendTimeout)
			defer cancel()
			if err := send(ctx, m.client, b, payload.Title, body); err != nil {
				m.logger.Warn("notify done send failed",
					"type", b.Type,
					"err", err)
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
