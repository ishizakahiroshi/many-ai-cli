package hub

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	webpush "github.com/SherClockHolmes/webpush-go"
	"many-ai-cli/internal/config"
	notifyPkg "many-ai-cli/internal/notify"
	"many-ai-cli/internal/sessionlog"
)

const (
	pushStoreFilename = "push_store.json"
	pushSendTimeout   = 10 * time.Second
	pushPayloadMaxLen = 180
)

type pushSubscription struct {
	Endpoint  string       `json:"endpoint"`
	Keys      webpush.Keys `json:"keys"`
	UserAgent string       `json:"user_agent,omitempty"`
	CreatedAt string       `json:"created_at,omitempty"`
	LastSeen  string       `json:"last_seen,omitempty"`
}

type pushStoreFile struct {
	VAPIDPublicKey  string             `json:"vapid_public_key"`
	VAPIDPrivateKey string             `json:"vapid_private_key"`
	Subscriptions   []pushSubscription `json:"subscriptions,omitempty"`
}

type pushApprovalPayload struct {
	ID        string `json:"id"`
	SessionID int    `json:"session_id"`
	Provider  string `json:"provider,omitempty"`
	Title     string `json:"title"`
	Body      string `json:"body"`
	URL       string `json:"url,omitempty"`
}

type pushStatus struct {
	Supported     bool   `json:"supported"`
	PublicKey     string `json:"public_key"`
	Subscriptions int    `json:"subscriptions"`
}

type pushSubscriptionRequest struct {
	Endpoint string       `json:"endpoint"`
	Keys     webpush.Keys `json:"keys"`
}

type pushManager struct {
	mu         sync.Mutex
	path       string
	store      pushStoreFile
	logger     *slog.Logger
	httpClient webpush.HTTPClient
	sent       map[string]time.Time
}

func newPushManager(logger *slog.Logger) (*pushManager, error) {
	dir, err := config.Dir()
	if err != nil {
		return nil, err
	}
	pm := &pushManager{
		path:       filepath.Join(dir, pushStoreFilename),
		logger:     logger,
		httpClient: makeExternalHTTPClient(pushSendTimeout),
		sent:       map[string]time.Time{},
	}
	if err := pm.loadOrCreate(); err != nil {
		return nil, err
	}
	return pm, nil
}

func (pm *pushManager) loadOrCreate() error {
	pm.mu.Lock()
	defer pm.mu.Unlock()
	data, err := os.ReadFile(pm.path)
	switch {
	case err == nil:
		if len(data) > 0 {
			if err := json.Unmarshal(data, &pm.store); err != nil {
				return fmt.Errorf("parse push store: %w", err)
			}
		}
	case os.IsNotExist(err):
		// handled below
	default:
		return fmt.Errorf("read push store: %w", err)
	}
	if strings.TrimSpace(pm.store.VAPIDPublicKey) != "" && strings.TrimSpace(pm.store.VAPIDPrivateKey) != "" {
		return nil
	}
	privateKey, publicKey, err := webpush.GenerateVAPIDKeys()
	if err != nil {
		return fmt.Errorf("generate VAPID keys: %w", err)
	}
	pm.store.VAPIDPrivateKey = privateKey
	pm.store.VAPIDPublicKey = publicKey
	return pm.saveLocked()
}

func (pm *pushManager) status() pushStatus {
	pm.mu.Lock()
	defer pm.mu.Unlock()
	return pushStatus{
		Supported:     pm.store.VAPIDPublicKey != "",
		PublicKey:     pm.store.VAPIDPublicKey,
		Subscriptions: len(pm.store.Subscriptions),
	}
}

func (pm *pushManager) publicKey() string {
	pm.mu.Lock()
	defer pm.mu.Unlock()
	return pm.store.VAPIDPublicKey
}

func (pm *pushManager) upsertSubscription(sub pushSubscription) error {
	sub.Endpoint = strings.TrimSpace(sub.Endpoint)
	sub.Keys.Auth = strings.TrimSpace(sub.Keys.Auth)
	sub.Keys.P256dh = strings.TrimSpace(sub.Keys.P256dh)
	if sub.Endpoint == "" || sub.Keys.Auth == "" || sub.Keys.P256dh == "" {
		return errors.New("subscription endpoint and keys are required")
	}
	now := time.Now().Format(time.RFC3339)
	pm.mu.Lock()
	defer pm.mu.Unlock()
	for i := range pm.store.Subscriptions {
		if pm.store.Subscriptions[i].Endpoint == sub.Endpoint {
			sub.CreatedAt = pm.store.Subscriptions[i].CreatedAt
			if sub.CreatedAt == "" {
				sub.CreatedAt = now
			}
			sub.LastSeen = now
			pm.store.Subscriptions[i] = sub
			return pm.saveLocked()
		}
	}
	sub.CreatedAt = now
	sub.LastSeen = now
	pm.store.Subscriptions = append(pm.store.Subscriptions, sub)
	return pm.saveLocked()
}

