package usagerelay

import (
	"os"
	"path/filepath"
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

	in, out, cache, total, ctxWindow, err := scanLastTokenCount(path)
	if err != nil {
		t.Fatal(err)
	}
	if in != 33079 || out != 473 || cache != 26880 || total != 33552 || ctxWindow != 258400 {
		t.Fatalf("scanLastTokenCount = in=%d out=%d cache=%d total=%d ctx=%d, want 33079/473/26880/33552/258400",
			in, out, cache, total, ctxWindow)
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
			in, out, cache, total, ctx := tc.ev.resolve()
			if in != tc.in || out != tc.out || cache != tc.c || total != tc.tot || ctx != tc.ctx {
				t.Fatalf("resolve = in=%d out=%d cache=%d total=%d ctx=%d, want %d/%d/%d/%d/%d",
					in, out, cache, total, ctx, tc.in, tc.out, tc.c, tc.tot, tc.ctx)
			}
		})
	}
}
