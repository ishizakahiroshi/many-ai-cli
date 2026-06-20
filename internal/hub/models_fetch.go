package hub

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
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
	ollamaLocalCacheTTL  = 60 * time.Second
	lmStudioCacheTTL     = 60 * time.Second
	openCodeModelsTTL    = 10 * time.Minute
	openCodeModelsNegTTL = 3 * time.Minute
)

// lmStudioModelsResponse は LM Studio `/v1/models` の OpenAI 互換レスポンス構造。
type lmStudioModelsResponse struct {
	Data []struct {
		ID     string `json:"id"`
		Object string `json:"object"`
	} `json:"data"`
}

type lmStudioModelsCacheEntry struct {
	models    []Model
	fetchedAt time.Time
	err       error
}

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
	models     []Model
	fetchedAt  time.Time
	err        error
	tagsURL    string
	generation uint64 // invalidate() で進む世代番号; finding #24 の force リフレッシュ判定に使用
}

type openCodeModelsCacheEntry struct {
	models    []Model
	fetchedAt time.Time
	err       error
}

// modelsCache は Ollama Local `/api/tags` および LM Studio `/v1/models` の取得結果を保持する。
// cloud 側のカタログは外部 fetch せず、ローカル daemon の remote_host で判定するため
// 専用のキャッシュは持たない。
type modelsCache struct {
	mu         sync.Mutex
	local      *ollamaTagsCacheEntry
	localFetch *ollamaTagsFetch
	lmStudio   *lmStudioModelsCacheEntry
	openCode   *openCodeModelsCacheEntry
	generation uint64 // invalidate() ごとに +1; force fetch が古い世代結果を受け取らないようにする
}

type ollamaTagsFetch struct {
	done     chan struct{}
	startGen uint64 // fetch 開始時の generation; 完了時に世代チェックして stale かどうか判定
}

func (c *modelsCache) getOpenCodeModels(force bool) ([]Model, time.Time, error) {
	if c == nil {
		return nil, time.Time{}, nil
	}
	c.mu.Lock()
	entry := c.openCode
	if entry != nil {
		age := time.Since(entry.fetchedAt)
		fresh := (!force && entry.err == nil && age < openCodeModelsTTL) || (!force && entry.err != nil && age < openCodeModelsNegTTL)
		if fresh {
			models := append([]Model(nil), entry.models...)
			fetchedAt := entry.fetchedAt
			err := entry.err
			c.mu.Unlock()
			return models, fetchedAt, err
		}
	}
	c.mu.Unlock()

	models, err := fetchOpenCodeModels(force)
	entry = &openCodeModelsCacheEntry{
		models:    append([]Model(nil), models...),
		fetchedAt: time.Now(),
		err:       err,
	}
	c.mu.Lock()
	c.openCode = entry
	c.mu.Unlock()
	return append([]Model(nil), models...), entry.fetchedAt, err
}

type openCodeVerboseModel struct {
	ID         string `json:"id"`
	ProviderID string `json:"providerID"`
	Name       string `json:"name"`
	Status     string `json:"status"`
}

