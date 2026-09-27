package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"strings"
	"time"
)

// InstallLinkDefaults は provider id → 公式インストール手順 URL の map。
// map にすることで、新しい provider を足すたびに Go の struct を変更せずに
// resources/install-links/defaults.json 側の行を足すだけで済む（modelsDefaults と同じ方針）。
type InstallLinkDefaults map[string]string

const (
	installLinkCacheTTL    = 24 * time.Hour
	installLinkNegativeTTL = 3 * time.Minute // 失敗後の再試行抑制期間
)

// newInstallLinkCache は install リンクデフォルトの TTL キャッシュを生成する。
// usage-links と異なり、静的 fallback（Go 側への URL 複製）は持たない。取得に
// 失敗した場合は空の map を返し、画面はリンク無しの案内文だけを出す。
func newInstallLinkCache() *ttlCache[InstallLinkDefaults] {
	return &ttlCache[InstallLinkDefaults]{
		ttl:         installLinkCacheTTL,
		negativeTTL: installLinkNegativeTTL,
		fallback:    InstallLinkDefaults{},
		fetch:       fetchInstallLinkDefaults,
		transform:   sanitizeInstallLinkDefaults,
	}
}

func fetchInstallLinkDefaults(sourceURL string) (InstallLinkDefaults, error) {
	client := makeExternalHTTPClient(10 * time.Second)
	resp, err := client.Get(sourceURL)
	if err != nil {
		return InstallLinkDefaults{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// エラーページを読み捨てて早期リターン（ボディは読まない）
		return InstallLinkDefaults{}, fmt.Errorf("fetch %s: %s", sourceURL, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 64*1024))
	if err != nil {
		return InstallLinkDefaults{}, err
	}
	var d InstallLinkDefaults
	if err := json.Unmarshal(body, &d); err != nil {
		return InstallLinkDefaults{}, err
	}
	return d, nil
}

// sanitizeInstallLinkDefaults は https:// で始まらない値・URL としてパースできない
// 値を捨てる。画面はここで返った URL をそのまま新しいタブで開くため、
// javascript: 等の危険なスキームを通さないための最終防御。
func sanitizeInstallLinkDefaults(fetched InstallLinkDefaults) InstallLinkDefaults {
	sanitized := InstallLinkDefaults{}
	for id, raw := range fetched {
		if !strings.HasPrefix(raw, "https://") {
			continue
		}
		if _, err := url.Parse(raw); err != nil {
			continue
		}
		sanitized[id] = raw
	}
	return sanitized
}
