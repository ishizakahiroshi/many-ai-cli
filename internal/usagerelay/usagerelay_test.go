package usagerelay

import (
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func TestScanLastTokenCountCodexEventMsgFormat(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rollout.jsonl")
	body := []byte(`{"timestamp":"2026-06-15T14:34:49.109Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":16457,"cached_input_tokens":10624,"output_tokens":135,"reasoning_output_tokens":88,"total_tokens":16592},"last_token_usage":{"input_tokens":16457,"cached_input_tokens":10624,"output_tokens":135,"reasoning_output_tokens":88,"total_tokens":16592},"model_context_window":258400}}}
{"timestamp":"2026-06-15T14:35:03.781Z","type":"response_item","payload":{"type":"message","content":"ignored body"}}
{"timestamp":"2026-06-15T14:35:17.360Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":33079,"cached_input_tokens":26880,"output_tokens":473,"reasoning_output_tokens":128,"total_tokens":33552},"last_token_usage":{"input_tokens":16622,"cached_input_tokens":16256,"output_tokens":338,"reasoning_output_tokens":40,"total_tokens":16960},"model_context_window":258400}}}
`)
	if err := os.WriteFile(path, body, 0o600); err != nil {
		t.Fatal(err)
	}

	in, out, cache, total, ctxWindow, reasoningOut, err := scanLastTokenCount(path)
	if err != nil {
		t.Fatal(err)
	}
	if in != 33079 || out != 473 || cache != 26880 || total != 33552 || ctxWindow != 258400 || reasoningOut != 128 {
		t.Fatalf("scanLastTokenCount = in=%d out=%d cache=%d total=%d ctx=%d reasoning=%d, want 33079/473/26880/33552/258400/128",
			in, out, cache, total, ctxWindow, reasoningOut)
	}
}

func TestRunCodexPropagatesReasoningOutputTokens(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rollout.jsonl")
	body := []byte(`{"timestamp":"2026-06-15T14:35:17.360Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":33079,"cached_input_tokens":26880,"output_tokens":473,"reasoning_output_tokens":128,"total_tokens":33552},"last_token_usage":{"input_tokens":16622,"cached_input_tokens":16256,"output_tokens":338,"reasoning_output_tokens":40,"total_tokens":16960},"model_context_window":258400}}}
`)
	if err := os.WriteFile(path, body, 0o600); err != nil {
		t.Fatal(err)
	}

	var got hubUsagePayload
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Errorf("decode payload: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()

	stdin := `{"transcript_path":` + strconv.Quote(path) + `,"model":"gpt-5 high"}`
	logger := slog.New(slog.NewTextHandler(io.Discard, nil))
	if err := runCodex(srv.URL, "tok", 1, strings.NewReader(stdin), logger); err != nil {
		t.Fatalf("runCodex: %v", err)
	}
	if got.ReasoningOut != 128 {
		t.Fatalf("ReasoningOut = %d, want 128", got.ReasoningOut)
	}
}

// TestRunClaudePropagatesUsedPercentage は Claude statusLine の
// context_window.used_percentage が relay の POST payload（ctx_used_pct）まで
// 欠落なく伝わること、及び欠落時（Codex 相当）は 0 のまま流れることを検証する。
func TestRunClaudePropagatesUsedPercentage(t *testing.T) {
	cases := []struct {
		name       string
		statusLine string
		wantPct    float64
		wantWindow int
		wantIn     int
	}{
		{
			name:       "with used_percentage (200k session)",
			statusLine: `{"model":{"display_name":"Opus 4.8"},"cost":{"total_cost_usd":0.1234},"context_window":{"total_input_tokens":123200,"total_output_tokens":4500,"context_window_size":200000,"used_percentage":68.4,"current_usage":{"cache_creation_input_tokens":5000,"cache_read_input_tokens":2000}}}`,
			wantPct:    68.4,
			wantWindow: 200000,
			wantIn:     123200,
		},
		{
			name:       "without used_percentage (early session / absent field)",
			statusLine: `{"model":{"display_name":"Opus 4.8"},"cost":{"total_cost_usd":0.01},"context_window":{"total_input_tokens":1000,"total_output_tokens":50,"context_window_size":200000,"current_usage":{"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}`,
			wantPct:    0,
			wantWindow: 200000,
			wantIn:     1000,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var got hubUsagePayload
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
					t.Errorf("decode payload: %v", err)
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(`{"ok":true}`))
			}))
			defer srv.Close()

			logger := slog.New(slog.NewTextHandler(io.Discard, nil))
			// runClaude は postUsage を同期実行するため、戻り時点で payload は受信済み。
			if err := runClaude(srv.URL, "tok", 1, strings.NewReader(tc.statusLine), io.Discard, logger); err != nil {
				t.Fatalf("runClaude: %v", err)
			}
			if got.CtxUsedPct != tc.wantPct {
				t.Fatalf("CtxUsedPct = %v, want %v", got.CtxUsedPct, tc.wantPct)
			}
			if got.CtxWindow != tc.wantWindow {
				t.Fatalf("CtxWindow = %d, want %d", got.CtxWindow, tc.wantWindow)
			}
			if got.TokensIn != tc.wantIn {
				t.Fatalf("TokensIn = %d, want %d", got.TokensIn, tc.wantIn)
			}
		})
	}
}

