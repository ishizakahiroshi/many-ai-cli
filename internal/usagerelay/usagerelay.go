// Package usagerelay は "many-ai-cli usage-relay" 隠しサブコマンドの実装。
//
// Claude（statusLine）/ Codex（Stop フック）の両モードに対応し、
// stdin の JSON から数値メタのみを抽出して Hub の POST /api/session-usage へ送信する。
//
// セキュリティ要件（必須・C1 完了条件）:
//   - Hub へ送るのは数値メタ（トークン数・コスト・モデル名・経過時間）のみ。
//     プロンプト/コード/ツール入出力などの本文は一切送らない。
//   - Codex rollout JSONL では token_count イベントの数値だけを抽出し、
//     会話本文の行は読み捨てる（メモリに保持しない）。
//   - HTTP 送信失敗は stderr に warn を出すが exit 0 で抜ける
//     （statusLine/フックを壊さない）。
package usagerelay

import (
	"bufio"
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"os"
	"time"
)

// hubTokenEnv は relay が Hub auth token を読み出す環境変数名。
// CLI 引数 --token は /proc/<pid>/cmdline / ps aux 経由で他ユーザーから読み取れる
// ため deprecated。AI CLI から spawn される relay は親プロセスの環境変数を継承する
// ので、env 経由なら argv に token が現れない。
const hubTokenEnv = "MANY_AI_CLI_HUB_TOKEN"

// Run は "many-ai-cli usage-relay" サブコマンドのエントリポイント。
func Run(args []string) error {
	fs := flag.NewFlagSet("usage-relay", flag.ContinueOnError)
	provider := fs.String("provider", "", "provider: claude or codex")
	hub := fs.String("hub", "", "Hub base URL (e.g. http://127.0.0.1:47777)")
	tokenArg := fs.String("token", "", "Hub auth token (deprecated: use "+hubTokenEnv+" env instead)")
	sessionID := fs.Int("session", 0, "session ID")
	if err := fs.Parse(args); err != nil {
		return err
	}

	logger := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelWarn}))

	// token は env を最優先で読む。空ならフォールバックで --token CLI 引数を使う
	// （旧 config 互換）。env 経由なら argv に出現しないため procfs / ps から漏れない。
	token := os.Getenv(hubTokenEnv)
	if token == "" {
		token = *tokenArg
		if token != "" {
			logger.Warn("usage-relay: --token CLI arg is deprecated and exposes the token via process argv; use " + hubTokenEnv + " env instead")
		}
	}

	switch *provider {
	case "claude":
		return runClaude(*hub, token, *sessionID, os.Stdin, os.Stdout, logger)
	case "codex":
		return runCodex(*hub, token, *sessionID, os.Stdin, logger)
	default:
		return fmt.Errorf("usage-relay: --provider must be claude or codex (got %q)", *provider)
	}
}

// ---------------------------------------------------------------------------
// Claude モード
// ---------------------------------------------------------------------------
//
// Claude statusLine は次の JSON を stdin に渡す（公式ドキュメント
// https://code.claude.com/docs/en/statusline.md で確認済みのフィールド抜粋）:
//
//	{
//	  "model": { "display_name": "claude-opus-4-5-20251101" },
//	  "cost": { "total_cost_usd": 0.18, "total_duration_ms": 42000 },
//	  "context_window": {
//	    "total_input_tokens": 15500,
//	    "total_output_tokens": 1200,
//	    "context_window_size": 200000,
//	    "current_usage": {
//	      "cache_creation_input_tokens": 5000,
//	      "cache_read_input_tokens": 2000
//	    }
//	  },
//	  "workspace": { "project": "..." }
//	}
//
// context_window は「現在のコンテキストウィンドウの使用量」（最新 API 応答時点）。
// Claude Code v2.1.132 以降はセッション累積値ではない点に注意。
//
// relay は cost / model / context_window のトークン数を取り出して Hub へ POST し、
// stdout には最小限のステータス行を返す。
// （CLI ターミナルの statusLine 表示に使われる。短く 1 行で返すこと）

type claudeStatusLineInput struct {
	Model struct {
		DisplayName string `json:"display_name"`
	} `json:"model"`
	Cost struct {
		TotalCostUSD    float64 `json:"total_cost_usd"`
		TotalDurationMs float64 `json:"total_duration_ms"`
	} `json:"cost"`
	ContextWindow struct {
		TotalInputTokens  int `json:"total_input_tokens"`
		TotalOutputTokens int `json:"total_output_tokens"`
		ContextWindowSize int `json:"context_window_size"`
		CurrentUsage      struct {
			CacheCreationInputTokens int `json:"cache_creation_input_tokens"`
			CacheReadInputTokens     int `json:"cache_read_input_tokens"`
		} `json:"current_usage"`
	} `json:"context_window"`
}

