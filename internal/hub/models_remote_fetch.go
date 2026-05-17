package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

// modelsDefaults は GitHub から取得する resources/models/defaults.json のスキーマ。
// 形式: {"anthropic": [{"id":"...", "label":"..."}, ...], "openai": [...]}
type modelsDefaults struct {
	Anthropic []Model `json:"anthropic"`
	OpenAI    []Model `json:"openai"`
}

type modelsRemoteCache struct {
	mu        sync.Mutex
	data      *modelsDefaults
	fetchedAt time.Time
}

const modelsRemoteCacheTTL = 24 * time.Hour

// hardcodedModelsDefaults は GitHub fetch 失敗時のフォールバック値。
// resources/models/defaults.json と同じ値を保つ。
var hardcodedModelsDefaults = modelsDefaults{
	Anthropic: []Model{
		{ID: "claude-opus-4-7", Label: "Claude Opus 4.7"},
		{ID: "claude-sonnet-4-6", Label: "Claude Sonnet 4.6"},
		{ID: "claude-haiku-4-5", Label: "Claude Haiku 4.5"},
		{ID: "claude-sonnet-4-5", Label: "Claude Sonnet 4.5"},
		{ID: "claude-3-7-sonnet-latest", Label: "Claude 3.7 Sonnet"},
		{ID: "claude-3-5-sonnet-latest", Label: "Claude 3.5 Sonnet"},
	},
	OpenAI: []Model{
		{ID: "gpt-5.5", Label: "GPT-5.5"},
		{ID: "gpt-5.4", Label: "GPT-5.4"},
		{ID: "gpt-5.4-mini", Label: "GPT-5.4 mini"},
		{ID: "gpt-5.4-nano", Label: "GPT-5.4 nano"},
	},
}

// get は TTL 内ならキャッシュを返し、期限切れなら sourceURL から再取得する。
// 取得失敗時はハードコード値を返す（stale は使わない）。
func (c *modelsRemoteCache) get(sourceURL string) modelsDefaults {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.data != nil && time.Since(c.fetchedAt) < modelsRemoteCacheTTL {
		return *c.data
	}
	fetched, err := fetchModelsDefaults(sourceURL)
	if err != nil {
		return hardcodedModelsDefaults
	}
	merged := mergeModelsDefaults(fetched, hardcodedModelsDefaults)
	c.data = &merged
	c.fetchedAt = time.Now()
	return merged
}

// invalidate は次回 get で必ず再 fetch されるようキャッシュを破棄する。
func (c *modelsRemoteCache) invalidate() {
	c.mu.Lock()
	c.data = nil
	c.fetchedAt = time.Time{}
	c.mu.Unlock()
}

func fetchModelsDefaults(sourceURL string) (modelsDefaults, error) {
	client := &http.Client{Timeout: 10 * time.Second}
	resp, err := client.Get(sourceURL)
	if err != nil {
		return modelsDefaults{}, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 256*1024))
	if err != nil {
		return modelsDefaults{}, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return modelsDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	var d modelsDefaults
	if err := json.Unmarshal(body, &d); err != nil {
		return modelsDefaults{}, err
	}
	return d, nil
}

func mergeModelsDefaults(preferred, fallback modelsDefaults) modelsDefaults {
	return modelsDefaults{
		Anthropic: mergeModelList(preferred.Anthropic, fallback.Anthropic),
		OpenAI:    mergeModelList(preferred.OpenAI, fallback.OpenAI),
	}
}

func mergeModelList(preferred, fallback []Model) []Model {
	out := make([]Model, 0, len(preferred)+len(fallback))
	seen := map[string]bool{}
	for _, m := range preferred {
		if m.ID == "" || seen[m.ID] {
			continue
		}
		seen[m.ID] = true
		out = append(out, m)
	}
	for _, m := range fallback {
		if m.ID == "" || seen[m.ID] {
			continue
		}
		seen[m.ID] = true
		out = append(out, m)
	}
	return out
}
