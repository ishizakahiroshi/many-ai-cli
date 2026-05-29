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
// RemoteHost は内部判定用（cloud alias か否か）で、JSON 出力には含めない。
type Model struct {
	ID         string `json:"id"`
	Label      string `json:"label,omitempty"`
	RemoteHost string `json:"-"`
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
	ollamaLocalCacheTTL = 60 * time.Second
	ollamaLocalURL      = "http://localhost:11434/api/tags"
)

// ollamaTagsResponse は `/api/tags` の最低限のレスポンス構造。
// remote_host が非空のエントリは cloud にプロキシされる pull 済み alias
// （例: `gemma4:31b-cloud` の `remote_host` は `https://ollama.com:443`）。
type ollamaTagsResponse struct {
	Models []struct {
		Name       string `json:"name"`
		Model      string `json:"model"`
		RemoteHost string `json:"remote_host"`
	} `json:"models"`
}

type ollamaTagsCacheEntry struct {
	models    []Model
	fetchedAt time.Time
	err       error
}

// modelsCache は Ollama Local `/api/tags` 取得結果を保持する。
// cloud 側のカタログは外部 fetch せず、ローカル daemon の remote_host で判定するため
// 専用のキャッシュは持たない。
type modelsCache struct {
	mu         sync.Mutex
	local      *ollamaTagsCacheEntry
	localFetch *ollamaTagsFetch
}

type ollamaTagsFetch struct {
	done chan struct{}
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
		out = append(out, Model{ID: id, Label: id, RemoteHost: strings.TrimSpace(m.RemoteHost)})
	}
	return out, nil
}

// getOllamaLocal はキャッシュ済みのローカル daemon モデル一覧を返す。
func (c *modelsCache) getOllamaLocal(force bool) (models []Model, fetchedAt time.Time, err error) {
	var inFlight *ollamaTagsFetch
	for {
		c.mu.Lock()
		entry := c.local
		if !force && entry != nil && time.Since(entry.fetchedAt) < ollamaLocalCacheTTL {
			c.mu.Unlock()
			return entry.models, entry.fetchedAt, entry.err
		}
		if c.localFetch == nil {
			inFlight = &ollamaTagsFetch{done: make(chan struct{})}
			c.localFetch = inFlight
			c.mu.Unlock()
			break
		}
		wait := c.localFetch.done
		c.mu.Unlock()
		<-wait
		force = false
	}
	models, err = fetchOllamaTags(ollamaLocalURL, 3*time.Second)
	newEntry := &ollamaTagsCacheEntry{
		models:    models,
		fetchedAt: time.Now(),
		err:       err,
	}
	c.mu.Lock()
	c.local = newEntry
	if c.localFetch == inFlight {
		close(c.localFetch.done)
		c.localFetch = nil
	}
	c.mu.Unlock()
	return models, newEntry.fetchedAt, err
}

// invalidate はローカルキャッシュを削除する。
func (c *modelsCache) invalidate() {
	c.mu.Lock()
	c.local = nil
	c.mu.Unlock()
}

// buildModelsResponse は Anthropic / OpenAI（GitHub fetch + 24h キャッシュ）と
// ローカル daemon の `/api/tags` を集約して /api/models のレスポンス body を作る。
//
// Ollama 系の Cloud / Local 分離は **ローカル daemon が返す `remote_host` フィールド**で判定する:
//   - `remote_host` 非空 → cloud にプロキシされる pull 済み alias → `[Ollama Cloud]` グループ
//   - `remote_host` 空    → 真のローカルモデル                     → `[Ollama Local]` グループ
//
// この設計により「daemon が知らない catalog 名」が選択肢に出ない（= 選んだら必ず呼べる）。
// 新規 cloud モデルの発見は UI 側の外部リンク（https://ollama.com/search?c=cloud）に任せる。
func buildModelsResponse(cache *modelsCache, remote *ttlCache[modelsDefaults], remoteSource string, localConfig []config.LocalModel, force bool) ModelsResponse {
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

	// Ollama Local daemon の /api/tags を 1 度だけ取得し、remote_host で 2 分割する
	localAll, localAt, localErr := cache.getOllamaLocal(force)
	var cloudFromDaemon, trulyLocal []Model
	for _, m := range localAll {
		if m.RemoteHost != "" {
			cloudFromDaemon = append(cloudFromDaemon, m)
		} else {
			trulyLocal = append(trulyLocal, m)
		}
	}

	// Ollama Cloud（pull 済み alias のみ）
	if len(cloudFromDaemon) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "Ollama Cloud",
			Provider: "",
			Route:    RouteOllama,
			Models:   cloudFromDaemon,
		})
		if localAt.After(newest) {
			newest = localAt
		}
	}

	// Ollama Local（真のローカル + config.yaml `local_models:` を merge）
	merged := mergeLocalModels(trulyLocal, localConfig)
	if localErr != nil && len(merged) == 0 && len(cloudFromDaemon) == 0 {
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
//
// ローカル daemon が返す全モデル（cloud alias 含む）を見るので、`gemma4:31b-cloud` のような
// pull 済みの cloud alias 名も Ollama route と判定される。
func collectOllamaModelIDs(cache *modelsCache, localConfig []config.LocalModel) map[string]bool {
	out := map[string]bool{}
	if cache != nil {
		cache.mu.Lock()
		local := cache.local
		cache.mu.Unlock()
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
