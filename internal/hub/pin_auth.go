package hub

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/crypto/bcrypt"
)

// 任意リモート PIN（既定 OFF・plan_hub-remote-auth.md / A）。
//
// 設計の芯:
//   - PIN は「非 loopback アクセス時の追加の扉」。loopback は従来どおり token のみで素通し。
//   - 平文 PIN は決して保存・ログ出力しない（bcrypt ハッシュのみ保存）。
//   - 6 桁以上の数字 = ブルートフォース可能なため、連続失敗でロックアウト（指数バックオフ）。
//   - 認証成功で HMAC 署名・期限付き cookie を発行し、期限内はワンタップで通す。
//   - 「全アクセス失効」(handleAuthRevokeAll) が AuthCookieSecret をローテーションすると
//     既存の PIN セッション cookie も一括失効する。

const (
	// pinMinDigits は PIN の最小桁数（数字 6 桁以上）。
	pinMinDigits = 6
	// pinMaxDigits は誤入力・ペースト事故からの保護のための上限。
	pinMaxDigits = 32
	// pinCookieName はリモート PIN 認証セッション cookie 名。
	pinCookieName = "MANY_AI_CLI_pin"
	// pinCookieTTL は PIN 認証セッションの有効期間。
	pinCookieTTL = 12 * time.Hour
	// pinLockThreshold は連続失敗で次のロックに入るまでの回数。
	pinLockThreshold = 5
	// pinGlobalFailCap は全 IP 合計でこの失敗数を超えたら一時的に全 PIN 受付を止める閾値
	//（分散ブルートフォース対策）。
	pinGlobalFailCap = 30
	// pinAttemptTTL は失敗カウント・既知デバイスの保持時間。
	pinAttemptTTL = time.Hour
)

// pinLockDurations はロックレベルごとのロック時間（pinLockThreshold 回失敗するたびに段階的に伸びる）。
var pinLockDurations = []time.Duration{1 * time.Minute, 5 * time.Minute, 30 * time.Minute}

// isValidPINFormat は PIN が数字のみで pinMinDigits..pinMaxDigits 桁か検証する。
func isValidPINFormat(pin string) bool {
	if len(pin) < pinMinDigits || len(pin) > pinMaxDigits {
		return false
	}
	for _, r := range pin {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

// hashPIN は PIN を bcrypt（salt 込み・単一文字列）でハッシュ化する。
func hashPIN(plain string) (string, error) {
	b, err := bcrypt.GenerateFromPassword([]byte(plain), bcrypt.DefaultCost)
	if err != nil {
		return "", fmt.Errorf("hash pin: %w", err)
	}
	return string(b), nil
}

// verifyPIN は plain が hash に一致するか検証する（bcrypt 内部で定数時間比較）。
func verifyPIN(hash, plain string) bool {
	if hash == "" || plain == "" {
		return false
	}
	return bcrypt.CompareHashAndPassword([]byte(hash), []byte(plain)) == nil
}

// signPINCookie は expiry（unix 秒）を secret で HMAC-SHA256 署名した cookie 値を返す。
func signPINCookie(secret string, expiry int64) string {
	payload := strconv.FormatInt(expiry, 10)
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(payload))
	return payload + "." + hex.EncodeToString(mac.Sum(nil))
}

// verifyPINCookie は cookie 値の署名と有効期限を検証する。
func verifyPINCookie(secret, value string, now time.Time) bool {
	if secret == "" || value == "" {
		return false
	}
	dot := strings.LastIndex(value, ".")
	if dot <= 0 || dot == len(value)-1 {
		return false
	}
	payload, sig := value[:dot], value[dot+1:]
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(payload))
	want := hex.EncodeToString(mac.Sum(nil))
	if subtle.ConstantTimeCompare([]byte(sig), []byte(want)) != 1 {
		return false
	}
	expiry, err := strconv.ParseInt(payload, 10, 64)
	if err != nil {
		return false
	}
	return now.Unix() < expiry
}

// --- ロックアウト（レート制限） ---

type pinAttempt struct {
	fails       int
	lockLevel   int
	lockedUntil time.Time
	lastSeen    time.Time
}

type pinLimiter struct {
	mu          sync.Mutex
	perIP       map[string]*pinAttempt
	globalFails int
	globalUntil time.Time
	globalSeen  time.Time
}

func newPINLimiter() *pinLimiter {
	return &pinLimiter{perIP: map[string]*pinAttempt{}}
}