func fetchOpenCodeModels(force bool) ([]Model, error) {
	bin, err := exec.LookPath("opencode")
	if err != nil {
		return nil, err
	}
	args := []string{"models", "opencode", "--verbose"}
	if force {
		args = append(args, "--refresh")
	}
	cmd := exec.Command(bin, args...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("opencode models: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return parseOpenCodeModelsOutput(out)
}

func parseOpenCodeModelsOutput(out []byte) ([]Model, error) {
	lines := bytes.Split(bytes.ReplaceAll(out, []byte("\r\n"), []byte("\n")), []byte("\n"))
	models := make([]Model, 0, len(lines))
	seen := map[string]bool{}
	var pendingID string

	flushPending := func() {
		if pendingID == "" {
			return
		}
		fullID := pendingID
		if !strings.Contains(fullID, "/") {
			fullID = "opencode/" + fullID
		}
		if !seen[fullID] {
			models = append(models, Model{ID: fullID, Label: humanizeOpenCodeModelLabel(pendingID)})
			seen[fullID] = true
		}
		pendingID = ""
	}

	for i := 0; i < len(lines); i++ {
		line := strings.TrimSpace(string(lines[i]))
		if line == "" {
			continue
		}
		if strings.HasPrefix(line, "opencode/") {
			flushPending()
			pendingID = strings.TrimSpace(strings.TrimPrefix(line, "opencode/"))
			continue
		}
		if !strings.HasPrefix(line, "{") {
			continue
		}
		objLines := [][]byte{[]byte(line)}
		depth := strings.Count(line, "{") - strings.Count(line, "}")
		for depth > 0 && i+1 < len(lines) {
			i++
			next := lines[i]
			nextLine := bytes.TrimSpace(next)
			objLines = append(objLines, nextLine)
			depth += strings.Count(string(nextLine), "{") - strings.Count(string(nextLine), "}")
		}
		var info openCodeVerboseModel
		if err := json.Unmarshal(bytes.Join(objLines, []byte("\n")), &info); err != nil {
			flushPending()
			continue
		}
		fullID := strings.TrimSpace(info.ID)
		if fullID == "" {
			fullID = pendingID
		}
		if fullID == "" {
			continue
		}
		if !strings.Contains(fullID, "/") {
			fullID = "opencode/" + fullID
		}
		if seen[fullID] {
			pendingID = ""
			continue
		}
		if strings.TrimSpace(info.Status) != "" && !strings.EqualFold(strings.TrimSpace(info.Status), "active") {
			pendingID = ""
			continue
		}
		label := strings.TrimSpace(info.Name)
		if label == "" {
			label = humanizeOpenCodeModelLabel(pendingID)
		}
		models = append(models, Model{ID: fullID, Label: label})
		seen[fullID] = true
		pendingID = ""
	}
	flushPending()
	return models, nil
}

func humanizeOpenCodeModelLabel(id string) string {
	id = strings.TrimSpace(id)
	id = strings.TrimPrefix(id, "opencode/")
	if id == "" {
		return "OpenCode"
	}
	parts := strings.FieldsFunc(id, func(r rune) bool {
		return r == '-' || r == '_' || r == '/'
	})
	for i, p := range parts {
		switch {
		case p == "":
			continue
		case len(p) == 1:
			parts[i] = strings.ToUpper(p)
		default:
			parts[i] = strings.ToUpper(p[:1]) + strings.ToLower(p[1:])
		}
	}
	return strings.Join(parts, " ")
}

// Anthropic / OpenAI / Copilot / Cursor Agent のモデル一覧は GitHub の
// resources/models/defaults.json から 24h TTL で取得する。
// fetch 失敗時は空配列に倒し、静的 fallback は持たない。

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

func ollamaTagsURL(baseURL string) string {
	return config.EffectiveOllamaBaseURL(baseURL) + "/api/tags"
}

// fetchLMStudioModels は LM Studio `/v1/models`（OpenAI 互換）を取得して models 列に変換する。
func fetchLMStudioModels(url string, timeout time.Duration) ([]Model, error) {
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
	var parsed lmStudioModelsResponse
	if err := json.Unmarshal(body, &parsed); err != nil {
		return nil, fmt.Errorf("parse %s: %w", url, err)
	}
	out := make([]Model, 0, len(parsed.Data))
	seen := map[string]bool{}
	for _, m := range parsed.Data {
		id := strings.TrimSpace(m.ID)
		if id == "" || seen[id] {
			continue
		}
		seen[id] = true
		out = append(out, Model{ID: id, Label: id})
	}
	return out, nil
}

func lmStudioModelsURL(baseURL string) string {
	return config.EffectiveLMStudioBaseURL(baseURL) + "/v1/models"
}

// getOllamaLocal はキャッシュ済みのローカル daemon モデル一覧を返す。
// force=true 時は TTL を無視し強制リフレッシュする。finding #24 対策として
// invalidate() で進んだ世代より前の in-flight 結果を受け取らない。
func (c *modelsCache) getOllamaLocal(force bool, tagsURL string) (models []Model, fetchedAt time.Time, err error) {
	tagsURL = strings.TrimSpace(tagsURL)
	if tagsURL == "" {
		tagsURL = ollamaTagsURL("")
	}
	var inFlight *ollamaTagsFetch
	for {
		c.mu.Lock()
		entry := c.local
		curGen := c.generation
		// force 時は世代が最新の fresh entry のみ受け入れる（stale 世代を拒否）。
		// entryFresh は force 用の世代チェック（entry.generation >= curGen）を内包する
		// ため、force でも「invalidate 後に完了済みの現世代 fresh entry」は再 fetch せず
		// 受け入れる（rapid force 連打での thundering・不要な Ollama 接続を防ぐ）。
		entryFresh := entry != nil && entry.tagsURL == tagsURL && (!force || entry.generation >= curGen) && time.Since(entry.fetchedAt) < ollamaLocalCacheTTL
		if entryFresh {
			c.mu.Unlock()
			return entry.models, entry.fetchedAt, entry.err
		}
		if c.localFetch == nil {
			inFlight = &ollamaTagsFetch{done: make(chan struct{}), startGen: curGen}
			c.localFetch = inFlight
			c.mu.Unlock()
			break
		}
		// 既に in-flight の fetch がある。待機後に世代チェック。
		myStartGen := curGen
		wait := c.localFetch.done
		c.mu.Unlock()
		<-wait
		// 待機後: fresh かつ世代が自分の startGen 以上なら受け入れる
		c.mu.Lock()
		entry = c.local
		if entry != nil && entry.tagsURL == tagsURL && entry.generation >= myStartGen && time.Since(entry.fetchedAt) < ollamaLocalCacheTTL {
			c.mu.Unlock()
			return entry.models, entry.fetchedAt, entry.err
		}
		c.mu.Unlock()
		// 世代が古い結果は受け入れず再試行（force は既に消費）
		force = false
	}
	models, err = fetchOllamaTags(tagsURL, 3*time.Second)
	c.mu.Lock()
	newGen := inFlight.startGen
	if c.generation > newGen {
		newGen = c.generation
	}
	newEntry := &ollamaTagsCacheEntry{
		models:     models,
		fetchedAt:  time.Now(),
		err:        err,
		tagsURL:    tagsURL,
		generation: newGen,
	}
	c.local = newEntry
	if c.localFetch == inFlight {
		close(c.localFetch.done)
		c.localFetch = nil
	}
	c.mu.Unlock()
	return models, newEntry.fetchedAt, err
}

// invalidate はローカルキャッシュを削除し世代を進める（finding #24: force リフレッシュが
// invalidate 前の stale in-flight 結果で満たされないようにする）。
func (c *modelsCache) invalidate() {
	c.mu.Lock()
	c.local = nil
	c.lmStudio = nil
	c.generation++
	c.mu.Unlock()
}

// getLMStudioModels は LM Studio `/v1/models` の取得結果をキャッシュ付きで返す。
// force=true 時は TTL を無視して再取得する。
func (c *modelsCache) getLMStudioModels(force bool, modelsURL string) ([]Model, time.Time, error) {
	if c == nil {
		return nil, time.Time{}, nil
	}
	c.mu.Lock()
	entry := c.lmStudio
	if entry != nil {
		age := time.Since(entry.fetchedAt)
		if !force && age < lmStudioCacheTTL {
			models := append([]Model(nil), entry.models...)
			fetchedAt := entry.fetchedAt
			err := entry.err
			c.mu.Unlock()
			return models, fetchedAt, err
		}
	}
	c.mu.Unlock()

	models, err := fetchLMStudioModels(modelsURL, 3*time.Second)
	entry = &lmStudioModelsCacheEntry{
		models:    append([]Model(nil), models...),
		fetchedAt: time.Now(),
		err:       err,
	}
	c.mu.Lock()
	c.lmStudio = entry
	c.mu.Unlock()
	return append([]Model(nil), models...), entry.fetchedAt, err
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
func buildModelsResponse(cache *modelsCache, remote *ttlCache[modelsDefaults], remoteSource string, localConfig []config.LocalModel, ollamaBaseURL string, lmStudioBaseURL string, force bool) ModelsResponse {
	if force && remote != nil {
		remote.invalidate()
	}
	defaults := modelsDefaults{}
	if remote != nil {
		defaults = remote.get(remoteSource)
	}

	resp := ModelsResponse{
		Sources: map[string]string{
			"anthropic":           remoteSource,
			"openai":              remoteSource,
			"opencode":            "opencode models opencode --verbose",
			"ollama_local":        ollamaTagsURL(ollamaBaseURL),
			"lm_studio":           lmStudioModelsURL(lmStudioBaseURL),
			"local_models_config": "~/.many-ai-cli/config.yaml#local_models",
		},
	}
	var newest time.Time

	// Anthropic（GitHub fetch only）
	resp.Groups = append(resp.Groups, ModelGroup{
		Label:    "Anthropic",
		Provider: "claude",
		Route:    RouteAnthropic,
		Models:   append([]Model{}, defaults.Anthropic...),
	})

	// OpenAI（GitHub fetch only）
	resp.Groups = append(resp.Groups, ModelGroup{
		Label:    "OpenAI",
		Provider: "codex",
		Route:    RouteOpenAI,
		Models:   append([]Model{}, defaults.OpenAI...),
	})

	// GitHub Copilot（GitHub fetch only）
	// Route は空: copilot は env 注入せず `copilot --model <id>` へ素通しする。
	if len(defaults.Copilot) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "GitHub Copilot",
			Provider: "copilot",
			Route:    "",
			Models:   append([]Model{}, defaults.Copilot...),
		})
	}

	// Cursor Agent（GitHub fetch only）
	// Route は空: cursor-agent は env 注入せず `cursor-agent --model <id>` へ素通しする。
	if len(defaults.CursorAgent) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "Cursor Agent",
			Provider: "cursor-agent",
			Route:    "",
			Models:   append([]Model{}, defaults.CursorAgent...),
		})
	}

	// Grok Build（GitHub fetch only）
	// Route は空: grok は env 注入せず `grok --model <id>` へ素通しする。
	if len(defaults.Grok) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "Grok",
			Provider: "grok",
			Route:    "",
			Models:   append([]Model{}, defaults.Grok...),
		})
	}

	// OpenCode は CLI から動的取得する。固定候補は持たない。
	if models, fetchedAt, _ := cache.getOpenCodeModels(force); len(models) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "OpenCode",
			Provider: "opencode",
			Route:    "",
			Models:   append([]Model{}, models...),
		})
		if fetchedAt.After(newest) {
			newest = fetchedAt
		}
	}

	// Ollama Local daemon の /api/tags を 1 度だけ取得し、remote_host で 2 分割する
	localAll, localAt, localErr := cache.getOllamaLocal(force, ollamaTagsURL(ollamaBaseURL))
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

	// LM Studio /v1/models を取得する
	lmStudioModels, lmStudioAt, lmStudioErr := cache.getLMStudioModels(force, lmStudioModelsURL(lmStudioBaseURL))
	if lmStudioErr != nil {
		resp.Warnings = append(resp.Warnings, "lm_studio_unreachable")
	} else if len(lmStudioModels) > 0 {
		resp.Groups = append(resp.Groups, ModelGroup{
			Label:    "LM Studio",
			Provider: "",
			Route:    RouteLMStudio,
			Models:   lmStudioModels,
		})
		if lmStudioAt.After(newest) {
			newest = lmStudioAt
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

// collectLMStudioModelIDs は cache から既知の LM Studio モデル ID 集合を作る。
// spawn API での route 推定に使う。stale でも返す（spawn ホットパスを止めない）。
func collectLMStudioModelIDs(cache *modelsCache) map[string]bool {
	out := map[string]bool{}
	if cache != nil {
		cache.mu.Lock()
		lms := cache.lmStudio
		cache.mu.Unlock()
		if lms != nil {
			for _, m := range lms.models {
				out[m.ID] = true
			}
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