func runClaude(hubURL, token string, sessionID int, stdin io.Reader, stdout io.Writer, logger *slog.Logger) error {
	raw, err := io.ReadAll(stdin)
	if err != nil {
		logger.Warn("usage-relay(claude): stdin read failed", "err", err)
		// exit 0 — statusLine を壊さない
		return nil
	}

	var input claudeStatusLineInput
	if err := json.Unmarshal(raw, &input); err != nil {
		logger.Warn("usage-relay(claude): JSON parse failed", "err", err)
		return nil
	}

	modelName := input.Model.DisplayName
	costUSD := input.Cost.TotalCostUSD
	tokIn := input.ContextWindow.TotalInputTokens
	tokOut := input.ContextWindow.TotalOutputTokens
	// cache 率の分子は cache_read のみ（実際にキャッシュヒットした分）。
	// cache_creation まで含めると分子≒分母となり常時 100% 表示で指標として無意味になる。
	tokCache := input.ContextWindow.CurrentUsage.CacheReadInputTokens

	// stdout に最小限のステータス行を返す（CLI ターミナル側の statusLine 表示）。
	// "$ <cost>  <model>  ↑in ↓out" の形式。trim して 1 行のみ。
	statusLine := fmt.Sprintf("$%.4f  %s  ↑%s ↓%s", costUSD, modelName, formatTokens(tokIn), formatTokens(tokOut))
	_, _ = fmt.Fprintln(stdout, statusLine)

	// Hub へ数値メタのみを POST する。
	if hubURL != "" && token != "" && sessionID > 0 {
		payload := hubUsagePayload{
			Provider:      "claude",
			SessionID:     sessionID,
			CostUSD:       costUSD,
			CostFromRelay: true, // Claude は計算済みコストを送る
			Model:         modelName,
			TokensIn:      tokIn,
			TokensOut:     tokOut,
			TokensCache:   tokCache,
			TokensTotal:   tokIn + tokOut,
			CtxWindow:     input.ContextWindow.ContextWindowSize,
			StartedAt:     time.Now().Format(time.RFC3339),
		}
		if err := postUsage(hubURL, token, payload, logger); err != nil {
			logger.Warn("usage-relay(claude): post failed", "err", err)
		}
	}
	return nil
}

// formatTokens はトークン数を k / M 単位の短い文字列にする
// （フロント token-statusbar.ts の formatTok と同じ表記）。
func formatTokens(n int) string {
	switch {
	case n >= 1_000_000:
		return fmt.Sprintf("%.1fM", float64(n)/1_000_000)
	case n >= 1_000:
		return fmt.Sprintf("%.1fk", float64(n)/1_000)
	default:
		return fmt.Sprintf("%d", n)
	}
}

// ---------------------------------------------------------------------------
// Codex モード
// ---------------------------------------------------------------------------
//
// Codex Stop フックは stdin に次の JSON を渡す（要実機確認・複数フォーマットにフォールバック）:
//
//	{ "transcript_path": "/home/user/.codex/sessions/xxx/rollout.jsonl" }
//
// relay は rollout JSONL の末尾から token_count イベントを走査し、
// input/output/cached/total の数値のみを抽出する。
// 会話本文（テキスト行）はメモリに保持しない（セキュリティ要件）。
//
// rollout JSONL の token_count イベント想定フォーマット（要実機確認）:
//
//	{ "type": "token_count", "input": 1234, "output": 567, "cached": 89, "total": 1890 }
//
// または:
//
//	{ "type": "token_count", "info": { "total_token_usage": { "input": ..., "output": ..., "cached": ..., "total": ... } } }
//
// 現行 Codex rollout では event_msg.payload 配下に token_count が入り、
// total_token_usage は input_tokens / cached_input_tokens / output_tokens /
// total_tokens 名で届く。model_context_window も同じ info 配下に入る。
//
// フォーマットが不確実なため、複数のキー名にフォールバックする寛容なパーサを採用する。

type codexStopInput struct {
	TranscriptPath string `json:"transcript_path"`
	// Model: フックが model を直接渡す場合
	Model string `json:"model"`
}

type tokenUsageNumbers struct {
	Input  int `json:"input"`
	Output int `json:"output"`
	Cached int `json:"cached"`
	Total  int `json:"total"`

	InputTokens           int `json:"input_tokens"`
	OutputTokens          int `json:"output_tokens"`
	CachedInputTokens     int `json:"cached_input_tokens"`
	CachedTokens          int `json:"cached_tokens"`
	PromptTokens          int `json:"prompt_tokens"`
	CompletionTokens      int `json:"completion_tokens"`
	TotalTokens           int `json:"total_tokens"`
	ReasoningOutputTokens int `json:"reasoning_output_tokens"`
}

