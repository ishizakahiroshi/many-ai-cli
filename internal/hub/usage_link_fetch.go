package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

// UsageLinkDefaults は provider ごとの usage リンクデフォルト URL。
type UsageLinkDefaults struct {
	Claude   string `json:"claude"`
	Codex    string `json:"codex"`
	Ollama   string `json:"ollama"`
	OpenCode string `json:"opencode"`
}

type usageLinkCache struct {
	mu        sync.Mutex
	data      *UsageLinkDefaults
	fetchedAt time.Time
}

const usageLinkCacheTTL = 24 * time.Hour

// hardcodedUsageLinkDefaults は GitHub fetch 失敗時のフォールバック値。
// app.js の DEFAULT_USAGE_LINKS と同じ値に揃えておく。
var hardcodedUsageLinkDefaults = UsageLinkDefaults{
	Claude:   "https://claude.ai/settings/usage",
	Codex:    "https://chatgpt.com/codex/cloud/settings/analytics#usage",
	Ollama:   "https://ollama.com/settings",
	OpenCode: "",
}

// get は TTL 内ならキャッシュを返し、期限切れなら sourceURL から再取得する。
// 取得失敗時はハードコード値を返す（stale は使わない）。
func (c *usageLinkCache) get(sourceURL string) UsageLinkDefaults {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.data != nil && time.Since(c.fetchedAt) < usageLinkCacheTTL {
		return *c.data
	}
	fetched, err := fetchUsageLinkDefaults(sourceURL)
	if err != nil {
		return hardcodedUsageLinkDefaults
	}
	c.data = &fetched
	c.fetchedAt = time.Now()
	return fetched
}

func fetchUsageLinkDefaults(sourceURL string) (UsageLinkDefaults, error) {
	client := &http.Client{Timeout: 10 * time.Second}
	resp, err := client.Get(sourceURL)
	if err != nil {
		return UsageLinkDefaults{}, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 64*1024))
	if err != nil {
		return UsageLinkDefaults{}, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return UsageLinkDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	var d UsageLinkDefaults
	if err := json.Unmarshal(body, &d); err != nil {
		return UsageLinkDefaults{}, err
	}
	return d, nil
}
