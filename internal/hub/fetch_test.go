package hub

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"
)

func useExternalHTTPClientForTest(t *testing.T, client *http.Client) {
	t.Helper()
	prev := makeExternalHTTPClient
	makeExternalHTTPClient = func(time.Duration) *http.Client { return client }
	t.Cleanup(func() { makeExternalHTTPClient = prev })
}

// --- C2: リダイレクト検証 ---

func TestNewExternalHTTPClientBlocksHTTPRedirect(t *testing.T) {
	// http:// へリダイレクトするサーバ
	httpTarget := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
	}))
	defer httpTarget.Close()

	httpsOrigin := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, httpTarget.URL, http.StatusFound)
	}))
	defer httpsOrigin.Close()

	client := newExternalHTTPClient(5 * time.Second)
	// TLS 証明書検証をスキップするためデフォルト transport を差し替え
	client.Transport = httpsOrigin.Client().Transport
	resp, err := client.Get(httpsOrigin.URL)
	if resp != nil {
		resp.Body.Close()
	}
	// http:// へのダウングレードでエラーになるはず
	if err == nil {
		t.Fatal("expected error for non-https redirect, got nil")
	}
}

func TestNewExternalHTTPClientBlocksInitialHTTP(t *testing.T) {
	client := newExternalHTTPClient(5 * time.Second)
	resp, err := client.Get("http://example.com/resource.md")
	if resp != nil {
		resp.Body.Close()
	}
	if err == nil {
		t.Fatal("expected error for initial non-https request, got nil")
	}
}

func TestNewExternalHTTPClientBlocksPrivateIPLiteral(t *testing.T) {
	client := newExternalHTTPClient(5 * time.Second)
	resp, err := client.Get("https://127.0.0.1/resource.md")
	if resp != nil {
		resp.Body.Close()
	}
	if err == nil {
		t.Fatal("expected error for private network host, got nil")
	}
}

func TestPrivateNetworkBlockingDialContextBlocksResolvedLoopback(t *testing.T) {
	called := false
	dial := privateNetworkBlockingDialContext(func(context.Context, string, string) (net.Conn, error) {
		called = true
		return nil, errors.New("dial should not be called")
	})

	_, err := dial(context.Background(), "tcp", "localhost:443")
	if err == nil {
		t.Fatal("expected error for localhost resolving to loopback")
	}
	if called {
		t.Fatal("wrapped dialer should not be called for blocked hosts")
	}
}

func TestNewExternalHTTPClientBlocksTooManyRedirects(t *testing.T) {
	var count int32
	var ts *httptest.Server
	ts = httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&count, 1)
		http.Redirect(w, r, ts.URL, http.StatusFound)
	}))
	defer ts.Close()

	client := newExternalHTTPClient(5 * time.Second)
	client.Transport = ts.Client().Transport
	resp, err := client.Get(ts.URL)
	if resp != nil {
		resp.Body.Close()
	}
	if err == nil {
		t.Fatal("expected error for too many redirects, got nil")
	}
}

// --- C3: ステータス判定順序 ---

func TestFetchModelsDefaultsRejectsNon2xx(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
		// ボディは書かない（読まれないことを確認するため）
	}))
	defer ts.Close()
	useExternalHTTPClientForTest(t, ts.Client())

	_, err := fetchModelsDefaults(ts.URL)
	if err == nil {
		t.Fatal("expected error for 404, got nil")
	}
}

func TestFetchUsageLinkDefaultsRejectsNon2xx(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer ts.Close()
	useExternalHTTPClientForTest(t, ts.Client())

	_, err := fetchUsageLinkDefaults(ts.URL)
	if err == nil {
		t.Fatal("expected error for 500, got nil")
	}
}

// --- C4: 負キャッシュ ---

func TestModelsRemoteCacheNegativeTTL(t *testing.T) {
	var fetchCount int32
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&fetchCount, 1)
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer ts.Close()
	useExternalHTTPClientForTest(t, ts.Client())

	cache := newModelsRemoteCache()

	// 1 回目: fetch 試行 → 失敗
	_ = cache.get(ts.URL)
	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected 1 fetch after first call, got %d", n)
	}

	// 2 回目: 負 TTL 内なので fetch しない
	_ = cache.get(ts.URL)
	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected no re-fetch within negative TTL, got %d total fetches", n)
	}
}

func TestUsageLinkCacheNegativeTTL(t *testing.T) {
	var fetchCount int32
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&fetchCount, 1)
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer ts.Close()
	useExternalHTTPClientForTest(t, ts.Client())

	cache := newUsageLinkCache()

	_ = cache.get(ts.URL)
	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected 1 fetch, got %d", n)
	}

	_ = cache.get(ts.URL)
	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected no re-fetch within negative TTL, got %d total fetches", n)
	}
}

func TestModelsRemoteCacheNegativeTTLResetOnSuccess(t *testing.T) {
	var fetchCount int32
	fail := true
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&fetchCount, 1)
		if fail {
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		d := modelsDefaults{
			Anthropic: []Model{{ID: "test-model", Label: "Test"}},
		}
		b, _ := json.Marshal(d)
		w.Header().Set("Content-Type", "application/json")
		w.Write(b)
	}))
	defer ts.Close()
	useExternalHTTPClientForTest(t, ts.Client())

	cache := newModelsRemoteCache()

	// 失敗 → 負 TTL 開始
	cache.get(ts.URL)

	// 負 TTL を強制的に過去に設定してリセット
	cache.mu.Lock()
	cache.failedAt = time.Now().Add(-modelsRemoteNegativeTTL - time.Second)
	cache.mu.Unlock()

	// 成功に切り替えて再取得
	fail = false
	result := cache.get(ts.URL)

	if len(result.Anthropic) == 0 || result.Anthropic[0].ID != "test-model" {
		t.Fatalf("expected test-model after recovery, got %+v", result)
	}
	// 負キャッシュがクリアされていること
	cache.mu.Lock()
	failedAt := cache.failedAt
	cache.mu.Unlock()
	if !failedAt.IsZero() {
		t.Fatal("failedAt should be reset after successful fetch")
	}
}

// --- ローカル fetch サイズ上限（C6） ---

func TestReadSlashCmdSourceLocalFileSizeLimit(t *testing.T) {
	dir := testConfigDir(t)
	path := filepath.Join(dir, "big.md")
	// slashCmdLocalMaxBytes + 1 バイトのファイルを作成
	data := make([]byte, slashCmdLocalMaxBytes+1)
	for i := range data {
		data[i] = 'a'
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}

	body, err := readSlashCmdSource(path)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// LimitReader により slashCmdLocalMaxBytes までしか読まれない
	if int64(len(body)) > slashCmdLocalMaxBytes {
		t.Fatalf("expected at most %d bytes, got %d", slashCmdLocalMaxBytes, len(body))
	}
}

func TestReadSlashCmdSourceRejectsDirectory(t *testing.T) {
	dir := testConfigDir(t)
	_, err := readSlashCmdSource(dir)
	if err == nil {
		t.Fatal("expected error for directory source, got nil")
	}
}