// retryAfter は >0 ならロック中で、その残り秒数を返す。0 なら試行可。
func (l *pinLimiter) retryAfter(ip string, now time.Time) int {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.pruneLocked(now)
	if now.Before(l.globalUntil) {
		return int(l.globalUntil.Sub(now).Seconds()) + 1
	}
	if a := l.perIP[ip]; a != nil && now.Before(a.lockedUntil) {
		return int(a.lockedUntil.Sub(now).Seconds()) + 1
	}
	return 0
}

func (l *pinLimiter) recordFailure(ip string, now time.Time) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.pruneLocked(now)
	a := l.perIP[ip]
	if a == nil {
		a = &pinAttempt{}
		l.perIP[ip] = a
	}
	a.lastSeen = now
	a.fails++
	if a.fails >= pinLockThreshold {
		a.lockedUntil = now.Add(pinLockDurations[min(a.lockLevel, len(pinLockDurations)-1)])
		if a.lockLevel < len(pinLockDurations)-1 {
			a.lockLevel++
		}
		a.fails = 0
	}
	// 全体カウンタ（分散ブルートフォース対策）。pinAttemptTTL 経過でリセット。
	if now.Sub(l.globalSeen) > pinAttemptTTL {
		l.globalFails = 0
	}
	l.globalSeen = now
	l.globalFails++
	if l.globalFails >= pinGlobalFailCap {
		l.globalUntil = now.Add(pinLockDurations[0])
		l.globalFails = 0
	}
}

func (l *pinLimiter) recordSuccess(ip string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	delete(l.perIP, ip)
}

func (l *pinLimiter) pruneLocked(now time.Time) {
	for k, a := range l.perIP {
		if now.Sub(a.lastSeen) > pinAttemptTTL && now.After(a.lockedUntil) {
			delete(l.perIP, k)
		}
	}
}

// pinLim は Server の pinLimiter を遅延生成して返す。
func (s *Server) pinLim() *pinLimiter {
	s.pinLimiterMu.Lock()
	defer s.pinLimiterMu.Unlock()
	if s.pinLimiter == nil {
		s.pinLimiter = newPINLimiter()
	}
	return s.pinLimiter
}

func clientIPKey(remoteAddr string) string {
	if ip := remoteAddrIP(remoteAddr); ip != nil {
		return ip.String()
	}
	return strings.TrimSpace(remoteAddr)
}

// --- ゲート ---

// hasValidPINCookie はリクエストが有効な PIN セッション cookie を提示しているか返す。
func (s *Server) hasValidPINCookie(r *http.Request) bool {
	if r == nil {
		return false
	}
	c, err := r.Cookie(pinCookieName)
	if err != nil {
		return false
	}
	s.cfgMu.Lock()
	secret := s.cfg.AuthCookieSecret
	s.cfgMu.Unlock()
	return verifyPINCookie(secret, c.Value, time.Now())
}

// remotePINRequired は「論理的に remote かつ PIN 設定済み」を返す。
// 非 loopback の直アクセスだけでなく、tailscale serve 等のリバースプロキシ経由
// （TCP 元は loopback だが Host が tailnet DNS 名）も remote として扱う
// （isLogicallyRemote）。これにより公開経路で PIN ゲートが素通しになる事故を防ぐ。
func (s *Server) remotePINRequired(r *http.Request) bool {
	if !s.isLogicallyRemote(r) {
		return false
	}
	s.cfgMu.Lock()
	enabled := strings.TrimSpace(s.cfg.RemotePINHash) != ""
	s.cfgMu.Unlock()
	return enabled
}

// requireRemotePIN は guard に組み込む追加ゲート。remote かつ PIN 設定済みのとき
// 有効な PIN セッション cookie を要求する。loopback / PIN 無効時は素通し。
func (s *Server) requireRemotePIN(w http.ResponseWriter, r *http.Request) bool {
	if !s.remotePINRequired(r) {
		return true
	}
	if s.hasValidPINCookie(r) {
		return true
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSONError(w, http.StatusUnauthorized, "pin_required", "remote pin required")
	return false
}

// --- ハンドラ ---

// handleAuthStatus は PIN の有効/未認証/ロックアウト状態を返す（フロントの PIN モーダル制御用）。
// PIN ゲートは通さない（未認証でも状態取得できる必要があるため guardBase を使う）。
func (s *Server) handleAuthStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guardBase(w, r, http.MethodGet) {
		return
	}
	s.cfgMu.Lock()
	pinSet := strings.TrimSpace(s.cfg.RemotePINHash) != ""
	s.cfgMu.Unlock()
	remote := s.isLogicallyRemote(r)
	authed := !pinSet || !remote || s.hasValidPINCookie(r)
	retry := 0
	if pinSet && remote {
		retry = s.pinLim().retryAfter(clientIPKey(r.RemoteAddr), time.Now())
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"pin_enabled": pinSet,
		"remote":      remote,
		"authed":      authed,
		"locked":      retry > 0,
		"retry_after": retry,
	})
}

