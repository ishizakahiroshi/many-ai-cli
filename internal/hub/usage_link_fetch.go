package hub

import (
	"encoding/json"
	"fmt"
	"io"
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
	failedAt  time.Time // 最後に fetch に失敗した時刻（負キャッシュ用）
}

const (
	usageLinkCacheTTL    = 24 * time.Hour
	usageLinkNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

// hardcodedUsageLinkDefaults は GitHub fetch 失敗時のフォールバック値。
// app.js の DEFAULT_USAGE_LINKS と同じ値に揃えておく。
var hardcodedUsageLinkDefaults = UsageLinkDefaults{
	Claude:   "https://claude.ai/settings/usage",
	Codex:    "https://chatgpt.com/codex/cloud/settings/analytics#usage",
	Ollama:   "https://ollama.com/settings",
	OpenCode: "",
}

// get は TTL 内ならキャッシュを返し、期限切れなら sourceURL から再取得する。
// 取得失敗時はハードコード値を返し、失敗時刻を記録して負 TTL 内は再試行しない。
func (c *usageLinkCache) get(sourceURL string) UsageLinkDefaults {
	c.mu.Lock()
	defer c.mu.Unlock()
	// 成功キャッシュが有効
	if c.data != nil && time.Since(c.fetchedAt) < usageLinkCacheTTL {
		return *c.data
	}
	// 負キャッシュが有効（失敗後の再試行抑制）
	if !c.failedAt.IsZero() && time.Since(c.failedAt) < usageLinkNegativeTTL {
		if c.data != nil {
			return *c.data
		}
		return hardcodedUsageLinkDefaults
	}
	fetched, err := fetchUsageLinkDefaults(sourceURL)
	if err != nil {
		c.failedAt = time.Now() // 負 TTL 開始
		if c.data != nil {
			return *c.data // 直近成功値があればそちらを返す
		}
		return hardcodedUsageLinkDefaults
	}
	c.data = &fetched
	c.fetchedAt = time.Now()
	c.failedAt = time.Time{} // 負キャッシュをリセット
	return fetched
}

func fetchUsageLinkDefaults(sourceURL string) (UsageLinkDefaults, error) {
	client := newExternalHTTPClient(10 * time.Second)
	resp, err := client.Get(sourceURL)
	if err != nil {
		return UsageLinkDefaults{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// エラーページを読み捨てて早期リターン（ボディは読まない）
		return UsageLinkDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 64*1024))
	if err != nil {
		return UsageLinkDefaults{}, err
	}
	var d UsageLinkDefaults
	if err := json.Unmarshal(body, &d); err != nil {
		return UsageLinkDefaults{}, err
	}
	return d, nil
}
