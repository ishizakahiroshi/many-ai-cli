package hub

import (
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
	baseURL := "http://192.168.11.50:11434"
	cache := &modelsCache{
		local: &ollamaTagsCacheEntry{
			models:    []Model{{ID: "qwen3:8b", Label: "qwen3:8b"}},
			fetchedAt: time.Now(),
			tagsURL:   ollamaTagsURL(baseURL),
		},
	}
	resp := buildModelsResponse(cache, nil, "", nil, baseURL, "", false)
	if got := resp.Sources["ollama_local"]; got != "http://192.168.11.50:11434/api/tags" {
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
