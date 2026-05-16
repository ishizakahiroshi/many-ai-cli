package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"any-ai-cli/internal/config"
)

// Model は /api/models response の 1 件分。
type Model struct {
	ID    string `json:"id"`
	Label string `json:"label,omitempty"`
}

// ModelGroup は同じ route / provider に属するモデル群。
//
// Provider:
//   - "claude" / "codex" は当該 provider 専用（他では非表示）
//   - "" は両 provider で表示可（Ollama Cloud / Local）
type ModelGroup struct {
	Label    string  `json:"label"`
	Provider string  `json:"provider,omitempty"`
	Route    string  `json:"route"`
	Models   []Model `json:"models"`
}

// ModelsResponse は /api/models のレスポンス。
type ModelsResponse struct {
	Groups   []ModelGroup      `json:"groups"`
	CachedAt string            `json:"cached_at,omitempty"`
	Sources  map[string]string `json:"sources,omitempty"`
	Warnings []string          `json:"warnings,omitempty"`
}

const (
	ollamaCloudCacheTTL = 24 * time.Hour
	ollamaLocalCacheTTL = 60 * time.Second
	ollamaCloudURL      = "https://ollama.com/api/tags"
	ollamaLocalURL      = "http://localhost:11434/api/tags"
)

// ollamaTagsResponse は `/api/tags` の最低限のレスポンス構造。
type ollamaTagsResponse struct {
	Models []struct {
		Name  string `json:"name"`
		Model string `json:"model"`
	} `json:"models"`
}

type ollamaTagsCacheEntry struct {
	models    []Model
	fetchedAt time.Time
	err       error
}

// modelsCache は Ollama Cloud / Local の `/api/tags` 取得結果を保持する。
type modelsCache struct {
	mu    sync.Mutex
	cloud *ollamaTagsCacheEntry
	local *ollamaTagsCacheEntry
}

// Anthropic / OpenAI のモデル一覧は GitHub の resources/models/defaults.json から
// 24h TTL で取得する（fetch 失敗時は models_remote_fetch.go の hardcodedModelsDefaults にフォールバック）。
// slash-commands / approval-patterns / usage-links と同じ仕組みで、リビルド不要・トークン消費 0 で更新できる。

// fetchOllamaTags は指定 URL から `/api/tags` を取得して models 列に変換する。
func fetchOllamaTags(url string, timeout time.Duration) ([]Model, error) {
	client := &http.Client{Timeout: timeout}
	resp, err := client.Get(url)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("fetch %s: %s", url, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 2<<20))
	if err != nil {
		return nil, err
	}
	var parsed ollamaTagsResponse
	if err := json.Unmarshal(body, &parsed); err != nil {
		return nil, fmt.Errorf("parse %s: %w", url, err)
	}
	out := make([]Model, 0, len(parsed.Models))
	seen := map[string]bool{}
	for _, m := range parsed.Models {
		id := strings.TrimSpace(m.Name)
		if id == "" {
			id = strings.TrimSpace(m.Model)
		}
		if id == "" || seen[id] {
			continue
		}
		seen[id] = true
		out = append(out, Model{ID: id, Label: id})
	}
	return out, nil
}

// getOllamaCloud はキャッシュ済みの cloud モデル一覧を返す。期限切れ or force 時に refresh。
func (c *modelsCache) getOllamaCloud(force bool) (models []Model, fetchedAt time.Time, err error) {
	c.mu.Lock()
	entry := c.cloud
	c.mu.Unlock()
	if !force && entry != nil && time.Since(entry.fetchedAt) < ollamaCloudCacheTTL {
		return entry.models, entry.fetchedAt, entry.err
	}
	models, err = fetchOllamaTags(ollamaCloudURL, 15*time.Second)
	newEntry := &ollamaTagsCacheEntry{
		models:    models,
		fetchedAt: time.Now(),
		err:       err,
	}
	c.mu.Lock()
	c.cloud = newEntry
	c.mu.Unlock()
	return models, newEntry.fetchedAt, err
}

// getOllamaLocal はキャッシュ済みのローカル daemon モデル一覧を返す。
func (c *modelsCache) getOllamaLocal(force bool) (models []Model, fetchedAt time.Time, err error) {
	c.mu.Lock()
	entry := c.local
	c.mu.Unlock()
	if !force && entry != nil && time.Since(entry.fetchedAt) < ollamaLocalCacheTTL {
		return entry.models, entry.fetchedAt, entry.err
	}
	models, err = fetchOllamaTags(ollamaLocalURL, 3*time.Second)
	newEntry := &ollamaTagsCacheEntry{
		models:    models,
		fetchedAt: time.Now(),
		err:       err,
	}
	c.mu.Lock()
	c.local = newEntry
	c.mu.Unlock()
	return models, newEntry.fetchedAt, err
}

