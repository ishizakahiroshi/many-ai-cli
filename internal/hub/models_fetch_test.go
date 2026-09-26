package hub

import (
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestParseOpenCodeModelsOutput(t *testing.T) {
	out := strings.Join([]string{
		"opencode/big-pickle",
		"{",
		`  "id": "big-pickle",`,
		`  "providerID": "opencode",`,
		`  "name": "Big Pickle",`,
		`  "status": "active"`,
		"}",
		"opencode/nemotron-3-ultra-free",
		"{",
		`  "id": "nemotron-3-ultra-free",`,
		`  "providerID": "opencode",`,
		`  "name": "Nemotron 3 Ultra Free",`,
		`  "status": "inactive"`,
		"}",
		"opencode/deepseek-v4-flash-free",
		"{",
		`  "id": "deepseek-v4-flash-free",`,
		`  "providerID": "opencode",`,
		`  "name": "DeepSeek V4 Flash Free",`,
		`  "status": "active"`,
		"}",
	}, "\n")

	models, err := parseOpenCodeModelsOutput([]byte(out))
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != 2 {
		t.Fatalf("len(models) = %d, want 2 (%+v)", len(models), models)
	}
	if models[0].ID != "opencode/big-pickle" || models[0].Label != "Big Pickle" {
		t.Fatalf("first model = %+v", models[0])
	}
	if models[1].ID != "opencode/deepseek-v4-flash-free" || models[1].Label != "DeepSeek V4 Flash Free" {
		t.Fatalf("second model = %+v", models[1])
	}
}

func TestParseCursorAgentModelsOutput(t *testing.T) {
	out := []byte(strings.Join([]string{
		"\x1b[36mauto\x1b[0m - Auto (default)",
		"composer-2.5 - Composer 2.5 (current)",
		"gpt-5.6-sol-high - GPT-5.6 Sol 1M High",
		"gpt-5.6-sol-high - Duplicate label is ignored",
		"--list-models",
		"not a model entry",
	}, "\r\n"))

	models, err := parseCursorAgentModelsOutput(out)
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != 3 {
		t.Fatalf("len(models) = %d, want 3 (%+v)", len(models), models)
	}
	if models[0].ID != "auto" || models[0].Label != "Auto (default)" {
		t.Fatalf("first model = %+v", models[0])
	}
	if models[1].ID != "composer-2.5" || models[1].Label != "Composer 2.5 (current)" {
		t.Fatalf("second model = %+v", models[1])
	}
	if models[2].ID != "gpt-5.6-sol-high" || models[2].Label != "GPT-5.6 Sol 1M High" {
		t.Fatalf("third model = %+v", models[2])
	}
}

func TestParseGrokModelsOutput(t *testing.T) {
	out := []byte(strings.Join([]string{
		"\x1b[36mgrok-4.6\x1b[0m",
		"grok-4.5",
		"other-provider-model",
		"grok-4.6",
	}, "\n"))

	models, err := parseGrokModelsOutput(out)
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != 2 {
		t.Fatalf("len(models) = %d, want 2 (%+v)", len(models), models)
	}
	if models[0].ID != "grok-4.6" || models[0].Label != "Grok 4.6" {
		t.Fatalf("first model = %+v", models[0])
	}
	if models[1].ID != "grok-4.5" || models[1].Label != "Grok 4.5" {
		t.Fatalf("second model = %+v", models[1])
	}
}

func TestNativeCLIModelsCacheHonorsTTLForceAndInvalidate(t *testing.T) {
	calls := 0
	cache := &modelsCache{
		nativeCLIModelLoaders: map[string]func(bool) ([]Model, error){
			"grok": func(bool) ([]Model, error) {
				calls++
				return []Model{{ID: "grok-4.6"}}, nil
			},
		},
	}

	for _, force := range []bool{false, false, true} {
		models, fetchedAt, err := cache.getNativeCLIModels("grok", force)
		if err != nil {
			t.Fatal(err)
		}
		if len(models) != 1 || models[0].ID != "grok-4.6" || fetchedAt.IsZero() {
			t.Fatalf("models/fetchedAt = %+v / %v", models, fetchedAt)
		}
	}
	if calls != 2 {
		t.Fatalf("loader calls after cache hit and force refresh = %d, want 2", calls)
	}

	cache.invalidate()
	if _, _, err := cache.getNativeCLIModels("grok", false); err != nil {
		t.Fatal(err)
	}
	if calls != 3 {
		t.Fatalf("loader calls after invalidate = %d, want 3", calls)
	}
}

func TestBuildModelsResponsePrefersNativeCatalogAndFallsBack(t *testing.T) {
	fallback := modelsDefaults{
		"cursor-agent":    []Model{{ID: "cursor-static"}},
		"grok":            []Model{{ID: "grok-static"}},
		"future-provider": []Model{{ID: "future-model"}},
	}

	for _, tc := range []struct {
		name         string
		nativeWorks  bool
		wantCursorID string
		wantGrokID   string
	}{
		{name: "native lists win", nativeWorks: true, wantCursorID: "cursor-live", wantGrokID: "grok-live"},
		{name: "remote catalog is fallback", nativeWorks: false, wantCursorID: "cursor-static", wantGrokID: "grok-static"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			now := time.Now()
			cache := &modelsCache{
				local: &ollamaTagsCacheEntry{fetchedAt: now, tagsURL: ollamaTagsURL("")},
				lmStudio: &lmStudioModelsCacheEntry{
					fetchedAt:    now,
					modelsURL:    lmStudioModelsURL(""),
					allowPrivate: false,
				},
				openCodeLoader: func(bool) ([]Model, error) { return nil, nil },
				nativeCLIModelLoaders: map[string]func(bool) ([]Model, error){
					"cursor-agent": func(bool) ([]Model, error) {
						if !tc.nativeWorks {
							return nil, errors.New("native model list unavailable")
						}
						return []Model{{ID: "cursor-live"}}, nil
					},
					"grok": func(bool) ([]Model, error) {
						if !tc.nativeWorks {
							return nil, errors.New("native model list unavailable")
						}
						return []Model{{ID: "grok-live"}}, nil
					},
				},
			}
			remote := &ttlCache[modelsDefaults]{
				ttl: time.Hour,
				fetch: func(string) (modelsDefaults, error) {
					return fallback, nil
				},
			}

			resp := buildModelsResponse(cache, remote, "test-source", nil, "", "", false)
			got := map[string]string{}
			for _, group := range resp.Groups {
				if len(group.Models) > 0 {
					got[group.Provider] = group.Models[0].ID
				}
			}
			if got["cursor-agent"] != tc.wantCursorID || got["grok"] != tc.wantGrokID {
				t.Fatalf("provider model IDs = %+v, want cursor-agent=%q and grok=%q", got, tc.wantCursorID, tc.wantGrokID)
			}
			wantCursorSource := "cursor-agent --list-models"
			wantGrokSource := "grok models"
			if !tc.nativeWorks {
				wantCursorSource += " (fallback: test-source)"
				wantGrokSource += " (fallback: test-source)"
			}
			if resp.Sources["cursor-agent"] != wantCursorSource || resp.Sources["grok"] != wantGrokSource {
				t.Fatalf("native source status = cursor %q / grok %q", resp.Sources["cursor-agent"], resp.Sources["grok"])
			}
			if resp.Sources["future-provider"] != "test-source" {
				t.Fatalf("dynamic catalog source = %q, want test-source", resp.Sources["future-provider"])
			}
		})
	}
}

