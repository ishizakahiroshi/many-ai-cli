package hub

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/proxy"
)

// chatProxyBaseURLValue は EnvPresetForProxy へ渡す base URL を atomic に保持する。
// proxy 起動成功時にセットされ、spawn 時に env 注入に使われる。失敗時は空文字のままで
// 既存の挙動（payload 取得 OFF）にフォールバックする。
var chatProxyBaseURLValue atomic.Value // string

// chatHistoryRingSize: 1 セッションあたりの payload 履歴の RAM 保持上限。
// Hub プロセス終了で揮発。永続化は将来 opt-in（plan の C2 で SQLite 化）。
const chatHistoryRingSize = 50

// chat_turn ID は session 内 monotonic にしたいが、proxy 経由は session 跨ぎで
// 単一 sink から流れるので、Hub 全体で monotonic にしてしまう。UI 側は ID で
// dedupe / 並び替えできれば十分。
var chatTurnIDCounter int64

// chatProxyState は session 紐付け用の pending token と、トークン→session の map を保持する。
type chatProxyState struct {
	mu              sync.Mutex
	pendingTokens   map[string]time.Time // spawn 直後・register 前の token
	tokenToSession  map[string]int       // register 確定後の token → session ID
	sessionToTokens map[int][]string     // session 終了時の掃除用
}

var chatProxy = &chatProxyState{
	pendingTokens:   map[string]time.Time{},
	tokenToSession:  map[string]int{},
	sessionToTokens: map[int][]string{},
}

// newProxyToken は spawn 時に払い出すランダムトークン。URL safe な hex 16 文字。
func newProxyToken() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("t%d", time.Now().UnixNano())
	}
	return hex.EncodeToString(b)
}

// registerPendingProxyToken は spawn 時に発行した token を「register 待ち」状態で記録する。
// 一定時間 (5 分) で自動 GC して memory leak を防ぐ。
func (s *Server) registerPendingProxyToken(token string) {
	if token == "" {
		return
	}
	chatProxy.mu.Lock()
	defer chatProxy.mu.Unlock()
	chatProxy.pendingTokens[token] = time.Now()
	// GC: 5 分以上前の pending を掃除（register まで届かなかった spawn）
	cutoff := time.Now().Add(-5 * time.Minute)
	for tok, t := range chatProxy.pendingTokens {
		if t.Before(cutoff) {
			delete(chatProxy.pendingTokens, tok)
		}
	}
}

// linkProxyTokenToSession は wrapper register で受け取った proxy token を session ID と紐付ける。
// pending に無い token は無視（古い spawn / 攻撃者）。
func (s *Server) linkProxyTokenToSession(token string, sessionID int) {
	if token == "" || sessionID <= 0 {
		return
	}
	chatProxy.mu.Lock()
	defer chatProxy.mu.Unlock()
	if _, ok := chatProxy.pendingTokens[token]; !ok {
		// pending に無い: 期限切れ or 不正
		return
	}
	delete(chatProxy.pendingTokens, token)
	chatProxy.tokenToSession[token] = sessionID
	chatProxy.sessionToTokens[sessionID] = append(chatProxy.sessionToTokens[sessionID], token)
}

// unlinkProxyTokensForSession は session 終了時に紐付け map から該当 token を全部消す。
func (s *Server) unlinkProxyTokensForSession(sessionID int) {
	chatProxy.mu.Lock()
	defer chatProxy.mu.Unlock()
	tokens := chatProxy.sessionToTokens[sessionID]
	for _, tok := range tokens {
		delete(chatProxy.tokenToSession, tok)
	}
	delete(chatProxy.sessionToTokens, sessionID)
}

// resolveProxyTokenToSession は token から session ID を返す。
func resolveProxyTokenToSession(token string) (int, bool) {
	if token == "" {
		return 0, false
	}
	chatProxy.mu.Lock()
	defer chatProxy.mu.Unlock()
	id, ok := chatProxy.tokenToSession[token]
	return id, ok
}

// chatProxyBaseURL は現在の内蔵プロキシ base URL を返す。未起動時は空文字。
func (s *Server) chatProxyBaseURL() string {
	if v, ok := chatProxyBaseURLValue.Load().(string); ok {
		return v
	}
	return ""
}

