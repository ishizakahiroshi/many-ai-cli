package hub

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// subscriptionStatusTimeout は Test ボタンが公式 CLI の status サブコマンドを
// 待つ上限。ネットワークを見に行く実装でも Settings 画面が固まらない長さ。
const subscriptionStatusTimeout = 20 * time.Second

// subscriptionNameMaxLen は表示名の上限。UI のカード幅に収まる範囲で切る。
const subscriptionNameMaxLen = 80

// handleSubscriptions は GET で一覧、POST で新規登録を扱う。
func (s *Server) handleSubscriptions(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.writeSubscriptionList(w)
	case http.MethodPost:
		s.handleSubscriptionAdd(w, r)
	}
}

// handleSubscriptionsItem は /api/subscriptions/<action> を捌く。
func (s *Server) handleSubscriptionsItem(w http.ResponseWriter, r *http.Request) {
	action := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/subscriptions/"), "/")
	switch action {
	case "update":
		s.handleSubscriptionUpdate(w, r)
	case "remove":
		s.handleSubscriptionRemove(w, r)
	case "test":
		s.handleSubscriptionTest(w, r)
	case "login":
		s.handleSubscriptionLogin(w, r)
	default:
		if !s.guard(w, r, http.MethodGet, http.MethodPost) {
			return
		}
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
	}
}

func (s *Server) writeSubscriptionList(w http.ResponseWriter) {
	cfg := s.snapshotCfg()
	dir := subscriptionConfigDir()
	writeJSON(w, map[string]any{
		"providers": subscription.List(cfg, dir),
		"root":      config.SubscriptionsRoot(dir),
	})
}

// subscriptionRequest は update / remove / test / login 共通の識別部。
type subscriptionRequest struct {
	Provider string `json:"provider"`
	ID       string `json:"id"`
}

func (s *Server) handleSubscriptionAdd(w http.ResponseWriter, r *http.Request) {
	var body struct {
		Provider string `json:"provider"`
		ID       string `json:"id"`
		Name     string `json:"name"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	if _, ok := subscription.AdapterFor(provider); !ok {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "provider does not support subscription profiles")
		return
	}
	name := trimDisplayName(body.Name)
	dir := subscriptionConfigDir()
	if dir == "" {
		writeJSONError(w, http.StatusInternalServerError, "config_dir_error", "cannot resolve the many-ai-cli config directory")
		return
	}

	// ID の採番・重複検査・登録は 1 回の cfgMu 保持で済ませる。分けると、2 つの
	// 追加要求が同じ採番を得て同じディレクトリを共有しうる。
	s.cfgMu.Lock()
	existing := map[string]bool{}
	for _, p := range s.cfg.Subscriptions[provider] {
		existing[config.NormalizeSubscriptionID(p.ID)] = true
	}
	id := config.NormalizeSubscriptionID(body.ID)
	if id == "" {
		id = uniqueSubscriptionID(slugifySubscriptionID(name), existing)
	}
	if err := config.ValidateSubscriptionID(id); err != nil {
		s.cfgMu.Unlock()
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	if existing[id] {
		s.cfgMu.Unlock()
		writeJSONError(w, http.StatusConflict, "duplicate_id", fmt.Sprintf("subscription id %q already exists for %s", id, provider))
		return
	}
	profile := config.SubscriptionProfile{ID: id, Name: name}
	profileDir, err := config.ResolveSubscriptionProfileDir(dir, provider, profile)
	if err != nil {
		s.cfgMu.Unlock()
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	if s.cfg.Subscriptions == nil {
		s.cfg.Subscriptions = config.SubscriptionProfiles{}
	}
	s.cfg.Subscriptions[provider] = append(s.cfg.Subscriptions[provider], profile)
	s.cfgMu.Unlock()

	// ディレクトリ作成とファイル書き込みはロックの外で行う（cfgMu を I/O で
	// 握らないという既存方針）。失敗したら登録を巻き戻す。
	if err := subscription.EnsureProfileDir(profileDir); err != nil {
		s.removeSubscriptionEntry(provider, id)
		writeJSONError(w, http.StatusInternalServerError, "profile_dir_error", errorDetail("create profile dir", err))
		return
	}
	if err := s.persistConfig(); err != nil {
		s.removeSubscriptionEntry(provider, id)
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}
	s.logger.Info("subscription profile added", "provider", provider, "id", id)
	s.writeSubscriptionList(w)
}

// removeSubscriptionEntry drops one profile from the in-memory config. It is the
// rollback for a half-finished add; the caller has already reported the failure.
func (s *Server) removeSubscriptionEntry(provider, id string) {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	list := s.cfg.Subscriptions[provider]
	kept := make([]config.SubscriptionProfile, 0, len(list))
	for _, p := range list {
		if config.NormalizeSubscriptionID(p.ID) == id {
			continue
		}
		kept = append(kept, p)
	}
	if len(kept) == 0 {
		delete(s.cfg.Subscriptions, provider)
		return
	}
	s.cfg.Subscriptions[provider] = kept
}

func (s *Server) handleSubscriptionUpdate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		subscriptionRequest
		Name    *string `json:"name"`
		Enabled *bool   `json:"enabled"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	id := config.NormalizeSubscriptionID(body.ID)
	if err := config.ValidateSubscriptionID(id); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	s.cfgMu.Lock()
	found := false
	for i, p := range s.cfg.Subscriptions[provider] {
		if config.NormalizeSubscriptionID(p.ID) != id {
			continue
		}
		if body.Name != nil {
			s.cfg.Subscriptions[provider][i].Name = trimDisplayName(*body.Name)
		}
		if body.Enabled != nil {
			v := *body.Enabled
			s.cfg.Subscriptions[provider][i].Enabled = &v
		}
		found = true
		break
	}
	s.cfgMu.Unlock()
	if !found {
		writeJSONError(w, http.StatusNotFound, "not_found", "subscription profile not found")
		return
	}
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}
	s.writeSubscriptionList(w)
}

