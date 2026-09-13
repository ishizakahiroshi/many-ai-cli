package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"time"
)

// modelsDefaults は GitHub から取得する resources/models/defaults.json のスキーマ。
// map にすることで、新しい静的 catalog key を Go の struct へ追加せずに decode できる。
// picker group の追加や provider 起動・route は別の built-in 実装を必要とする。
type modelsDefaults map[string][]Model

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