func (pm *pushManager) deleteSubscription(endpoint string) error {
	endpoint = strings.TrimSpace(endpoint)
	if endpoint == "" {
		return errors.New("subscription endpoint is required")
	}
	pm.mu.Lock()
	defer pm.mu.Unlock()
	next := pm.store.Subscriptions[:0]
	for _, sub := range pm.store.Subscriptions {
		if sub.Endpoint != endpoint {
			next = append(next, sub)
		}
	}
	pm.store.Subscriptions = next
	return pm.saveLocked()
}

func (pm *pushManager) sendApproval(ctx context.Context, payload pushApprovalPayload) {
	payload.ID = strings.TrimSpace(payload.ID)
	if payload.ID == "" {
		payload.ID = fmt.Sprintf("session-%d-%x", payload.SessionID, time.Now().UnixNano())
	}
	pm.mu.Lock()
	pm.pruneSentLocked(time.Now())
	if _, ok := pm.sent[payload.ID]; ok {
		pm.mu.Unlock()
		return
	}
	store := pm.store
	pm.mu.Unlock()

	if len(store.Subscriptions) == 0 || store.VAPIDPublicKey == "" || store.VAPIDPrivateKey == "" {
		return
	}
	payload.Body = truncateUTF8Bytes(payload.Body, pushPayloadMaxLen)
	body, err := json.Marshal(map[string]any{
		"type":       "approval_waiting",
		"id":         payload.ID,
		"session_id": payload.SessionID,
		"provider":   payload.Provider,
		"title":      payload.Title,
		"body":       payload.Body,
		"url":        payload.URL,
	})
	if err != nil {
		return
	}

	var expired []string
	sentOK := 0
	for _, sub := range store.Subscriptions {
		select {
		case <-ctx.Done():
			return
		default:
		}
		resp, err := webpush.SendNotificationWithContext(ctx, body, &webpush.Subscription{
			Endpoint: sub.Endpoint,
			Keys:     sub.Keys,
		}, &webpush.Options{
			HTTPClient:      pm.httpClient,
			Subscriber:      "mailto:many-ai-cli@localhost.invalid",
			VAPIDPublicKey:  store.VAPIDPublicKey,
			VAPIDPrivateKey: store.VAPIDPrivateKey,
			TTL:             300,
			Topic:           topicForPush(payload.ID),
		})
		if err != nil {
			if pm.logger != nil {
				pm.logger.Warn("web push send failed", "endpoint_hash", endpointHash(sub.Endpoint), "err", pushSanitizeErr(err))
			}
			continue
		}
		if resp != nil {
			if resp.StatusCode == http.StatusGone || resp.StatusCode == http.StatusNotFound {
				expired = append(expired, sub.Endpoint)
			} else {
				sentOK++
			}
			_ = resp.Body.Close()
		}
	}
	// dedup マークは「送信試行前」ではなく「最低1件成功後」に記録する。全送信失敗
	// （例: 一時的なネットワーク全断）のときはマークせず、同一 ID の次回通知で再送可能に
	// する。成功時は従来通り pruneSentLocked が消すまで（1時間）dedup される。
	if sentOK > 0 {
		pm.mu.Lock()
		pm.sent[payload.ID] = time.Now()
		pm.mu.Unlock()
	}
	if len(expired) > 0 {
		pm.removeExpired(expired)
	}
}

