package hub

// usage_stat.go — POST /api/session-usage 受信エンドポイント + セッション単位 usage 保持 + 価格表。
//
// セキュリティ要件（必須・C1 完了条件）:
//   - Hub はメモリ保持のみ。ディスクへ書き込まない。
//   - relay から受け取るのは数値メタ（トークン数・コスト・モデル名・経過時間）のみ。
//     プロンプト/コード/ツール入出力などの本文は受け付けない（リクエスト構造体に本文フィールドなし）。

import (
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/proto"
)

// usageStat はセッション単位のトークン / コスト累積値。
// メモリのみ保持し、ディスクへは書き込まない。
type usageStat struct {
	// CostUSD: relay から受信したコスト（Claude は statusLine 値をそのまま採用）。
	// Codex は Hub 側の価格表で算出。
	CostUSD float64
	// CostKnown: 価格表に登録されているモデルか。false なら表示側は "$ —" とする。
	CostKnown   bool
	TokensIn    int
	TokensOut   int
	TokensCache int
	TokensTotal int
	// CtxWindow: モデルのコンテキストウィンドウ上限（relay から受信。不明なら 0）。
	CtxWindow int
	// UsageModel: relay が報告したモデル ID / display_name。
	UsageModel string
	StartedAt  string
	ReceivedAt time.Time
}

// usageStatsMu は usageStats map を保護する。sessionsMu とは独立したロック。
// ロック順序: sessionsMu を保持したまま usageStatsMu を取得しない。
var usageStatsMu sync.Mutex
var usageStats = map[int]*usageStat{} // key: session ID (live)

// ---------------------------------------------------------------------------
// 価格表（per-MTok 単価、USD）
// ---------------------------------------------------------------------------
//
// 注意: UI 文字列の固定分岐にモデル名を使わない（feedback_no_hardcoded_model_names）。
// 価格表は「モデル ID → 単価」のデータとしてのみ持つ。
//
// 最終更新: 2026-06 時点の公開価格を参考値として収録。
// 実際の課金は各プロバイダの契約・割引・バッチ価格等で異なる場合がある。
// 価格改定時はこのマップを更新する。

type modelPricing struct {
	InputPerMTok      float64 // USD per 1M input tokens
	OutputPerMTok     float64 // USD per 1M output tokens
	CacheReadPerMTok  float64 // USD per 1M cache-read tokens (0 = cache なし or 同 input)
	CacheWritePerMTok float64 // USD per 1M cache-write tokens (0 = 計上しない)
}

