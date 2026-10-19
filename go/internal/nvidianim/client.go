package nvidianim

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

const (
	CatalogURL     = "https://integrate.api.nvidia.com/v1/models"
	catalogTimeout = 10 * time.Second
	maxCatalogBody = 2 << 20
)

var (
	errCatalogRequest = errors.New("NVIDIA NIM model catalog request failed")
	errCatalogParse   = errors.New("NVIDIA NIM model catalog response is invalid")
	ErrCatalogTimeout = errors.New("NVIDIA NIM model catalog request timed out")
)

// CatalogHTTPError exposes only the HTTP status. Response bodies are never
// included because an upstream error body is untrusted and may contain data
// that should not be surfaced in the Hub UI.
type CatalogHTTPError struct {
	StatusCode int
}

func (e *CatalogHTTPError) Error() string {
	return fmt.Sprintf("NVIDIA NIM model catalog returned HTTP %d", e.StatusCode)
}

type catalogResponse struct {
	Data []struct {
		ID string `json:"id"`
	} `json:"data"`
}

// FetchModels retrieves the authenticated NVIDIA NIM model catalog. It uses a
// dedicated client so catalog latency and redirects do not affect other Hub
// requests.
func FetchModels(apiKey string) ([]string, error) {
	return fetchModels(newCatalogClient(), CatalogURL, apiKey)
}

// CheckAPIKey checks the key against the fixed NVIDIA /v1/models endpoint.
// It has a bounded timeout and does not return or parse upstream response text.
func CheckAPIKey(apiKey string) error {
	if strings.TrimSpace(apiKey) == "" {
		return ErrEmptyAPIKey
	}
	ctx, cancel := context.WithTimeout(context.Background(), catalogTimeout)
	defer cancel()
	return checkAPIKey(ctx, newCatalogClient(), CatalogURL, apiKey)
}

func newCatalogClient() *http.Client {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	return &http.Client{
		Timeout:   catalogTimeout,
		Transport: transport,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= 3 {
				return errors.New("too many redirects")
			}
			if req.URL.Scheme != "https" || req.URL.Host != "integrate.api.nvidia.com" || req.URL.User != nil {
				return errors.New("redirect outside NVIDIA NIM catalog is blocked")
			}
			return nil
		},
	}
}

func checkAPIKey(ctx context.Context, client *http.Client, endpoint, apiKey string) error {
	if strings.TrimSpace(apiKey) == "" {
		return ErrEmptyAPIKey
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return errCatalogRequest
	}
	req.Header.Set("Authorization", "Bearer "+strings.TrimSpace(apiKey))
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		var netErr net.Error
		if errors.Is(err, context.DeadlineExceeded) || (errors.As(err, &netErr) && netErr.Timeout()) {
			return ErrCatalogTimeout
		}
		return errCatalogRequest
	}
	defer resp.Body.Close()
	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return &CatalogHTTPError{StatusCode: resp.StatusCode}
	}
	_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, maxCatalogBody))
	return nil
}

func fetchModels(client *http.Client, endpoint, apiKey string) ([]string, error) {
	if strings.TrimSpace(apiKey) == "" {
		return nil, ErrEmptyAPIKey
	}
	req, err := http.NewRequest(http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, errCatalogRequest
	}
	req.Header.Set("Authorization", "Bearer "+strings.TrimSpace(apiKey))
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return nil, errCatalogRequest
	}
	defer resp.Body.Close()
	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return nil, &CatalogHTTPError{StatusCode: resp.StatusCode}
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxCatalogBody))
	if err != nil {
		return nil, errCatalogRequest
	}
	var parsed catalogResponse
	if err := json.Unmarshal(body, &parsed); err != nil {
		return nil, errCatalogParse
	}
	if parsed.Data == nil {
		return nil, errCatalogParse
	}
	ids := make([]string, 0, len(parsed.Data))
	seen := make(map[string]struct{}, len(parsed.Data))
	for _, item := range parsed.Data {
		id := strings.TrimSpace(item.ID)
		if !validCatalogModelID(id) {
			return nil, errCatalogParse
		}
		if _, duplicate := seen[id]; duplicate {
			return nil, errCatalogParse
		}
		seen[id] = struct{}{}
		ids = append(ids, id)
	}
	return ids, nil
}

func validCatalogModelID(id string) bool {
	if id == "" || len(id) > 256 {
		return false
	}
	for _, segment := range strings.Split(id, "/") {
		if segment == "" || segment == "." || segment == ".." {
			return false
		}
		for i := 0; i < len(segment); i++ {
			c := segment[i]
			if c >= 0x80 || !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
				(c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.') {
				return false
			}
		}
	}
	return true
}