// sendSecurity は SEC-C セキュリティ警告（新規デバイス接続）を全購読へ Web Push する。
// sw.ts の push ハンドラは title/body をそのまま表示するため専用 type でも問題ない。
func (pm *pushManager) sendSecurity(ctx context.Context, title, body string) {
	pm.mu.Lock()
	store := pm.store
	pm.mu.Unlock()
	if len(store.Subscriptions) == 0 || store.VAPIDPublicKey == "" || store.VAPIDPrivateKey == "" {
		return
	}
	body = truncateUTF8Bytes(body, pushPayloadMaxLen)
	sum := sha256.Sum256([]byte(title + "\x00" + body))
	id := "security-" + hex.EncodeToString(sum[:])[:16]
	payload, err := json.Marshal(map[string]any{
		"type":  "security_alert",
		"id":    id,
		"title": title,
		"body":  body,
		"url":   "/",
	})
	if err != nil {
		return
	}
	var expired []string
	for _, sub := range store.Subscriptions {
		select {
		case <-ctx.Done():
			return
		default:
		}
		resp, err := webpush.SendNotificationWithContext(ctx, payload, &webpush.Subscription{
			Endpoint: sub.Endpoint,
			Keys:     sub.Keys,
		}, &webpush.Options{
			HTTPClient:      pm.httpClient,
			Subscriber:      "mailto:many-ai-cli@localhost.invalid",
			VAPIDPublicKey:  store.VAPIDPublicKey,
			VAPIDPrivateKey: store.VAPIDPrivateKey,
			TTL:             300,
			Topic:           topicForPush(id),
		})
		if err != nil {
			if pm.logger != nil {
				pm.logger.Warn("web push security send failed", "endpoint_hash", endpointHash(sub.Endpoint), "err", pushSanitizeErr(err))
			}
			continue
		}
		if resp != nil {
			if resp.StatusCode == http.StatusGone || resp.StatusCode == http.StatusNotFound {
				expired = append(expired, sub.Endpoint)
			}
			_ = resp.Body.Close()
		}
	}
	if len(expired) > 0 {
		pm.removeExpired(expired)
	}
}

// pushSanitizeErr は web push 送信エラーから購読エンドポイント URL（push 購読の秘密を
// 含み得る）を取り除き、操作種別とトランスポート由来メッセージのみを残す。hub.log への
// エンドポイント URL 平文記録を防ぐ（notify.go のエラー整形と方針を揃える）。
func pushSanitizeErr(err error) string {
	if err == nil {
		return ""
	}
	var ue *url.Error
	if errors.As(err, &ue) {
		msg := "error"
		if ue.Err != nil {
			msg = ue.Err.Error()
		}
		if ue.Op != "" {
			return ue.Op + ": " + msg
		}
		return msg
	}
	return err.Error()
}

func (pm *pushManager) removeExpired(endpoints []string) {
	expired := make(map[string]struct{}, len(endpoints))
	for _, endpoint := range endpoints {
		expired[endpoint] = struct{}{}
	}
	pm.mu.Lock()
	defer pm.mu.Unlock()
	next := pm.store.Subscriptions[:0]
	for _, sub := range pm.store.Subscriptions {
		if _, ok := expired[sub.Endpoint]; !ok {
			next = append(next, sub)
		}
	}
	pm.store.Subscriptions = next
	if err := pm.saveLocked(); err != nil && pm.logger != nil {
		pm.logger.Warn("save push store after expiry failed", "err", err)
	}
}

func (pm *pushManager) pruneSentLocked(now time.Time) {
	for k, ts := range pm.sent {
		if now.Sub(ts) > time.Hour {
			delete(pm.sent, k)
		}
	}
}

func (pm *pushManager) saveLocked() error {
	dir := filepath.Dir(pm.path)
	if err := os.MkdirAll(dir, config.DirMode); err != nil {
		return fmt.Errorf("mkdir push store dir: %w", err)
	}
	if err := os.Chmod(dir, config.DirMode); err != nil {
		return fmt.Errorf("chmod push store dir: %w", err)
	}
	data, err := json.MarshalIndent(pm.store, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal push store: %w", err)
	}
	tmp, err := os.CreateTemp(dir, "push-store-*.json.tmp")
	if err != nil {
		return fmt.Errorf("create temp push store: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp push store: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temp push store: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp push store: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp push store: %w", err)
	}
	return os.Rename(tmpName, pm.path)
}

func topicForPush(id string) string {
	sum := sha256.Sum256([]byte(id))
	return hex.EncodeToString(sum[:])[:24]
}

func endpointHash(endpoint string) string {
	sum := sha256.Sum256([]byte(endpoint))
	return hex.EncodeToString(sum[:])[:12]
}

func (s *Server) handlePushStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.push == nil {
		writeJSON(w, pushStatus{Supported: false})
		return
	}
	writeJSON(w, s.push.status())
}

func (s *Server) handlePushVAPIDPublicKey(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.push == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "push_unavailable", "push unavailable")
		return
	}
	writeJSON(w, map[string]string{"public_key": s.push.publicKey()})
}

