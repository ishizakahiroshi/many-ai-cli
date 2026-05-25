package hub

import (
	"encoding/json"
	"fmt"
	"io"
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
	mu          sync.Mutex
	data        *modelsDefaults
	fetchedAt   time.Time
	failedAt    time.Time // 最後に fetch に失敗した時刻（負キャッシュ用）
}

const (
	modelsRemoteCacheTTL    = 24 * time.Hour
	modelsRemoteNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

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
// 取得失敗時はハードコード値を返し、失敗時刻を記録して負 TTL 内は再試行しない。
func (c *modelsRemoteCache) get(sourceURL string) modelsDefaults {
	c.mu.Lock()
	defer c.mu.Unlock()
	// 成功キャッシュが有効
	if c.data != nil && time.Since(c.fetchedAt) < modelsRemoteCacheTTL {
		return *c.data
	}
	// 負キャッシュが有効（失敗後の再試行抑制）
	if !c.failedAt.IsZero() && time.Since(c.failedAt) < modelsRemoteNegativeTTL {
		if c.data != nil {
			return *c.data
		}
		return hardcodedModelsDefaults
	}
	fetched, err := fetchModelsDefaults(sourceURL)
	if err != nil {
		c.failedAt = time.Now() // 負 TTL 開始
		if c.data != nil {
			return *c.data // 直近成功値があればそちらを返す
		}
		return hardcodedModelsDefaults
	}
	merged := mergeModelsDefaults(fetched, hardcodedModelsDefaults)
	c.data = &merged
	c.fetchedAt = time.Now()
	c.failedAt = time.Time{} // 負キャッシュをリセット
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
	client := newExternalHTTPClient(10 * time.Second)
	resp, err := client.Get(sourceURL)
	if err != nil {
		return modelsDefaults{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// エラーページを読み捨てて早期リターン（ボディは読まない）
		return modelsDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 256*1024))
	if err != nil {
		return modelsDefaults{}, err
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
