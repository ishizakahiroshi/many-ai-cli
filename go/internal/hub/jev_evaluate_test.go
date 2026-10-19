package hub

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

type jevTransport func(*http.Request) (*http.Response, error)

func (fn jevTransport) RoundTrip(r *http.Request) (*http.Response, error) { return fn(r) }

func TestEvaluateJevSendsOnlyEnteredTextAndReturnsDisplayResult(t *testing.T) {
	client := &http.Client{Transport: jevTransport(func(r *http.Request) (*http.Response, error) {
		if r.Method != http.MethodPost || r.URL.String() != jevEndpoint || r.Header.Get("Authorization") != "Bearer test-key" {
			t.Fatalf("unexpected request metadata")
		}
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatal(err)
		}
		var payload map[string]any
		if err := json.Unmarshal(body, &payload); err != nil {
			t.Fatal(err)
		}
		if payload["state"] != "synthetic request" || payload["model"] != "jev-latest" {
			t.Fatalf("unexpected request payload")
		}
		if _, ok := payload["questions"].(map[string]any)["needs_strong_model"]; !ok {
			t.Fatal("missing question")
		}
		return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(`{"answers":{"task_type":{"type":"choice","choice":"feature","confidence":0.8},"complexity":{"type":"score","score":1.5,"confidence":0.7},"needs_strong_model":{"type":"noul","noul":0.4}},"usage":{"input_tokens":42},"model":"jev-1.13.0"}`)), Header: http.Header{}}, nil
	})}
	result, err := evaluateJev(context.Background(), client, jevEndpoint, "test-key", "synthetic request")
	if err != nil {
		t.Fatal(err)
	}
	if result.TaskType != "feature" || result.Complexity != 2.5 || result.NeedsStrongModelProbability != 0.4 || result.InputTokens != 42 {
		t.Fatalf("unexpected result: %+v", result)
	}
}

func TestEvaluateJevRejectsUnexpectedAnswer(t *testing.T) {
	client := &http.Client{Transport: jevTransport(func(*http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(`{"answers":{"task_type":{"type":"choice","choice":"unknown"},"complexity":{"type":"score","score":0},"needs_strong_model":{"type":"noul","noul":0}}}`)), Header: http.Header{}}, nil
	})}
	if _, err := evaluateJev(context.Background(), client, jevEndpoint, "test-key", "synthetic request"); err == nil {
		t.Fatal("unexpected answer accepted")
	}
}
