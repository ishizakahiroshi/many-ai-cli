package hub

import (
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