// invalidate は cloud / local 両キャッシュを削除する。
func (c *modelsCache) invalidate() {
	c.mu.Lock()
	c.cloud = nil
	c.local = nil
	c.mu.Unlock()
}

// buildModelsResponse は Anthropic / OpenAI（GitHub fetch + 24h キャッシュ）/ Ollama Cloud / Local を
// 集約して /api/models のレスポンス body を作る。
func buildModelsResponse(cache *modelsCache, remote *modelsRemoteCache, remoteSource string, localConfig []config.LocalModel, force bool) ModelsResponse {
	if force && remote != nil {
		remote.invalidate()
	}
	defaults := hardcodedModelsDefaults
	if remote != nil {
		defaults = remote.get(remoteSource)
	}

	resp := ModelsResponse{
		Sources: map[string]string{
			"anthropic":           remoteSource,
			"openai":              remoteSource,
			"ollama_cloud":        ollamaCloudURL,
			"ollama_local":        ollamaLocalURL,
			"local_models_config": "~/.any-ai-cli/config.yaml#local_models",
		},
	}
	var newest time.Time

	// Anthropic（GitHub fetch / fallback ハードコード）
	resp.Groups = append(resp.Groups, ModelGroup{
		Label:    "Anthropic",
		Provider: "claude",
		Route:    RouteAnthropic,
		Models:   append([]Model{}, defaults.Anthropic...),
	})

	// OpenAI（GitHub fetch / fallback ハードコード）
	resp.Groups = append(resp.Groups, ModelGroup{
		Label:    "OpenAI",
		Provider: "codex",
		Route:    RouteOpenAI,
		Models:   append([]Model{}, defaults.OpenAI...),
	})

	// Ollama Cloud
	cloudModels, cloudAt, cloudErr := cache.getOllamaCloud(force)
	if cloudErr != nil {
		resp.Warnings = append(resp.Warnings, "ollama_cloud_fetch_failed")
	} else if len(cloudModels) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "Ollama Cloud",
			Provider: "",
			Route:    RouteOllama,
			Models:   cloudModels,
		})
		if cloudAt.After(newest) {
			newest = cloudAt
		}
	}

	// Ollama Local（daemon + config.yaml の merge）
	localModels, localAt, localErr := cache.getOllamaLocal(force)
	merged := mergeLocalModels(localModels, localConfig)
	if localErr != nil && len(merged) == 0 {
		// daemon オフライン + config 由来も無い場合のみ warnings + 省略
		resp.Warnings = append(resp.Warnings, "ollama_daemon_unreachable")
	} else if len(merged) > 0 {
		if localErr != nil {
			// daemon オフラインだが config に手書きがあるケース
			resp.Warnings = append(resp.Warnings, "ollama_daemon_unreachable")
		}
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "Ollama Local",
			Provider: "",
			Route:    RouteOllama,
			Models:   merged,
		})
		if localAt.After(newest) {
			newest = localAt
		}
	}

	if !newest.IsZero() {
		resp.CachedAt = newest.UTC().Format(time.RFC3339)
	}
	return resp
}

// mergeLocalModels は daemon `/api/tags` 結果と config.yaml `local_models` を merge する。
// 同 ID が両方にある場合 config 側の label を優先する。
func mergeLocalModels(daemon []Model, configList []config.LocalModel) []Model {
	idx := map[string]int{}
	out := make([]Model, 0, len(daemon)+len(configList))
	for _, m := range daemon {
		idx[m.ID] = len(out)
		out = append(out, m)
	}
	for _, lm := range configList {
		id := strings.TrimSpace(lm.ID)
		if id == "" {
			continue
		}
		label := strings.TrimSpace(lm.Label)
		if label == "" {
			label = id
		}
		if i, ok := idx[id]; ok {
			out[i].Label = label
		} else {
			idx[id] = len(out)
			out = append(out, Model{ID: id, Label: label})
		}
	}
	return out
}

// collectOllamaModelIDs は cache から既知の Ollama Cloud / Local の ID 集合を作る。
// spawn API での route 推定に使う。force=false で stale でも返す（spawn ホットパスを止めない）。
func collectOllamaModelIDs(cache *modelsCache, localConfig []config.LocalModel) map[string]bool {
	out := map[string]bool{}
	if cache != nil {
		cache.mu.Lock()
		cloud := cache.cloud
		local := cache.local
		cache.mu.Unlock()
		if cloud != nil {
			for _, m := range cloud.models {
				out[m.ID] = true
			}
		}
		if local != nil {
			for _, m := range local.models {
				out[m.ID] = true
			}
		}
	}
	for _, lm := range localConfig {
		id := strings.TrimSpace(lm.ID)
		if id != "" {
			out[id] = true
		}
	}
	return out
}