func (u tokenUsageNumbers) resolve() (in, out, cache, total int) {
	in = firstNonZero(u.InputTokens, u.Input, u.PromptTokens)
	out = firstNonZero(u.OutputTokens, u.Output, u.CompletionTokens)
	cache = firstNonZero(u.CachedInputTokens, u.Cached, u.CachedTokens)
	total = firstNonZero(u.TotalTokens, u.Total)
	if total == 0 && (in > 0 || out > 0) {
		total = in + out
	}
	return in, out, cache, total
}

func firstNonZero(vals ...int) int {
	for _, v := range vals {
		if v != 0 {
			return v
		}
	}
	return 0
}

// tokenCountEvent は rollout JSONL の token_count イベントを寛容にパースする。
// 実機確認前は複数のキー名にフォールバックする。
// 要実機確認: フォーマットが確定次第このコメントと不要なフォールバックを削除する。
type tokenCountEvent struct {
	// フォーマット A: フラットな数値フィールド
	tokenUsageNumbers

	// フォーマット B: info.total_token_usage ネスト
	Info struct {
		TotalTokenUsage    tokenUsageNumbers `json:"total_token_usage"`
		LastTokenUsage     tokenUsageNumbers `json:"last_token_usage"`
		ModelContextWindow int               `json:"model_context_window"`
	} `json:"info"`

	// フォーマット C: usage フィールド
	Usage tokenUsageNumbers `json:"usage"`

	// 現行 Codex rollout: {"type":"event_msg","payload":{"type":"token_count",...}}
	Payload struct {
		Type string `json:"type"`
		Info struct {
			TotalTokenUsage    tokenUsageNumbers `json:"total_token_usage"`
			LastTokenUsage     tokenUsageNumbers `json:"last_token_usage"`
			ModelContextWindow int               `json:"model_context_window"`
		} `json:"info"`
		ModelContextWindow int `json:"model_context_window"`
	} `json:"payload"`

	ModelContextWindow int `json:"model_context_window"`
}

// resolve は複数フォーマットを試して (tokIn, tokOut, tokCache, tokTotal, ctxWindow) を返す。
//
// 各フォーマット内で TotalTokenUsage が空（累計未集計の初回ラウンド等）の場合は
// LastTokenUsage にフォールバックする。これがないと最初のターンの使用量が常にゼロ
// 表示になり、ステータスバーが沈黙する。
func (e *tokenCountEvent) resolve() (in, out, cache, total, ctxWindow int) {
	// 現行 Codex rollout の event_msg.payload ネストを最優先する。
	if e.Payload.Type == "token_count" {
		in, out, cache, total = e.Payload.Info.TotalTokenUsage.resolve()
		if in > 0 || out > 0 || total > 0 {
			return in, out, cache, total, firstNonZero(e.Payload.Info.ModelContextWindow, e.Payload.ModelContextWindow)
		}
		// TotalTokenUsage が空なら LastTokenUsage を試す。
		in, out, cache, total = e.Payload.Info.LastTokenUsage.resolve()
		if in > 0 || out > 0 || total > 0 {
			return in, out, cache, total, firstNonZero(e.Payload.Info.ModelContextWindow, e.Payload.ModelContextWindow)
		}
	}
	// フォーマット A
	in, out, cache, total = e.tokenUsageNumbers.resolve()
	if in > 0 || out > 0 || total > 0 {
		return in, out, cache, total, e.ModelContextWindow
	}
	// フォーマット B
	in, out, cache, total = e.Info.TotalTokenUsage.resolve()
	if in > 0 || out > 0 || total > 0 {
		return in, out, cache, total, e.Info.ModelContextWindow
	}
	in, out, cache, total = e.Info.LastTokenUsage.resolve()
	if in > 0 || out > 0 || total > 0 {
		return in, out, cache, total, e.Info.ModelContextWindow
	}
	// フォーマット C (OpenAI 互換)
	in, out, cache, total = e.Usage.resolve()
	if in > 0 || out > 0 || total > 0 {
		return in, out, cache, total, 0
	}
	return 0, 0, 0, 0, 0
}

