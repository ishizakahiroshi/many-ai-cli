package nvidianim

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestFetchModelsUsesBearerAndValidatesCatalogIDs(t *testing.T) {
	const apiKey = "test-nvidia-key"
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/models" {
			t.Errorf("request = %s %s, want GET /v1/models", r.Method, r.URL.Path)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer "+apiKey {
			t.Errorf("authorization header = %q, want bearer key", got)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":" moonshotai/kimi-k3 "},{"id":"deepseek-ai/deepseek-v4"}]}`))
	}))
	defer server.Close()

	ids, err := fetchModels(server.Client(), server.URL+"/v1/models", apiKey)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"moonshotai/kimi-k3", "deepseek-ai/deepseek-v4"}
	if fmt.Sprint(ids) != fmt.Sprint(want) {
		t.Fatalf("model IDs = %v, want %v", ids, want)
	}
}

func TestFetchModelsRejectsMalformedCatalogIDs(t *testing.T) {
	for _, tc := range []struct {
		name string
		id   string
	}{
		{name: "empty", id: " "},
		{name: "control", id: "model\nname"},
		{name: "space", id: "model name"},
		{name: "empty path segment", id: "vendor//model"},
		{name: "dot path segment", id: "vendor/./model"},
		{name: "parent path segment", id: "vendor/../model"},
		{name: "unsafe punctuation", id: "vendor/model;echo"},
		{name: "over 256 bytes", id: strings.Repeat("a", 257)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				_, _ = fmt.Fprintf(w, `{"data":[{"id":%q}]}`, tc.id)
			}))
			defer server.Close()
			if _, err := fetchModels(server.Client(), server.URL, "test-key"); err == nil {
				t.Fatalf("fetchModels accepted invalid ID %q", tc.id)
			}
		})
	}

	t.Run("duplicate", func(t *testing.T) {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write([]byte(`{"data":[{"id":"vendor/model"},{"id":"vendor/model"}]}`))
		}))
		defer server.Close()
		if _, err := fetchModels(server.Client(), server.URL, "test-key"); err == nil {
			t.Fatal("fetchModels accepted duplicate IDs")
		}
	})

	t.Run("missing data", func(t *testing.T) {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write([]byte(`{}`))
		}))
		defer server.Close()
		if _, err := fetchModels(server.Client(), server.URL, "test-key"); err == nil {
			t.Fatal("fetchModels accepted a missing data field")
		}
	})
}

func TestFetchModelsDoesNotIncludeAPIKeyInErrors(t *testing.T) {
	const apiKey = "synthetic-secret-key"
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, apiKey, http.StatusUnauthorized)
	}))
	defer server.Close()

	_, err := fetchModels(server.Client(), server.URL, apiKey)
	if err == nil || strings.Contains(err.Error(), apiKey) {
		t.Fatalf("error = %v, want sanitized status without API key", err)
	}
}

func TestFetchModelsRejectsMissingKeyBeforeRequest(t *testing.T) {
	called := false
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { called = true }))
	defer server.Close()
	if _, err := fetchModels(server.Client(), server.URL, "  "); err == nil {
		t.Fatal("fetchModels accepted an empty API key")
	}
	if called {
		t.Fatal("fetchModels made a request without an API key")
	}
}

func TestCheckAPIKeyUsesBearerAndReturnsOnlyHTTPStatus(t *testing.T) {
	const key = "synthetic-check-secret"
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/models" {
			t.Errorf("request = %s %s, want GET /v1/models", r.Method, r.URL.Path)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer "+key {
			t.Error("connection check did not send the configured key as bearer auth")
		}
		http.Error(w, key, http.StatusUnauthorized)
	}))
	defer server.Close()

	err := checkAPIKey(context.Background(), server.Client(), server.URL+"/v1/models", key)
	var statusErr *CatalogHTTPError
	if !errors.As(err, &statusErr) || statusErr.StatusCode != http.StatusUnauthorized {
		t.Fatalf("checkAPIKey error = %v, want HTTP 401 classification", err)
	}
	if strings.Contains(err.Error(), key) {
		t.Fatal("connection check error exposed the key or upstream response body")
	}
}

func TestCheckAPIKeyAcceptsSuccessfulStatusWithoutParsingBody(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()
	if err := checkAPIKey(context.Background(), server.Client(), server.URL, "synthetic-key"); err != nil {
		t.Fatalf("checkAPIKey returned error for successful status: %v", err)
	}
}

func TestCheckAPIKeyRejectsMissingKeyBeforeRequest(t *testing.T) {
	called := false
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { called = true }))
	defer server.Close()
	if err := checkAPIKey(context.Background(), server.Client(), server.URL, " "); !errors.Is(err, ErrEmptyAPIKey) {
		t.Fatalf("checkAPIKey error = %v, want missing-key error", err)
	}
	if called {
		t.Fatal("checkAPIKey made a request without a key")
	}
}
