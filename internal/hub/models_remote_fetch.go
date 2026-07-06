package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"time"
)

// modelsDefaults は GitHub から取得する resources/models/defaults.json のスキーマ。
// 形式: {"anthropic": [{"id":"...", "label":"..."}, ...], "openai": [...], ...}
type modelsDefaults struct {
	Anthropic []Model `json:"anthropic"`
	OpenAI    []Model `json:"openai"`
	// Copilot は GitHub Copilot CLI の `--model` に渡す候補。route/env 注入は行わず、
	// 対応可否は Copilot CLI 側に委ねる（slug が外れても datalist の自由入力で補える）。
	Copilot []Model `json:"copilot"`
	// CursorAgent は cursor-agent CLI の `--model` に渡す候補。route/env 注入は行わず、
	// 対応可否は cursor-agent 側に委ねる（全 ID は `cursor-agent --list-models` 参照、datalist で自由入力可）。
	CursorAgent []Model `json:"cursor-agent"`
	// Grok は Grok Build CLI の `--model` に渡す候補。route/env 注入は行わず、
	// 対応可否は grok 側に委ねる（全 ID は `grok models` 参照、datalist で自由入力可）。
	Grok []Model `json:"grok"`
}

const (
	modelsRemoteCacheTTL    = 24 * time.Hour
	modelsRemoteNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

// newModelsRemoteCache は GitHub のモデル defaults を扱う TTL キャッシュを生成する。
// 失敗時は空を返し、静的 fallback は持たない。
func newModelsRemoteCache() *ttlCache[modelsDefaults] {
	return &ttlCache[modelsDefaults]{
		ttl:         modelsRemoteCacheTTL,
		negativeTTL: modelsRemoteNegativeTTL,
		fetch:       fetchModelsDefaults,
	}
}

func fetchModelsDefaults(sourceURL string) (modelsDefaults, error) {
	client := makeExternalHTTPClient(10 * time.Second)
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