func TestBuildModelsResponseUsesConfiguredOllamaBaseURL(t *testing.T) {
	baseURL := "http://192.0.2.1:11434"
	cache := &modelsCache{
		local: &ollamaTagsCacheEntry{
			models:    []Model{{ID: "qwen3:8b", Label: "qwen3:8b"}},
			fetchedAt: time.Now(),
			tagsURL:   ollamaTagsURL(baseURL),
		},
	}
	resp := buildModelsResponse(cache, nil, "", nil, baseURL, "", false)
	if got := resp.Sources["ollama_local"]; got != "http://192.0.2.1:11434/api/tags" {
		t.Fatalf("ollama source = %q, want configured /api/tags URL", got)
	}
}

func TestOpenCodeModelsSingleFlight(t *testing.T) {
	c := &modelsCache{}
	started := make(chan struct{})
	waiterObserved := make(chan struct{})
	release := make(chan struct{})
	var mu sync.Mutex
	calls := 0
	c.openCodeLoader = func(bool) ([]Model, error) {
		mu.Lock()
		calls++
		mu.Unlock()
		close(started)
		<-release
		return []Model{{ID: "opencode/test"}}, nil
	}
	c.openCodeWaitHook = func() { close(waiterObserved) }

	var wg sync.WaitGroup
	results := make([][]Model, 2)
	errs := make([]error, 2)
	for i := range results {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			results[i], _, errs[i] = c.getOpenCodeModels(true)
		}(i)
	}
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("OpenCode model fetch did not start")
	}
	select {
	case <-waiterObserved:
	case <-time.After(2 * time.Second):
		t.Fatal("second OpenCode model request did not join the in-flight fetch")
	}
	close(release)
	wg.Wait()

	mu.Lock()
	gotCalls := calls
	mu.Unlock()
	if gotCalls != 1 {
		t.Fatalf("OpenCode loader calls = %d, want 1", gotCalls)
	}
	for i, err := range errs {
		if err != nil {
			t.Fatalf("request %d error: %v", i, err)
		}
		if len(results[i]) != 1 || results[i][0].ID != "opencode/test" {
			t.Fatalf("request %d models = %+v", i, results[i])
		}
	}
}

