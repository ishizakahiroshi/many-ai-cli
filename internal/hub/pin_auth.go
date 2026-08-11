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
	// maxPINSessions は nonce レジストリの上限。成功ログインの連打で
	// cookie TTL 中のエントリが無制限に増えないようにする。
	maxPINSessions = 256
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

// signPINCookie は expiry.nonce.deviceHash を HMAC-SHA256 署名する。
// payloadParts を省略した expiry-only 形式は既存 unit test と旧 cookie の検証互換専用。
func signPINCookie(secret string, expiry int64, payloadParts ...string) string {
	payload := strconv.FormatInt(expiry, 10)
	if len(payloadParts) == 2 {
		payload += "." + payloadParts[0] + "." + payloadParts[1]
	}
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(payload))
	return payload + "." + hex.EncodeToString(mac.Sum(nil))
}

// verifyPINCookie は cookie 値の署名と有効期限を検証する。
func verifyPINCookie(secret, value string, now time.Time, expectedDeviceHash ...string) bool {
	expiry, _, deviceHash, ok := parsePINCookie(secret, value)
	if !ok || now.Unix() >= expiry {
		return false
	}
	return len(expectedDeviceHash) == 0 || (deviceHash != "" && subtle.ConstantTimeCompare([]byte(deviceHash), []byte(expectedDeviceHash[0])) == 1)
}

func parsePINCookie(secret, value string) (int64, string, string, bool) {
	if secret == "" || value == "" {
		return 0, "", "", false
	}
	dot := strings.LastIndex(value, ".")
	if dot <= 0 || dot == len(value)-1 {
		return 0, "", "", false
	}
	payload, sig := value[:dot], value[dot+1:]
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(payload))
	want := hex.EncodeToString(mac.Sum(nil))
	if subtle.ConstantTimeCompare([]byte(sig), []byte(want)) != 1 {
		return 0, "", "", false
	}
	parts := strings.Split(payload, ".")
	expiry, err := strconv.ParseInt(parts[0], 10, 64)
	if err != nil {
		return 0, "", "", false
	}
	if len(parts) == 1 {
		return expiry, "", "", true
	}
	if len(parts) != 3 || parts[1] == "" || parts[2] == "" {
		return 0, "", "", false
	}
	return expiry, parts[1], parts[2], true
}