// modelPriceTable: モデル ID（完全一致または前方一致で検索）→ 単価。
// Codex 用: トークン → コスト算出に使う。
// Claude 用: relay が cost をそのまま送るため原則使わないが、
//            将来的にトークン内訳を表示する場合に備えて収録。
var modelPriceTable = map[string]modelPricing{
	// --- OpenAI / Codex ---
	// gpt-4.1 系 (2025-04 発表)
	"gpt-4.1":      {InputPerMTok: 2.00, OutputPerMTok: 8.00, CacheReadPerMTok: 0.50},
	"gpt-4.1-mini": {InputPerMTok: 0.40, OutputPerMTok: 1.60, CacheReadPerMTok: 0.10},
	"gpt-4.1-nano": {InputPerMTok: 0.10, OutputPerMTok: 0.40, CacheReadPerMTok: 0.025},
	"gpt-4o":       {InputPerMTok: 2.50, OutputPerMTok: 10.00, CacheReadPerMTok: 1.25},
	"gpt-4o-mini":  {InputPerMTok: 0.15, OutputPerMTok: 0.60, CacheReadPerMTok: 0.075},
	"gpt-5":        {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"gpt-5.5":      {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"o3":           {InputPerMTok: 10.00, OutputPerMTok: 40.00, CacheReadPerMTok: 2.50},
	"o4-mini":      {InputPerMTok: 1.10, OutputPerMTok: 4.40, CacheReadPerMTok: 0.275},
	// --- Anthropic / Claude ---
	// Claude 4 系 (2026 Q1)
	"claude-opus-4":   {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-sonnet-4": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-haiku-4":  {InputPerMTok: 0.80, OutputPerMTok: 4.00, CacheReadPerMTok: 0.08, CacheWritePerMTok: 1.00},
	// 現行モデル（2026-06 公式単価。CacheRead=入力×0.1 / CacheWrite=入力×1.25 の慣習で算出）
	// audit #31: 現行モデル ID を正規単価で表に収録（claude-api スキル公式テーブル 2026-06-04 確認）。
	"claude-fable-5":    {InputPerMTok: 10.00, OutputPerMTok: 50.00, CacheReadPerMTok: 1.00, CacheWritePerMTok: 12.50},
	"claude-opus-4-8":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-opus-4-7":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-opus-4-6":   {InputPerMTok: 5.00, OutputPerMTok: 25.00, CacheReadPerMTok: 0.50, CacheWritePerMTok: 6.25},
	"claude-sonnet-4-6": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	// claude-opus-4-5 / claude-opus-4（4.0）: 公式 pricing 表に現行単価の記載なし（legacy/deprecated）。
	// 確証なしのため $15/$75 据え置き（C6 判断ログ参照）。
	"claude-opus-4-5":   {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-sonnet-4-5": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-haiku-4-5":  {InputPerMTok: 1.00, OutputPerMTok: 5.00, CacheReadPerMTok: 0.10, CacheWritePerMTok: 1.25},
	"claude-3-5-sonnet": {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-3-5-haiku":  {InputPerMTok: 0.80, OutputPerMTok: 4.00, CacheReadPerMTok: 0.08, CacheWritePerMTok: 1.00},
	"claude-3-opus":     {InputPerMTok: 15.00, OutputPerMTok: 75.00, CacheReadPerMTok: 1.50, CacheWritePerMTok: 18.75},
	"claude-3-sonnet":   {InputPerMTok: 3.00, OutputPerMTok: 15.00, CacheReadPerMTok: 0.30, CacheWritePerMTok: 3.75},
	"claude-3-haiku":    {InputPerMTok: 0.25, OutputPerMTok: 1.25, CacheReadPerMTok: 0.03, CacheWritePerMTok: 0.30},
}

// lookupModelPricing はモデル ID（完全一致 → 前方一致の順）で価格表を引く。
// ヒットしない場合は (modelPricing{}, false) を返す。
// Codex は "gpt-4.1 medium" のように effort サフィックスが付く場合があるため
// スペース以前のプレフィックスでも検索する。
func lookupModelPricing(modelID string) (modelPricing, bool) {
	if p, ok := modelPriceTable[modelID]; ok {
		return p, true
	}
	// スペースで区切った最初のトークンで再試行（effort サフィックス除去）
	for i, c := range modelID {
		if c == ' ' {
			if p, ok := modelPriceTable[modelID[:i]]; ok {
				return p, true
			}
			break
		}
	}
	return modelPricing{}, false
}

// calcCostUSD はトークン数と価格表からコスト（USD）を算出する。
// 価格表にモデルが無い場合は (0, false) を返す。
func calcCostUSD(modelID string, tokIn, tokOut, tokCacheRead int) (cost float64, known bool) {
	p, ok := lookupModelPricing(modelID)
	if !ok {
		return 0, false
	}
	const mTok = 1_000_000.0
	cost = float64(tokIn)*p.InputPerMTok/mTok +
		float64(tokOut)*p.OutputPerMTok/mTok +
		float64(tokCacheRead)*p.CacheReadPerMTok/mTok
	return cost, true
}

func (s *Server) sessionUsageModel(sessionID int) string {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses := s.sessions[sessionID]; ses != nil {
		return strings.TrimSpace(ses.Model)
	}
	return ""
}

// ---------------------------------------------------------------------------
// HTTP ハンドラ
// ---------------------------------------------------------------------------

// sessionUsageRequest は relay が POST する数値メタのみを受け取る構造体。
// プロンプト本文・コード・ツール入出力などのフィールドは存在しない（セキュリティ要件）。
type sessionUsageRequest struct {
	Provider  string  `json:"provider"`
	SessionID int     `json:"session_id"`
	CostUSD   float64 `json:"cost_usd"`
	// CostFromRelay: relay 側（Claude）が計算済みコストを送る場合は true。
	// false（Codex 等）の場合は Hub 側の価格表で算出する。
	CostFromRelay bool   `json:"cost_from_relay"`
	Model         string `json:"model"`
	TokensIn      int    `json:"tokens_in"`
	TokensOut     int    `json:"tokens_out"`
	TokensCache   int    `json:"tokens_cache"`
	TokensTotal   int    `json:"tokens_total"`
	CtxWindow     int    `json:"ctx_window"`
	StartedAt     string `json:"started_at"`
}

// handleSessionUsage は POST /api/session-usage を処理する。
// token 認証必須。メモリ保持のみで、ディスクへは書き込まない（セキュリティ要件）。
func (s *Server) handleSessionUsage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req sessionUsageRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.SessionID <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	usageModel := strings.TrimSpace(req.Model)
	if usageModel == "" {
		usageModel = s.sessionUsageModel(req.SessionID)
	}

	// コスト決定:
	//   - Claude: relay が計算済みコストを送る（CostFromRelay=true）→ そのまま採用。
	//   - Codex: トークン数 + 価格表で算出。価格表未登録なら CostKnown=false。
	var costUSD float64
	var costKnown bool
	if req.CostFromRelay {
		costUSD = req.CostUSD
		costKnown = true
	} else {
		costUSD, costKnown = calcCostUSD(usageModel, req.TokensIn, req.TokensOut, req.TokensCache)
	}

	stat := &usageStat{
		CostUSD:     costUSD,
		CostKnown:   costKnown,
		TokensIn:    req.TokensIn,
		TokensOut:   req.TokensOut,
		TokensCache: req.TokensCache,
		TokensTotal: req.TokensTotal,
		CtxWindow:   req.CtxWindow,
		UsageModel:  usageModel,
		StartedAt:   req.StartedAt,
		ReceivedAt:  time.Now(),
	}

	usageStatsMu.Lock()
	usageStats[req.SessionID] = stat
	usageStatsMu.Unlock()

	s.logger.Info("usage_stat received",
		slog.Int("session_id", req.SessionID),
		slog.String("provider", req.Provider),
		slog.String("model", usageModel),
		slog.Float64("cost_usd", costUSD),
		slog.Bool("cost_known", costKnown),
		slog.Int("tokens_in", req.TokensIn),
		slog.Int("tokens_out", req.TokensOut),
		slog.Int("tokens_cache", req.TokensCache),
		slog.Int("tokens_total", req.TokensTotal),
	)

	// C3: usage 更新を全 UI クライアントに broadcast する。
	s.broadcast(proto.Message{
		Type:           "usage_stat",
		SessionID:      req.SessionID,
		Provider:       req.Provider,
		CostUSD:        costUSD,
		CostKnown:      costKnown,
		TokensIn:       req.TokensIn,
		TokensOut:      req.TokensOut,
		TokensCache:    req.TokensCache,
		TokensTotal:    req.TokensTotal,
		CtxWindow:      req.CtxWindow,
		UsageModel:     usageModel,
		UsageStartedAt: req.StartedAt,
	})

	writeJSON(w, map[string]any{"ok": true})
}

// GetSessionUsageStat はセッション ID に対応する usageStat を返す（C3 から呼び出し予定）。
// 存在しない場合は nil を返す。
func GetSessionUsageStat(sessionID int) *usageStat {
	usageStatsMu.Lock()
	defer usageStatsMu.Unlock()
	return usageStats[sessionID]
}

// DeleteSessionUsageStat はセッション終了時に usage をメモリから削除する（C3 で呼び出し予定）。
func DeleteSessionUsageStat(sessionID int) {
	usageStatsMu.Lock()
	defer usageStatsMu.Unlock()
	delete(usageStats, sessionID)
}