// handleAuthLogin はリモート PIN ログイン。token は guardBase で検証済み、ここで PIN を検証して
// 成功時に PIN セッション cookie を発行する。PIN ゲートは通さない（ログイン前に通せないため）。
func (s *Server) handleAuthLogin(w http.ResponseWriter, r *http.Request) {
	if !s.guardBase(w, r, http.MethodPost) {
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	s.cfgMu.Lock()
	pinHash := s.cfg.RemotePINHash
	secret := s.cfg.AuthCookieSecret
	s.cfgMu.Unlock()
	if strings.TrimSpace(pinHash) == "" {
		writeJSON(w, map[string]any{"ok": true, "pin_enabled": false})
		return
	}
	if secret == "" {
		// PIN 設定時に必ず secret を生成するため通常起こり得ないが、防御的に弾く。
		writeJSONError(w, http.StatusInternalServerError, "internal", "pin not configured")
		return
	}
	ip := clientIPKey(r.RemoteAddr)
	now := time.Now()
	if retry := s.pinLim().retryAfter(ip, now); retry > 0 {
		w.Header().Set("Retry-After", strconv.Itoa(retry))
		writeJSONError(w, http.StatusTooManyRequests, "locked_out", "too many attempts")
		return
	}
	var body struct {
		PIN string `json:"pin"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if !verifyPIN(pinHash, body.PIN) {
		s.pinLim().recordFailure(ip, now)
		retry := s.pinLim().retryAfter(ip, now)
		// 失敗ログは件数/ロックアウト状態のみ。PIN 値・入力は決して残さない。
		if s.logger != nil {
			s.logger.Warn("remote pin login failed", "retry_after", retry)
		}
		if retry > 0 {
			w.Header().Set("Retry-After", strconv.Itoa(retry))
			writeJSONError(w, http.StatusTooManyRequests, "locked_out", "too many attempts")
			return
		}
		writeJSONError(w, http.StatusUnauthorized, "bad_pin", "incorrect pin")
		return
	}
	s.pinLim().recordSuccess(ip)
	expiry := now.Add(pinCookieTTL).Unix()
	http.SetCookie(w, &http.Cookie{
		Name:     pinCookieName,
		Value:    signPINCookie(secret, expiry),
		Path:     "/",
		HttpOnly: true,
		SameSite: http.SameSiteStrictMode,
		MaxAge:   int(pinCookieTTL / time.Second),
	})
	s.noteRemoteDevice(r, "pin_login")
	writeJSON(w, map[string]any{"ok": true})
}

// handleAuthSetPIN は PIN の設定/変更/解除。full guard（token + host + origin + PIN ゲート）を通す。
// よって remote からの変更は既存 PIN で認証済みのときだけ可能。loopback は常に可。
//
// 追加ゲート: PIN 未設定時の bootstrap で remotePINRequired() が false を返すため、
// guard() 単体ではリモート token 保持者が初回 PIN を奪える（所有者を締め出せる）。
// loopback でないリモートからの POST は、既存 PIN cookie で本人確認できる場合のみ
// 通す（PIN 解除を含む）。
func (s *Server) handleAuthSetPIN(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.isLogicallyRemote(r) && !s.hasValidPINCookie(r) {
		w.Header().Set("Cache-Control", "no-store")
		writeJSONError(w, http.StatusForbidden, "forbidden", "set-pin from a remote address requires existing PIN authentication or a local (loopback) session")
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	var body struct {
		PIN   string `json:"pin"`
		Clear bool   `json:"clear"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	// 解除
	if body.Clear || strings.TrimSpace(body.PIN) == "" {
		s.cfgMu.Lock()
		s.cfg.RemotePINHash = ""
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "internal", "failed to persist config")
			return
		}
		writeJSON(w, map[string]any{"ok": true, "pin_enabled": false})
		return
	}
	if !isValidPINFormat(body.PIN) {
		writeJSONError(w, http.StatusBadRequest, "bad_pin_format", "pin must be 6 or more digits")
		return
	}
	hash, err := hashPIN(body.PIN)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to hash pin")
		return
	}
	// PIN を有効化するなら HMAC secret も必須。無ければここで生成しておく
	//（cookie 署名・revoke 連動の前提）。
	candidate, err := randomHex(32)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to init secret")
		return
	}
	s.cfgMu.Lock()
	s.cfg.RemotePINHash = hash
	if s.cfg.AuthCookieSecret == "" {
		s.cfg.AuthCookieSecret = candidate
	}
	effSecret := s.cfg.AuthCookieSecret
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to persist config")
		return
	}
	// 設定/変更した本人（このリクエスト元）が直後に弾かれないよう PIN cookie を発行する。
	expiry := time.Now().Add(pinCookieTTL).Unix()
	http.SetCookie(w, &http.Cookie{
		Name:     pinCookieName,
		Value:    signPINCookie(effSecret, expiry),
		Path:     "/",
		HttpOnly: true,
		SameSite: http.SameSiteStrictMode,
		MaxAge:   int(pinCookieTTL / time.Second),
	})
	writeJSON(w, map[string]any{"ok": true, "pin_enabled": true})
}