type pinCookieSession struct {
	expiry     int64
	deviceHash string
	issuedAt   int64
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
// 状態を変えない読み取り専用。handleAuthStatus のような UI 表示照会用。
func (l *pinLimiter) retryAfter(ip string, now time.Time) int {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.pruneLocked(now)
	if now.Before(l.globalUntil) {
		return int(l.globalUntil.Sub(now).Seconds()) + 1
	}
	if ip != "" {
		if a := l.perIP[ip]; a != nil && now.Before(a.lockedUntil) {
			return int(a.lockedUntil.Sub(now).Seconds()) + 1
		}
	}
	return 0
}

// beginAttempt はロック中でなければ「これから 1 回試行するつもり」として
// 失敗カウンタを先に進めてロック閾値に達したらロックを開始し、retry>0 を返す。
// 返値 0 なら呼び出し元は verify に進み、成功なら recordSuccess を呼ぶ
// （その時点で予約分の失敗カウンタは delete で丸ごと消える）。
// 従来は retryAfter → verify（bcrypt cost 10 = 数十 ms） → recordFailure と
// 段階が別々のロック区間になっていたため、並行度 N の wrong-PIN バーストで
// verify 中の窓に別リクエストが retryAfter=0 を通過してしまい、実効閾値が
// 並行度倍まで緩んでいた（HUB-13）。beginAttempt は「予約時点で++」する
// ことでロック判定を先取りし、bcrypt 進行中に来た次リクエストは既にロック
// 状態と一致するように直す。
func (l *pinLimiter) beginAttempt(ip string, now time.Time) int {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.pruneLocked(now)
	if now.Before(l.globalUntil) {
		return int(l.globalUntil.Sub(now).Seconds()) + 1
	}
	if ip != "" {
		if a := l.perIP[ip]; a != nil && now.Before(a.lockedUntil) {
			return int(a.lockedUntil.Sub(now).Seconds()) + 1
		}
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
	return 0
}

func (l *pinLimiter) recordSuccess(ip string) {
	if ip == "" {
		return
	}
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

// pinRateLimitKey avoids collapsing every Tailscale Serve client into the
// proxy's 127.0.0.1 bucket. A Tailscale identity header is trusted only when
// the TCP peer is loopback and the request is logically remote; otherwise the
// per-client bucket is skipped and the global failure cap remains in force.
func (s *Server) pinRateLimitKey(r *http.Request) string {
	if r == nil {
		return ""
	}
	if isLoopbackRemote(r.RemoteAddr) && s.isLogicallyRemote(r) {
		login := strings.ToLower(strings.TrimSpace(r.Header.Get("Tailscale-User-Login")))
		if login == "" {
			return ""
		}
		sum := sha256.Sum256([]byte("tailscale:" + login))
		return "ts:" + hex.EncodeToString(sum[:8])
	}
	return clientIPKey(r.RemoteAddr)
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
	deviceHash := requestDeviceHash(r)
	if !verifyPINCookie(secret, c.Value, time.Now(), deviceHash) {
		return false
	}
	expiry, nonce, signedDeviceHash, ok := parsePINCookie(secret, c.Value)
	if !ok || nonce == "" {
		return false
	}
	s.pinSessionsMu.Lock()
	defer s.pinSessionsMu.Unlock()
	now := time.Now().Unix()
	s.pruneExpiredPINSessionsLocked(now)
	session, ok := s.pinSessions[nonce]
	return ok && session.expiry == expiry && session.deviceHash == signedDeviceHash
}

func (s *Server) issuePINCookie(secret string, r *http.Request, expiry int64) (string, error) {
	nonce, err := randomHex(16)
	if err != nil {
		return "", err
	}
	deviceHash := requestDeviceHash(r)
	now := time.Now()
	s.pinSessionsMu.Lock()
	if s.pinSessions == nil {
		s.pinSessions = map[string]pinCookieSession{}
	}
	s.pruneExpiredPINSessionsLocked(now.Unix())
	s.evictPINSessionsForInsertLocked()
	s.pinSessions[nonce] = pinCookieSession{expiry: expiry, deviceHash: deviceHash, issuedAt: now.UnixNano()}
	s.pinSessionsMu.Unlock()
	return signPINCookie(secret, expiry, nonce, deviceHash), nil
}

func (s *Server) pruneExpiredPINSessionsLocked(now int64) {
	for nonce, session := range s.pinSessions {
		if session.expiry <= now {
			delete(s.pinSessions, nonce)
		}
	}
}

// evictPINSessionsForInsertLocked reserves one slot for a new PIN session.
// The caller holds pinSessionsMu.
func (s *Server) evictPINSessionsForInsertLocked() {
	for len(s.pinSessions) >= maxPINSessions {
		oldestNonce := ""
		var oldestIssuedAt int64
		for nonce, session := range s.pinSessions {
			if oldestNonce == "" || session.issuedAt < oldestIssuedAt {
				oldestNonce = nonce
				oldestIssuedAt = session.issuedAt
			}
		}
		if oldestNonce == "" {
			return
		}
		delete(s.pinSessions, oldestNonce)
	}
}

func (s *Server) revokePINCookie(value, secret string) {
	_, nonce, _, ok := parsePINCookie(secret, value)
	if !ok || nonce == "" {
		return
	}
	s.pinSessionsMu.Lock()
	delete(s.pinSessions, nonce)
	s.pinSessionsMu.Unlock()
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
		retry = s.pinLim().retryAfter(s.pinRateLimitKey(r), time.Now())
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
	ip := s.pinRateLimitKey(r)
	now := time.Now()
	var body struct {
		PIN string `json:"pin"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	// 失敗を先に予約する（HUB-13）。ロック中なら verify に入らず即弾く。
	// 予約成功後は verifyPIN の結果に応じて recordSuccess で丸ごとクリアする。
	if retry := s.pinLim().beginAttempt(ip, now); retry > 0 {
		w.Header().Set("Retry-After", strconv.Itoa(retry))
		writeJSONError(w, http.StatusTooManyRequests, "locked_out", "too many attempts")
		return
	}
	// PIN フォーマット違反（空文字・非数字・72B 超）は bcrypt へ到達させない。
	// beginAttempt で既に失敗計上済みなので、追加カウントは行わず「bad_pin」で返す。
	// handleAuthSetPIN 側と対称にする。
	if !isValidPINFormat(body.PIN) {
		if s.logger != nil {
			s.logger.Warn("remote pin login rejected", "reason", "invalid_format")
		}
		writeJSONError(w, http.StatusUnauthorized, "bad_pin", "incorrect pin")
		return
	}
	if !verifyPIN(pinHash, body.PIN) {
		// beginAttempt で既に失敗計上済み。ここでは追加でカウントしない。
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
	cookieValue, err := s.issuePINCookie(secret, r, expiry)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to create pin session")
		return
	}
	http.SetCookie(w, &http.Cookie{
		Name:     pinCookieName,
		Value:    cookieValue,
		Path:     "/",
		HttpOnly: true,
		Secure:   requestUsesHTTPS(r),
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
	cookieValue, err := s.issuePINCookie(effSecret, r, expiry)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to create pin session")
		return
	}
	http.SetCookie(w, &http.Cookie{
		Name:     pinCookieName,
		Value:    cookieValue,
		Path:     "/",
		HttpOnly: true,
		Secure:   requestUsesHTTPS(r),
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

// deviceKey は IP + browser brand/OS の粗い bucket から不可逆な識別子を作る。
func deviceKey(remoteAddr, ua string) string {
	ipStr := strings.TrimSpace(remoteAddr)
	if ip := remoteAddrIP(remoteAddr); ip != nil {
		ipStr = ip.String()
	}
	sum := sha256.Sum256([]byte(ipStr + "\x00" + userAgentBucket(ua)))
	return hex.EncodeToString(sum[:])[:16]
}

func requestDeviceHash(r *http.Request) string {
	if r == nil {
		return deviceKey("", "")
	}
	identity := r.RemoteAddr
	if isLoopbackRemote(r.RemoteAddr) {
		login := strings.ToLower(strings.TrimSpace(r.Header.Get("Tailscale-User-Login")))
		if login != "" {
			identity = "tailscale:" + login
		}
	}
	return deviceKey(identity, r.UserAgent())
}

func userAgentBucket(ua string) string {
	lower := strings.ToLower(ua)
	brand := "other"
	for _, candidate := range []string{"edg/", "chrome/", "firefox/", "safari/", "curl/", "many-ai-cli"} {
		if strings.Contains(lower, candidate) {
			brand = strings.TrimSuffix(candidate, "/")
			break
		}
	}
	osName := "other"
	for _, candidate := range []string{"windows", "android", "iphone", "ipad", "mac os", "linux"} {
		if strings.Contains(lower, candidate) {
			osName = candidate
			break
		}
	}
	return brand + "/" + osName
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
	key := requestDeviceHash(r)
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