// TestRunClaudePropagatesStatusbarMeta は statusbar 追加メタ（rate_limits / lines /
// effort / thinking / exceeds_200k / durations）が relay の POST payload まで欠落なく
// 伝わることを検証する。
func TestRunClaudePropagatesStatusbarMeta(t *testing.T) {
	var got hubUsagePayload
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Errorf("decode payload: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()

	statusLine := `{` +
		`"model":{"display_name":"Opus 4.8"},` +
		`"cost":{"total_cost_usd":0.5,"total_duration_ms":45000,"total_api_duration_ms":2300,"total_lines_added":156,"total_lines_removed":23},` +
		`"context_window":{"total_input_tokens":123200,"total_output_tokens":4500,"context_window_size":200000,"used_percentage":68.4},` +
		`"exceeds_200k_tokens":true,` +
		`"effort":{"level":"high"},` +
		`"thinking":{"enabled":true},` +
		`"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}` +
		`}`

	logger := slog.New(slog.NewTextHandler(io.Discard, nil))
	if err := runClaude(srv.URL, "tok", 1, strings.NewReader(statusLine), io.Discard, logger); err != nil {
		t.Fatalf("runClaude: %v", err)
	}
	if got.RateLimit5hPct != 23.5 || got.RateLimit5hReset != 1738425600 {
		t.Fatalf("5h rate limit = %v / %d, want 23.5 / 1738425600", got.RateLimit5hPct, got.RateLimit5hReset)
	}
	if got.RateLimit7dPct != 41.2 || got.RateLimit7dReset != 1738857600 {
		t.Fatalf("7d rate limit = %v / %d, want 41.2 / 1738857600", got.RateLimit7dPct, got.RateLimit7dReset)
	}
	if got.LinesAdded != 156 || got.LinesRemoved != 23 {
		t.Fatalf("lines = +%d -%d, want +156 -23", got.LinesAdded, got.LinesRemoved)
	}
	if got.EffortLevel != "high" {
		t.Fatalf("effort = %q, want high", got.EffortLevel)
	}
	if !got.Thinking {
		t.Fatalf("thinking = false, want true")
	}
	if !got.Exceeds200k {
		t.Fatalf("exceeds_200k = false, want true")
	}
	if got.DurationMs != 45000 || got.APIDurationMs != 2300 {
		t.Fatalf("durations = %d / %d, want 45000 / 2300", got.DurationMs, got.APIDurationMs)
	}
}

// TestRunClaudePropagatesLowPriorityStatusbarMeta は低優先の Claude statusLine
// フィールドが relay の POST payload まで欠落なく伝わることを検証する。
func TestRunClaudePropagatesLowPriorityStatusbarMeta(t *testing.T) {
	var got hubUsagePayload
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Errorf("decode payload: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()

	statusLine := `{` +
		`"version":"2.1.132",` +
		`"model":{"display_name":"Opus 4.8"},` +
		`"output_style":{"name":"concise"},` +
		`"vim":{"mode":"INSERT"},` +
		`"agent":{"name":"reviewer"},` +
		`"workspace":{"repo":{"host":"github.com","owner":"ishizakahiroshi","name":"many-ai-cli"}},` +
		`"cost":{"total_cost_usd":0.5},` +
		`"context_window":{"total_input_tokens":123200,"total_output_tokens":4500,"context_window_size":200000,"used_percentage":68.4,"remaining_percentage":31.6}` +
		`}`

	logger := slog.New(slog.NewTextHandler(io.Discard, nil))
	if err := runClaude(srv.URL, "tok", 1, strings.NewReader(statusLine), io.Discard, logger); err != nil {
		t.Fatalf("runClaude: %v", err)
	}
	if got.Version != "2.1.132" || got.OutputStyle != "concise" || got.VimMode != "INSERT" || got.AgentName != "reviewer" {
		t.Fatalf("low-priority meta = version=%q output=%q vim=%q agent=%q",
			got.Version, got.OutputStyle, got.VimMode, got.AgentName)
	}
	if got.RepoHost != "github.com" || got.RepoOwner != "ishizakahiroshi" || got.RepoName != "many-ai-cli" {
		t.Fatalf("repo = %q/%q/%q, want github.com/ishizakahiroshi/many-ai-cli",
			got.RepoHost, got.RepoOwner, got.RepoName)
	}
	if got.RemainingPct != 31.6 {
		t.Fatalf("RemainingPct = %v, want 31.6", got.RemainingPct)
	}
}

func TestTokenCountEventResolveLegacyFormats(t *testing.T) {
	cases := []struct {
		name string
		ev   tokenCountEvent
		in   int
		out  int
		c    int
		tot  int
		ctx  int
	}{
		{
			name: "flat",
			ev: tokenCountEvent{
				tokenUsageNumbers:  tokenUsageNumbers{Input: 1, Output: 2, Cached: 3, Total: 4},
				ModelContextWindow: 5,
			},
			in:  1,
			out: 2,
			c:   3,
			tot: 4,
			ctx: 5,
		},
		{
			name: "usage-openai-compatible",
			ev: tokenCountEvent{
				Usage: tokenUsageNumbers{PromptTokens: 10, CompletionTokens: 20, CachedTokens: 7, TotalTokens: 30},
			},
			in:  10,
			out: 20,
			c:   7,
			tot: 30,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			in, out, cache, total, ctx, _ := tc.ev.resolve()
			if in != tc.in || out != tc.out || cache != tc.c || total != tc.tot || ctx != tc.ctx {
				t.Fatalf("resolve = in=%d out=%d cache=%d total=%d ctx=%d, want %d/%d/%d/%d/%d",
					in, out, cache, total, ctx, tc.in, tc.out, tc.c, tc.tot, tc.ctx)
			}
		})
	}
}