func newNVIDIANIMTestModelsCache() *modelsCache {
	now := time.Now()
	return &modelsCache{
		local: &ollamaTagsCacheEntry{fetchedAt: now, tagsURL: ollamaTagsURL("")},
		lmStudio: &lmStudioModelsCacheEntry{
			fetchedAt:    now,
			modelsURL:    lmStudioModelsURL(""),
			allowPrivate: false,
		},
		openCodeLoader: func(bool) ([]Model, error) {
			return []Model{{ID: "opencode/test", Label: "OpenCode Test"}}, nil
		},
		nativeCLIModelLoaders: map[string]func(bool) ([]Model, error){
			"cursor-agent": func(bool) ([]Model, error) { return nil, nil },
			"grok":         func(bool) ([]Model, error) { return nil, nil },
		},
	}
}

func TestBuildModelsResponseAddsHostedTrialNVIDIANIMGroup(t *testing.T) {
	const apiKey = "synthetic-nvidia-key"
	cache := newNVIDIANIMTestModelsCache()
	calls := 0
	cache.nvidiaNIMLoader = func(gotKey string) ([]string, error) {
		calls++
		if gotKey != apiKey {
			t.Errorf("catalog API key = %q, want injected key", gotKey)
		}
		return []string{"moonshotai/kimi-k3"}, nil
	}
	resp := buildModelsResponseWithNVIDIANIM(cache, nil, "", nil, "", "", false, nil, nvidiaNIMCatalogOptions{
		enabled: true,
		apiKey:  apiKey,
	})
	if calls != 1 {
		t.Fatalf("catalog fetch calls = %d, want 1", calls)
	}
	var got *ModelGroup
	for i := range resp.Groups {
		if resp.Groups[i].Route == RouteNVIDIANIM {
			got = &resp.Groups[i]
			break
		}
	}
	if got == nil {
		t.Fatalf("NVIDIA NIM group missing: %+v", resp.Groups)
	}
	if got.Provider != "opencode" || !got.Hosted || !got.Trial {
		t.Fatalf("NVIDIA NIM group metadata = %+v", got)
	}
	if len(got.Models) != 1 || got.Models[0].ID != "nvidia/moonshotai/kimi-k3" || got.Models[0].Label != "moonshotai/kimi-k3" {
		t.Fatalf("NVIDIA NIM models = %+v", got.Models)
	}
	encoded, err := json.Marshal(resp)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), apiKey) {
		t.Fatal("serialized model response contains the API key")
	}
}

func TestBuildModelsResponseSkipsNVIDIANIMWhenDisabledOrKeyMissing(t *testing.T) {
	for _, tc := range []struct {
		name    string
		options nvidiaNIMCatalogOptions
	}{
		{name: "disabled", options: nvidiaNIMCatalogOptions{apiKey: "test-key"}},
		{name: "missing key", options: nvidiaNIMCatalogOptions{enabled: true}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cache := newNVIDIANIMTestModelsCache()
			cache.nvidiaNIMLoader = func(string) ([]string, error) {
				t.Fatal("NVIDIA NIM loader called while disabled or unconfigured")
				return nil, nil
			}
			resp := buildModelsResponseWithNVIDIANIM(cache, nil, "", nil, "", "", false, nil, tc.options)
			for _, group := range resp.Groups {
				if group.Route == RouteNVIDIANIM {
					t.Fatal("unexpected NVIDIA NIM group")
				}
			}
		})
	}
}