// --- SEC-C: 新規デバイスのリモート接続通知 ---

const (
	// maxKnownDevices は既知デバイス map のソフト上限（TTL prune と併用）。
	maxKnownDevices = 256
	// knownDeviceTTL を過ぎた既知デバイスは忘れ、再接続で再通知する。
	knownDeviceTTL = 24 * time.Hour
)

// deviceKey は IP + UA から不可逆なデバイス識別子（先頭 16 hex）を作る。
func deviceKey(remoteAddr, ua string) string {
	ipStr := ""
	if ip := remoteAddrIP(remoteAddr); ip != nil {
		ipStr = ip.String()
	}
	sum := sha256.Sum256([]byte(ipStr + "\x00" + ua))
	return hex.EncodeToString(sum[:])[:16]
}

// noteRemoteDevice は remote（非 loopback）からの認証済みアクセスを記録し、
// 未知デバイスの初回接続時に SEC-C 通知（push / ntfy / webhook）を本人へ送る。
// loopback は無視。盗まれた QR / token が使われたら即気づけるようにするのが狙い。
func (s *Server) noteRemoteDevice(r *http.Request, via string) {
	// loopback 直アクセス（ローカル PC のブラウザ・wrapper/CLI）は通知対象外。
	// tailscale serve 等のプロキシ経由（loopback 元だが Host が tailnet 名）は
	// isLogicallyRemote が true を返し、未知デバイス通知の対象になる。
	if !s.isLogicallyRemote(r) {
		return
	}
	ua := strings.TrimSpace(r.UserAgent())
	key := deviceKey(r.RemoteAddr, ua)
	now := time.Now()

	s.devicesMu.Lock()
	if s.knownDevices == nil {
		s.knownDevices = map[string]time.Time{}
	}
	for k, ts := range s.knownDevices {
		if now.Sub(ts) > knownDeviceTTL {
			delete(s.knownDevices, k)
		}
	}
	_, known := s.knownDevices[key]
	// ソフト上限超過時は最古を 1 件落としてから追加（無制限増加の防止）。
	if !known && len(s.knownDevices) >= maxKnownDevices {
		var oldestK string
		var oldestT time.Time
		for k, ts := range s.knownDevices {
			if oldestK == "" || ts.Before(oldestT) {
				oldestK, oldestT = k, ts
			}
		}
		if oldestK != "" {
			delete(s.knownDevices, oldestK)
		}
	}
	s.knownDevices[key] = now
	s.devicesMu.Unlock()

	if known {
		return
	}
	ipLabel := "unknown"
	if ip := remoteAddrIP(r.RemoteAddr); ip != nil {
		ipLabel = ip.String()
	}
	uaShort := ua
	if len(uaShort) > 80 {
		uaShort = strings.TrimSpace(uaShort[:80])
	}
	title := "many-ai-cli: new device connected"
	body := fmt.Sprintf("New remote connection from %s (%s) via %s", ipLabel, uaShort, via)
	s.notifyNewDevice(title, body)
}

// notifyNewDevice は SEC-C 通知を push / ntfy / webhook の両系統へ送る（設定があれば）。
// セキュリティ警告のため notify のイベントフィルタは無視して送る。
func (s *Server) notifyNewDevice(title, body string) {
	if s.notifyMgr != nil {
		s.notifyMgr.SendSecurity(title, body)
	}
	if s.push != nil {
		s.safeGo("web push security", func() {
			ctx, cancel := context.WithTimeout(context.Background(), pushSendTimeout)
			defer cancel()
			s.push.sendSecurity(ctx, title, body)
		})
	}
}
