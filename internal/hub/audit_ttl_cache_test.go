package hub

import (
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// audit #29 回帰テスト:
// ttlCache.get が fetch（外部ネットワーク I/O 相当）の実行中も mutex を保持して
// 並行リクエストを直列化していた問題の修正を検証する。
//
// 検証観点:
//   1. 単一フライト: N 並行 caller がキャッシュ切れに同時遭遇しても fetch は 1 本に束ねられる。
//   2. ロック非保持: fetch 中でもキャッシュヒットする別 caller はブロックされず即座に返る。
//   3. TTL / 負 TTL / fallback の既存挙動が不変。

// ttlAuditNewCache はテスト用の int キャッシュを最小構成で生成する。
func ttlAuditNewCache(ttl, negativeTTL time.Duration, fallback int, fetch func(string) (int, error)) *ttlCache[int] {
	return &ttlCache[int]{
		ttl:         ttl,
		negativeTTL: negativeTTL,
		fallback:    fallback,
		fetch:       fetch,
	}
}

// TestTTLCacheGetSingleFlight: 空キャッシュへ N 並行 get が来ても fetch は 1 回だけ。
func TestTTLCacheGetSingleFlight(t *testing.T) {
	const callers = 16
	var fetchCount int32
	// fetch を開始したら全 caller が出揃うまで待ってから 1 本目を完了させる。
	started := make(chan struct{}, callers)
	release := make(chan struct{})

	cache := ttlAuditNewCache(time.Hour, time.Minute, -1, func(string) (int, error) {
		atomic.AddInt32(&fetchCount, 1)
		started <- struct{}{}
		<-release // 全 caller が Lock 待ち or Cond 待ちに入るまでブロック
		return 42, nil
	})

	results := make([]int, callers)
	var wg sync.WaitGroup
	for i := 0; i < callers; i++ {
		wg.Add(1)
		go func(idx int) {
			defer wg.Done()
			results[idx] = cache.get("src")
		}(i)
	}

	// 1 本目の fetch が始まるのを待つ。
	<-started
	// 残り caller が fetchCond.Wait() に入る猶予を与える。
	time.Sleep(50 * time.Millisecond)
	close(release)
	wg.Wait()

	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected single-flight fetch (1), got %d concurrent fetches", n)
	}
	for i, v := range results {
		if v != 42 {
			t.Fatalf("caller %d got %d, want shared fetch result 42", i, v)
		}
	}
}

// TestTTLCacheGetCacheHitNotBlockedByInflightFetch:
// あるキーの fetch が長時間 in-flight な間に、別 caller がキャッシュヒット（TTL 内）した場合、
// fetch 完了を待たず即座に返ること（= fetch 中にロックを保持していないこと）を検証する。
func TestTTLCacheGetCacheHitNotBlockedByInflightFetch(t *testing.T) {
	fetchEntered := make(chan struct{})
	fetchRelease := make(chan struct{})
	var fetchCount int32

	cache := ttlAuditNewCache(time.Hour, time.Minute, -1, func(string) (int, error) {
		n := atomic.AddInt32(&fetchCount, 1)
		if n == 1 {
			// 1 本目はプリウォーム用（即返す）。
			return 7, nil
		}
		// 2 本目（強制再 fetch）は長時間ブロックして in-flight を維持。
		close(fetchEntered)
		<-fetchRelease
		return 8, nil
	})

	// プリウォーム: data をセットして TTL 内のキャッシュヒットを可能にする。
	if v := cache.get("src"); v != 7 {
		t.Fatalf("prewarm got %d, want 7", v)
	}

	// 強制再 fetch を起こすため TTL を無効化（fetchedAt を過去へ）。
	cache.mu.Lock()
	cache.fetchedAt = time.Now().Add(-2 * time.Hour)
	cache.mu.Unlock()

	// in-flight fetch を起動。
	go func() { _ = cache.get("src") }()
	<-fetchEntered // fetch がロック外で走っていることを保証

	// この間に TTL を再び有効化し、別 caller がキャッシュヒットできる状態にする。
	cache.mu.Lock()
	cache.fetchedAt = time.Now() // data はまだ 7 のまま、TTL 内に戻す
	cache.mu.Unlock()

	done := make(chan int, 1)
	go func() { done <- cache.get("src") }()

	select {
	case v := <-done:
		if v != 7 {
			t.Fatalf("cache-hit caller got %d, want cached 7", v)
		}
	case <-time.After(2 * time.Second):
		close(fetchRelease)
		t.Fatal("cache-hit caller blocked behind in-flight fetch (lock held during fetch)")
	}
	close(fetchRelease)
}

// TestTTLCacheGetNegativeTTL: fetch 失敗後、負 TTL 内は再 fetch せず fallback を返す（既存挙動の不変確認）。
func TestTTLCacheGetNegativeTTL(t *testing.T) {
	var fetchCount int32
	cache := ttlAuditNewCache(time.Hour, time.Minute, -99, func(string) (int, error) {
		atomic.AddInt32(&fetchCount, 1)
		return 0, errTTLAuditFetch
	})

	if v := cache.get("src"); v != -99 {
		t.Fatalf("first get got %d, want fallback -99", v)
	}
	if v := cache.get("src"); v != -99 {
		t.Fatalf("second get got %d, want fallback -99", v)
	}
	if n := atomic.LoadInt32(&fetchCount); n != 1 {
		t.Fatalf("expected no re-fetch within negative TTL, got %d fetches", n)
	}
}

var errTTLAuditFetch = ttlAuditError("ttl audit fetch failed")

type ttlAuditError string

func (e ttlAuditError) Error() string { return string(e) }