func (s *Server) handleSubscriptionRemove(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		subscriptionRequest
		// DeleteCredentials は vendor CLI の認証ファイルごと消すかどうか。
		// 既定 false = many-ai-cli の登録解除のみ。UI 側もチェックを外した状態を
		// 既定にすること（意図しない再ログイン要求を避けるため）。
		DeleteCredentials bool `json:"delete_credentials"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	id := config.NormalizeSubscriptionID(body.ID)
	if err := config.ValidateSubscriptionID(id); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	dir := subscriptionConfigDir()
	if body.DeleteCredentials && s.subscriptionProfileInUse(provider, id) {
		writeJSONError(w, http.StatusConflict, "profile_in_use", "cannot delete credentials while the subscription profile is used by a live session")
		return
	}

	s.cfgMu.Lock()
	list := s.cfg.Subscriptions[provider]
	kept := make([]config.SubscriptionProfile, 0, len(list))
	var removed *config.SubscriptionProfile
	for _, p := range list {
		if removed == nil && config.NormalizeSubscriptionID(p.ID) == id {
			cp := p
			removed = &cp
			continue
		}
		kept = append(kept, p)
	}
	if removed != nil {
		if len(kept) == 0 {
			delete(s.cfg.Subscriptions, provider)
		} else {
			s.cfg.Subscriptions[provider] = kept
		}
	}
	s.cfgMu.Unlock()
	if removed == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "subscription profile not found")
		return
	}
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}

	credentialsDeleted := false
	if body.DeleteCredentials && dir != "" {
		profileDir, err := config.ResolveSubscriptionProfileDir(dir, provider, *removed)
		switch {
		case err != nil:
			s.logger.Warn("subscription profile dir not deleted: unresolvable", "provider", provider, "id", id, "err", err)
		case strings.TrimSpace(removed.ProfileDir) != "":
			// 利用者が config.yaml へ手書きした外部ディレクトリは消さない。
			// many-ai-cli が作っていない場所を再帰削除しない。
			s.logger.Warn("subscription profile dir not deleted: custom profile_dir is managed by the user",
				"provider", provider, "id", id)
		default:
			if rmErr := os.RemoveAll(profileDir); rmErr != nil {
				s.logger.Warn("subscription profile dir delete failed", "provider", provider, "id", id, "err", rmErr)
			} else {
				credentialsDeleted = true
				s.logger.Info("subscription profile dir deleted", "provider", provider, "id", id)
			}
		}
	}
	s.logger.Info("subscription profile removed", "provider", provider, "id", id, "credentials_deleted", credentialsDeleted)

	cfg := s.snapshotCfg()
	writeJSON(w, map[string]any{
		"providers":           subscription.List(cfg, dir),
		"root":                config.SubscriptionsRoot(dir),
		"credentials_deleted": credentialsDeleted,
	})
}

func (s *Server) handleSubscriptionTest(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body subscriptionRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	adapter, ok := subscription.AdapterFor(provider)
	if !ok {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "provider does not support subscription profiles")
		return
	}
	cfg := s.snapshotCfg()
	dir := subscriptionConfigDir()
	resolved, err := subscription.Resolve(cfg, dir, provider, body.ID)
	if err != nil {
		writeJSONError(w, subscriptionErrorStatus(err), "invalid_subscription", err.Error())
		return
	}
	if resolved == nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "subscription id is required")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), subscriptionStatusTimeout)
	defer cancel()
	status, statusErr := adapter.Status(ctx, resolved.ProfileDir)
	if statusErr != nil {
		// エラー本文には公式 CLI の生出力を載せない（アカウント情報を含みうる）。
		writeJSONError(w, http.StatusBadGateway, "status_failed", statusErr.Error())
		return
	}
	// plan を config へ書き戻し、次回以降 CLI を起動しなくても一覧に出せるようにする。
	if status.LoggedIn && strings.TrimSpace(status.Plan) != "" {
		s.cfgMu.Lock()
		for i, p := range s.cfg.Subscriptions[provider] {
			if config.NormalizeSubscriptionID(p.ID) == resolved.ID {
				s.cfg.Subscriptions[provider][i].Plan = status.Plan
				break
			}
		}
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("failed to persist subscription plan", "provider", provider, "id", resolved.ID, "err", err)
		}
	}
	writeJSON(w, status)
}

func (s *Server) handleSubscriptionLogin(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		subscriptionRequest
		CWD string `json:"cwd"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	if _, ok := subscription.AdapterFor(provider); !ok {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "provider does not support subscription profiles")
		return
	}
	cfg := s.snapshotCfg()
	dir := subscriptionConfigDir()
	resolved, err := subscription.Resolve(cfg, dir, provider, body.ID)
	if err != nil {
		writeJSONError(w, subscriptionErrorStatus(err), "invalid_subscription", err.Error())
		return
	}
	if resolved == nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "subscription id is required")
		return
	}
	// Login must not inherit Hub's process cwd. That is often the directory
	// that holds the running exe (repo-root `dist/` on this project), and the
	// session list then shows a leftover project group named after it.
	// The client may still send cwd; it is ignored on purpose.
	_ = body.CWD
	cwd, cwdErr := ensureSubscriptionLoginRoot(dir)
	if cwdErr != nil {
		writeJSONError(w, http.StatusInternalServerError, "login_cwd", errorDetail("login workspace error", cwdErr))
		return
	}
	label := fmt.Sprintf("login-%s-%s", provider, resolved.ID)
	if !spawnValidModelLabel(label) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid subscription id")
		return
	}
	sessionID, err := s.spawnWrappedSession(spawnWrappedSpec{
		Provider:              provider,
		CWD:                   cwd,
		Label:                 label,
		SubscriptionProfileID: resolved.ID,
		SubscriptionLogin:     true,
	}, 20*time.Second)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("login session spawn error", err))
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session_id": sessionID})
}