func (s *Server) handlePushSubscriptions(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost, http.MethodDelete) {
		return
	}
	if s.push == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "push_unavailable", "push unavailable")
		return
	}
	var req pushSubscriptionRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	switch r.Method {
	case http.MethodPost:
		if err := s.push.upsertSubscription(pushSubscription{
			Endpoint:  req.Endpoint,
			Keys:      req.Keys,
			UserAgent: strings.TrimSpace(r.UserAgent()),
		}); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_subscription", err.Error())
			return
		}
	case http.MethodDelete:
		if err := s.push.deleteSubscription(req.Endpoint); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_subscription", err.Error())
			return
		}
	}
	writeJSON(w, map[string]bool{"ok": true})
}

func (s *Server) notifyApprovalPush(id int, approvalID, provider, question, contextText string) {
	if s.push == nil {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	titleName := strings.TrimSpace(ses.Display)
	if titleName == "" {
		titleName = strings.TrimSpace(ses.Provider)
	}
	if titleName == "" {
		titleName = "many-ai-cli"
	}
	if ses.Label != "" {
		titleName = fmt.Sprintf("%s #%d [%s]", titleName, id, ses.Label)
	} else {
		titleName = fmt.Sprintf("%s #%d", titleName, id)
	}
	body := firstNonEmpty(question, contextText, ses.LastMessage, ses.FirstMessage, ses.CWD, "Approval is waiting.")
	s.sessionsMu.Unlock()
	// 承認 question/context は生 PTY テキスト由来で未マスク。ntfy/webhook/Web Push
	// という端末外の第三者へ送出する前に MaskSecrets を通す（全外部送出の単一ボトルネック）。
	body = sessionlog.MaskSecrets(body)
	approvalID = strings.TrimSpace(approvalID)
	if approvalID == "" {
		approvalID = fmt.Sprintf("session-%d-%s", id, body)
	}
	body = strings.Join(strings.Fields(body), " ")
	url := approvalPushURL(id)
	payload := pushApprovalPayload{
		ID:        approvalID,
		SessionID: id,
		Provider:  provider,
		Title:     titleName,
		Body:      body,
		URL:       url,
	}
	s.safeGo("web push approval", func() {
		ctx, cancel := context.WithTimeout(context.Background(), pushSendTimeout)
		defer cancel()
		s.push.sendApproval(ctx, payload)
	})
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func approvalPushURL(id int) string {
	return fmt.Sprintf("/?session_id=%d", id)
}

// notifyApprovalOutbound は ntfy/webhook バックエンドへの承認通知を行う。
// notifyApprovalPush (Web Push) と同じ引数・同じイベント箇所から呼ばれる。
func (s *Server) notifyApprovalOutbound(id int, approvalID, provider, question, contextText string) {
	if s.notifyMgr == nil {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	titleName := strings.TrimSpace(ses.Display)
	if titleName == "" {
		titleName = strings.TrimSpace(ses.Provider)
	}
	if titleName == "" {
		titleName = "many-ai-cli"
	}
	if ses.Label != "" {
		titleName = fmt.Sprintf("%s #%d [%s]", titleName, id, ses.Label)
	} else {
		titleName = fmt.Sprintf("%s #%d", titleName, id)
	}
	body := firstNonEmpty(question, contextText, ses.LastMessage, ses.FirstMessage, ses.CWD, "Approval is waiting.")
	s.sessionsMu.Unlock()
	// 承認 question/context は生 PTY テキスト由来で未マスク。ntfy/webhook という
	// 端末外の第三者へ送出する前に MaskSecrets を通す（全外部送出の単一ボトルネック）。
	body = sessionlog.MaskSecrets(body)
	body = strings.Join(strings.Fields(body), " ")

	s.notifyMgr.SendApproval(notifyPkg.ApprovalPayload{
		ID:        approvalID,
		SessionID: id,
		Provider:  provider,
		Title:     titleName,
		Body:      body,
	})
}

// configToNotify は config.NotifyConfig を notify.Config に変換する。
func configToNotify(cfg config.NotifyConfig) notifyPkg.Config {
	backends := make([]notifyPkg.BackendConfig, len(cfg.Backends))
	for i, b := range cfg.Backends {
		backends[i] = notifyPkg.BackendConfig{
			Type:  b.Type,
			URL:   b.URL,
			Topic: b.Topic,
		}
	}
	events := make([]string, len(cfg.Events))
	copy(events, cfg.Events)
	return notifyPkg.Config{
		Backends: backends,
		Events:   events,
	}
}

func truncateUTF8Bytes(s string, max int) string {
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