func TestBuildModelsResponseIsolatesNVIDIANIMCatalogFailure(t *testing.T) {
	cache := newNVIDIANIMTestModelsCache()
	cache.nvidiaNIMLoader = func(string) ([]string, error) {
		return nil, errors.New("synthetic failure")
	}
	resp := buildModelsResponseWithNVIDIANIM(cache, nil, "", nil, "", "", false, nil, nvidiaNIMCatalogOptions{
		enabled: true,
		apiKey:  "synthetic-key",
	})
	if !slicesContain(resp.Warnings, "nvidia_nim_unreachable") {
		t.Fatalf("warnings = %v", resp.Warnings)
	}
	for _, group := range resp.Groups {
		if group.Route == RouteNVIDIANIM {
			t.Fatal("failed NVIDIA NIM catalog unexpectedly produced a group")
		}
		if group.Provider == "opencode" && len(group.Models) > 0 {
			return
		}
	}
	t.Fatal("NVIDIA NIM failure hid the existing OpenCode model group")
}

func TestNVIDIANIMModelsCacheTTLForceAndInvalidate(t *testing.T) {
	calls := 0
	cache := &modelsCache{nvidiaNIMLoader: func(string) ([]string, error) {
		calls++
		return []string{"vendor/model"}, nil
	}}
	for _, force := range []bool{false, false, true} {
		if _, _, err := cache.getNVIDIANIMModels("test-key", force); err != nil {
			t.Fatal(err)
		}
	}
	if calls != 2 {
		t.Fatalf("loader calls after cache hit and forced refresh = %d, want 2", calls)
	}
	cache.mu.Lock()
	cache.nvidiaNIM.fetchedAt = time.Now().Add(-nvidiaNIMModelsTTL - time.Second)
	cache.mu.Unlock()
	if _, _, err := cache.getNVIDIANIMModels("test-key", false); err != nil {
		t.Fatal(err)
	}
	cache.invalidate()
	if _, _, err := cache.getNVIDIANIMModels("test-key", false); err != nil {
		t.Fatal(err)
	}
	if calls != 4 {
		t.Fatalf("loader calls after expiry and invalidation = %d, want 4", calls)
	}
}

func TestNVIDIANIMModelsFailureUsesShortNegativeTTL(t *testing.T) {
	calls := 0
	cache := &modelsCache{nvidiaNIMLoader: func(string) ([]string, error) {
		calls++
		return nil, errors.New("temporary catalog failure")
	}}
	for range 2 {
		if _, _, err := cache.getNVIDIANIMModels("test-key", false); err == nil {
			t.Fatal("expected catalog failure")
		}
	}
	if calls != 1 {
		t.Fatalf("loader calls during negative TTL = %d, want 1", calls)
	}
	cache.mu.Lock()
	cache.nvidiaNIM.fetchedAt = time.Now().Add(-nvidiaNIMModelsNegTTL - time.Second)
	cache.mu.Unlock()
	if _, _, err := cache.getNVIDIANIMModels("test-key", false); err == nil {
		t.Fatal("expected catalog failure after negative TTL expired")
	}
	if calls != 2 {
		t.Fatalf("loader calls after negative TTL expiry = %d, want 2", calls)
	}
}

func TestNVIDIANIMModelsSingleFlight(t *testing.T) {
	cache := &modelsCache{}
	started := make(chan struct{})
	waiterObserved := make(chan struct{})
	release := make(chan struct{})
	var mu sync.Mutex
	calls := 0
	cache.nvidiaNIMLoader = func(string) ([]string, error) {
		mu.Lock()
		calls++
		mu.Unlock()
		close(started)
		<-release
		return []string{"vendor/model"}, nil
	}
	cache.nvidiaNIMWaitHook = func() { close(waiterObserved) }
	var wg sync.WaitGroup
	errs := make([]error, 2)
	for i := range errs {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			_, _, errs[i] = cache.getNVIDIANIMModels("test-key", false)
		}(i)
	}
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("NVIDIA NIM model fetch did not start")
	}
	select {
	case <-waiterObserved:
	case <-time.After(2 * time.Second):
		t.Fatal("second NVIDIA NIM model request did not join the in-flight fetch")
	}
	close(release)
	wg.Wait()
	mu.Lock()
	gotCalls := calls
	mu.Unlock()
	if gotCalls != 1 {
		t.Fatalf("loader calls = %d, want 1", gotCalls)
	}
	for i, err := range errs {
		if err != nil {
			t.Errorf("request %d error = %v", i, err)
		}
	}
}

func slicesContain(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}