// scanLastTokenCount は rollout JSONL を末尾から走査し、
// 最新の token_count イベントの数値だけを返す。
//
// セキュリティ要件: 会話本文の行はメモリに保持しない。
// 各行を読んだら type フィールドを確認し、token_count 以外は即座に破棄する。
func scanLastTokenCount(path string) (in, out, cache, total, ctxWindow int, err error) {
	f, err := os.Open(path)
	if err != nil {
		return 0, 0, 0, 0, 0, fmt.Errorf("open rollout: %w", err)
	}
	defer f.Close()

	var lastIn, lastOut, lastCache, lastTotal, lastCtxWindow int
	scanner := bufio.NewScanner(f)
	// バッファを 256 KB に制限（1 行が異常に長い場合の保護）
	scanner.Buffer(make([]byte, 256*1024), 256*1024)

	for scanner.Scan() {
		line := scanner.Bytes()
		// type フィールドが "token_count" の行のみを処理する。
		// 会話本文（type: "message" 等）はここで読み捨てる（セキュリティ要件）。
		if !isTokenCountLine(line) {
			// 本文行はメモリに保持せず即破棄
			continue
		}
		var ev tokenCountEvent
		if err := json.Unmarshal(line, &ev); err != nil {
			continue
		}
		i, o, c, t, ctx := ev.resolve()
		if i > 0 || o > 0 || t > 0 {
			lastIn, lastOut, lastCache, lastTotal = i, o, c, t
			if ctx > 0 {
				lastCtxWindow = ctx
			}
		}
	}
	if err := scanner.Err(); err != nil {
		return 0, 0, 0, 0, 0, fmt.Errorf("scan rollout: %w", err)
	}
	return lastIn, lastOut, lastCache, lastTotal, lastCtxWindow, nil
}

// isTokenCountLine は行が token_count イベントかどうかを JSON 全パース前に
// バイト列スキャンで高速判定する。偽陽性は許容（後段で再 Unmarshal）。
func isTokenCountLine(line []byte) bool {
	return bytes.Contains(line, []byte(`"token_count"`))
}

func runCodex(hubURL, token string, sessionID int, stdin io.Reader, logger *slog.Logger) error {
	raw, err := io.ReadAll(stdin)
	if err != nil {
		logger.Warn("usage-relay(codex): stdin read failed", "err", err)
		return nil
	}

	var input codexStopInput
	if err := json.Unmarshal(raw, &input); err != nil {
		logger.Warn("usage-relay(codex): JSON parse failed", "err", err)
		return nil
	}

	if input.TranscriptPath == "" {
		logger.Warn("usage-relay(codex): transcript_path not found in stdin JSON")
		return nil
	}

	// rollout JSONL から token_count の数値のみを抽出。本文行は読み捨て。
	tokIn, tokOut, tokCache, tokTotal, ctxWindow, err := scanLastTokenCount(input.TranscriptPath)
	if err != nil {
		logger.Warn("usage-relay(codex): rollout scan failed", "path", input.TranscriptPath, "err", err)
		return nil
	}

	if hubURL != "" && token != "" && sessionID > 0 {
		payload := hubUsagePayload{
			Provider:      "codex",
			SessionID:     sessionID,
			CostFromRelay: false, // Codex は Hub 側で価格表算出
			Model:         input.Model,
			TokensIn:      tokIn,
			TokensOut:     tokOut,
			TokensCache:   tokCache,
			TokensTotal:   tokTotal,
			CtxWindow:     ctxWindow,
			StartedAt:     time.Now().Format(time.RFC3339),
		}
		if err := postUsage(hubURL, token, payload, logger); err != nil {
			logger.Warn("usage-relay(codex): post failed", "err", err)
		}
	}
	return nil
}

// ---------------------------------------------------------------------------
// Hub への POST
// ---------------------------------------------------------------------------

// hubUsagePayload は /api/session-usage への送信ペイロード。
// 数値メタのみを持ち、本文フィールドは存在しない（セキュリティ要件）。
type hubUsagePayload struct {
	Provider      string  `json:"provider"`
	SessionID     int     `json:"session_id"`
	CostUSD       float64 `json:"cost_usd,omitempty"`
	CostFromRelay bool    `json:"cost_from_relay"`
	Model         string  `json:"model,omitempty"`
	TokensIn      int     `json:"tokens_in,omitempty"`
	TokensOut     int     `json:"tokens_out,omitempty"`
	TokensCache   int     `json:"tokens_cache,omitempty"`
	TokensTotal   int     `json:"tokens_total,omitempty"`
	// CtxWindow: モデルのコンテキストウィンドウ上限（Claude statusline の
	// context_window_size をそのまま中継。取得できないプロバイダは 0）。
	CtxWindow int    `json:"ctx_window,omitempty"`
	StartedAt string `json:"started_at,omitempty"`
}

func postUsage(hubURL, token string, payload hubUsagePayload, logger *slog.Logger) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("marshal payload: %w", err)
	}

	// token は URL クエリではなく Authorization ヘッダで送る
	// （HTTP アクセスログ・上流プロキシのログに ?token=<hex> が残らないようにするため）。
	// Hub 側 requestToken() は Authorization Bearer / Cookie / URL クエリの 3 経路を
	// サポートしているので互換性に問題はない（06-15 監査で確認済）。
	url := hubURL + "/api/session-usage"
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}

	client := &http.Client{Timeout: 5 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("http post: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("hub returned %d", resp.StatusCode)
	}
	logger.Info("usage-relay: posted to hub",
		slog.Int("session_id", payload.SessionID),
		slog.String("provider", payload.Provider),
	)
	return nil
}