func subscriptionErrorStatus(err error) int {
	switch {
	case errors.Is(err, subscription.ErrProfileNotFound):
		return http.StatusNotFound
	case errors.Is(err, subscription.ErrProfileDisabled), errors.Is(err, subscription.ErrProviderUnsupported):
		return http.StatusBadRequest
	default:
		return http.StatusBadRequest
	}
}

func trimDisplayName(name string) string {
	name = strings.TrimSpace(name)
	// 制御文字は UI とログの両方を壊すので落とす。
	name = strings.Map(func(r rune) rune {
		if r < 0x20 || r == 0x7f {
			return -1
		}
		return r
	}, name)
	runes := []rune(name)
	if len(runes) > subscriptionNameMaxLen {
		return string(runes[:subscriptionNameMaxLen])
	}
	return name
}

// slugifySubscriptionID は表示名から ID 候補を作る。ASCII 以外（日本語名など）は
// 落ちるので、その場合は呼び出し側の uniqueSubscriptionID が既定名で埋める。
func slugifySubscriptionID(name string) string {
	var b strings.Builder
	prevDash := false
	for _, r := range strings.ToLower(name) {
		switch {
		case (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9'):
			b.WriteRune(r)
			prevDash = false
		case r == '-' || r == '_' || r == '.' || r == ' ':
			if b.Len() > 0 && !prevDash {
				b.WriteByte('-')
				prevDash = true
			}
		}
	}
	slug := strings.Trim(b.String(), "-")
	if len(slug) > config.MaxSubscriptionIDLen {
		slug = strings.Trim(slug[:config.MaxSubscriptionIDLen], "-")
	}
	return slug
}

// uniqueSubscriptionID は base（空なら "profile"）に連番を足して衝突を避ける。
func uniqueSubscriptionID(base string, existing map[string]bool) string {
	if base == "" {
		base = "profile"
	}
	if !existing[base] {
		return base
	}
	for i := 2; i < 1000; i++ {
		candidate := fmt.Sprintf("%s-%d", base, i)
		if !existing[candidate] {
			return candidate
		}
	}
	return base
}
