// Package usagerelay は "any-ai-cli usage-relay" 隠しサブコマンドの実装。
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

// Run は "any-ai-cli usage-relay" サブコマンドのエントリポイント。
func Run(args []string) error {
	fs := flag.NewFlagSet("usage-relay", flag.ContinueOnError)
	provider := fs.String("provider", "", "provider: claude or codex")
	hub := fs.String("hub", "", "Hub base URL (e.g. http://127.0.0.1:47777)")
	token := fs.String("token", "", "Hub auth token")
	sessionID := fs.Int("session", 0, "session ID")
	if err := fs.Parse(args); err != nil {
		return err
	}

	logger := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelWarn}))

	switch *provider {
	case "claude":
		return runClaude(*hub, *token, *sessionID, os.Stdin, os.Stdout, logger)
	case "codex":
		return runCodex(*hub, *token, *sessionID, os.Stdin, logger)
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
	tokCache := input.ContextWindow.CurrentUsage.CacheCreationInputTokens +
		input.ContextWindow.CurrentUsage.CacheReadInputTokens

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
// フォーマットが不確実なため、複数のキー名にフォールバックする寛容なパーサを採用する。

type codexStopInput struct {
	TranscriptPath string `json:"transcript_path"`
	// Model: フックが model を直接渡す場合
	Model string `json:"model"`
}

// tokenCountEvent は rollout JSONL の token_count イベントを寛容にパースする。
// 実機確認前は複数のキー名にフォールバックする。
// 要実機確認: フォーマットが確定次第このコメントと不要なフォールバックを削除する。
type tokenCountEvent struct {
	// フォーマット A: フラットな数値フィールド
	Input  int `json:"input"`
	Output int `json:"output"`
	Cached int `json:"cached"`
	Total  int `json:"total"`

	// フォーマット B: info.total_token_usage ネスト
	Info struct {
		TotalTokenUsage struct {
			Input  int `json:"input"`
			Output int `json:"output"`
			Cached int `json:"cached"`
			Total  int `json:"total"`
		} `json:"total_token_usage"`
	} `json:"info"`

	// フォーマット C: usage フィールド
	Usage struct {
		Input  int `json:"input"`
		Output int `json:"output"`
		Cached int `json:"cached"`
		Total  int `json:"total"`
		// OpenAI 互換名
		PromptTokens     int `json:"prompt_tokens"`
		CompletionTokens int `json:"completion_tokens"`
		CachedTokens     int `json:"cached_tokens"`
		TotalTokens      int `json:"total_tokens"`
	} `json:"usage"`
}

// resolve は複数フォーマットを試して (tokIn, tokOut, tokCache, tokTotal) を返す。
func (e *tokenCountEvent) resolve() (in, out, cache, total int) {
	// フォーマット A
	if e.Input > 0 || e.Output > 0 {
		return e.Input, e.Output, e.Cached, e.Total
	}
	// フォーマット B
	u := e.Info.TotalTokenUsage
	if u.Input > 0 || u.Output > 0 {
		return u.Input, u.Output, u.Cached, u.Total
	}
	// フォーマット C (OpenAI 互換)
	v := e.Usage
	if v.PromptTokens > 0 || v.CompletionTokens > 0 {
		return v.PromptTokens, v.CompletionTokens, v.CachedTokens, v.TotalTokens
	}
	if v.Input > 0 || v.Output > 0 {
		return v.Input, v.Output, v.Cached, v.Total
	}
	return 0, 0, 0, 0
}

// scanLastTokenCount は rollout JSONL を末尾から走査し、
// 最新の token_count イベントの数値だけを返す。
//
// セキュリティ要件: 会話本文の行はメモリに保持しない。
// 各行を読んだら type フィールドを確認し、token_count 以外は即座に破棄する。
func scanLastTokenCount(path string) (in, out, cache, total int, err error) {
	f, err := os.Open(path)
	if err != nil {
		return 0, 0, 0, 0, fmt.Errorf("open rollout: %w", err)
	}
	defer f.Close()

	var lastIn, lastOut, lastCache, lastTotal int
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
		i, o, c, t := ev.resolve()
		if i > 0 || o > 0 || t > 0 {
			lastIn, lastOut, lastCache, lastTotal = i, o, c, t
		}
	}
	if err := scanner.Err(); err != nil {
		return 0, 0, 0, 0, fmt.Errorf("scan rollout: %w", err)
	}
	return lastIn, lastOut, lastCache, lastTotal, nil
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
	tokIn, tokOut, tokCache, tokTotal, err := scanLastTokenCount(input.TranscriptPath)
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
	StartedAt     string  `json:"started_at,omitempty"`
}

func postUsage(hubURL, token string, payload hubUsagePayload, logger *slog.Logger) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("marshal payload: %w", err)
	}

	url := hubURL + "/api/session-usage?token=" + token
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")

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
