package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"time"
)

// UsageLinkDefaults は provider ごとの usage リンクデフォルト URL。
type UsageLinkDefaults struct {
	Claude      string `json:"claude"`
	Codex       string `json:"codex"`
	Copilot     string `json:"copilot"`
	CursorAgent string `json:"cursor-agent"`
	Ollama      string `json:"ollama"`
	OpenCode    string `json:"opencode"`
}

const (
	usageLinkCacheTTL    = 24 * time.Hour
	usageLinkNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

// hardcodedUsageLinkDefaults は GitHub fetch 失敗時のフォールバック値。
// app.js の DEFAULT_USAGE_LINKS と同じ値に揃えておく。
var hardcodedUsageLinkDefaults = UsageLinkDefaults{
	Claude:      "https://claude.ai/settings/usage",
	Codex:       "https://chatgpt.com/codex/cloud/settings/analytics#usage",
	Copilot:     "https://github.com/settings/billing",
	CursorAgent: "https://cursor.com/dashboard",
	Ollama:      "https://ollama.com/settings",
	OpenCode:    "https://opencode.ai/go",
}

// newUsageLinkCache は usage リンクデフォルトの TTL キャッシュを生成する。
func newUsageLinkCache() *ttlCache[UsageLinkDefaults] {
	return &ttlCache[UsageLinkDefaults]{
		ttl:         usageLinkCacheTTL,
		negativeTTL: usageLinkNegativeTTL,
		fallback:    hardcodedUsageLinkDefaults,
		fetch:       fetchUsageLinkDefaults,
	}
}

func fetchUsageLinkDefaults(sourceURL string) (UsageLinkDefaults, error) {
	client := makeExternalHTTPClient(10 * time.Second)
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