// startChatProxy は内蔵プロキシを 127.0.0.1 の空きポートで起動する。
// 失敗しても Hub 起動は継続する（payload 取得 OFF / PTY スクレイプにフォールバック）。
func (s *Server) startChatProxy() {
	p, err := proxy.New(0, proxy.SinkFunc(s.onChatProxyTurn), s.logger)
	if err != nil {
		s.logger.Warn("chat proxy disabled", "err", err)
		return
	}
	s.chatProxy = p
	baseURL := fmt.Sprintf("http://127.0.0.1:%d", p.Port())
	chatProxyBaseURLValue.Store(baseURL)
	s.logger.Info("chat proxy started", "url", baseURL)
	s.safeGo("chat_proxy_serve", func() {
		if err := p.Serve(); err != nil {
			s.logger.Warn("chat proxy serve ended with error", "err", err)
		}
	})
}

// stopChatProxy は内蔵プロキシを graceful shutdown する。
func (s *Server) stopChatProxy() {
	if s.chatProxy == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	_ = s.chatProxy.Shutdown(ctx)
	chatProxyBaseURLValue.Store("")
}

// onChatProxyTurn は内蔵プロキシが捕捉した 1 リクエスト/レスポンスの受け口。
// SessionToken → session ID を解決して session ring buffer に push、UI に broadcast する。
func (s *Server) onChatProxyTurn(turn proxy.CapturedTurn) {
	sessionID, _ := resolveProxyTokenToSession(turn.SessionToken)
	chatTurn := buildChatTurn(turn)
	if sessionID > 0 {
		s.pushChatTurnToSession(sessionID, chatTurn)
		s.broadcast(proto.Message{
			Type:      "chat_turn",
			SessionID: sessionID,
			ChatTurn:  chatTurn,
		})
	} else {
		// 未紐付け: log のみ（register 前のレース / 古い token）
		s.logger.Debug("chat proxy turn without session",
			"provider", turn.Provider, "endpoint", turn.Endpoint,
			"token_present", turn.SessionToken != "")
	}
}

// buildChatTurn は proxy.CapturedTurn を proto.ChatTurn（マスキング済み）に変換する。
func buildChatTurn(turn proxy.CapturedTurn) *proto.ChatTurn {
	ct := &proto.ChatTurn{
		ID:           atomic.AddInt64(&chatTurnIDCounter, 1),
		Provider:     string(turn.Provider),
		Endpoint:     turn.Endpoint,
		ReceivedAt:   turn.ReceivedAt.Format(time.RFC3339Nano),
		DurationMS:   turn.DurationMS,
		StatusCode:   turn.StatusCode,
		IsStream:     turn.IsStream,
		Truncated:    turn.Truncated,
		RequestJSON:  string(proxy.MaskSecrets(turn.RequestBody)),
		ResponseJSON: string(proxy.MaskSecrets(turn.ResponseBody)),
	}
	switch turn.Provider {
	case proxy.ProviderAnthropic:
		r := proxy.SummarizeAnthropicRequest(turn.RequestBody)
		ct.Model = r.Model
		ct.MessageCount = r.MessageCount
		ct.ToolCount = r.ToolCount
		ct.HasSystem = r.HasSystem
	case proxy.ProviderOpenAI:
		r := proxy.SummarizeOpenAIRequest(turn.RequestBody)
		ct.Model = r.Model
		ct.MessageCount = r.MessageCount
		ct.ToolCount = r.ToolCount
	}
	// token usage は response body から抽出（best-effort）
	ct.TokensIn, ct.TokensOut = extractTokenUsage(turn.Provider, turn.ResponseBody)
	if errs := strings.TrimSpace(turn.RequestErr + " " + turn.ResponseErr); errs != "" {
		ct.ErrorText = errs
	}
	return ct
}

// pushChatTurnToSession は session ringbuffer に turn を追加する（lock 取得）。
func (s *Server) pushChatTurnToSession(sessionID int, turn *proto.ChatTurn) {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses, ok := s.sessions[sessionID]
	if !ok {
		return
	}
	ses.chatTurns = append(ses.chatTurns, turn)
	if len(ses.chatTurns) > chatHistoryRingSize {
		ses.chatTurns = ses.chatTurns[len(ses.chatTurns)-chatHistoryRingSize:]
	}
}

// snapshotChatTurns は UI 接続時のスナップショット配信用にコピーを返す。
func (s *Server) snapshotChatTurns(sessionID int) []*proto.ChatTurn {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses, ok := s.sessions[sessionID]
	if !ok || len(ses.chatTurns) == 0 {
		return nil
	}
	out := make([]*proto.ChatTurn, len(ses.chatTurns))
	copy(out, ses.chatTurns)
	return out
}
