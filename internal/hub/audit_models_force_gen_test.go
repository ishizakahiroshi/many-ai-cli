package hub

import (
	"testing"
	"time"
)

// audit_models_force_gen_test.go は finding #24（ollama local キャッシュの
// singleflight が force リクエストを並行非force取得で満たし最新化を取りこぼす）の
// 回帰テスト。
//
// これらのテストはネットワーク（localhost:11434 の daemon）に依存しない:
//   - getOllamaLocal は満たされない場合に実 fetch を試みるが、daemon の有無に
//     かかわらず「新しい entry に置き換わったか」「世代が進んだか」だけを観測する。
//   - 並行レースの再現では in-flight fetch を手動でシミュレートし、リーダー path を
//     実 fetch に到達させないよう待機 path だけを検証する。

// auditModelsSeedEntry は完了済みの古い世代の entry を cache に直接植える。
func auditModelsSeedEntry(c *modelsCache, gen uint64, fetchedAt time.Time) *ollamaTagsCacheEntry {
	e := &ollamaTagsCacheEntry{
		models:     []Model{{ID: "stale:latest", Label: "stale"}},
		fetchedAt:  fetchedAt,
		err:        nil,
		tagsURL:    ollamaTagsURL(""),
		generation: gen,
	}
	c.mu.Lock()
	c.local = e
	c.generation = gen
	c.mu.Unlock()
	return e
}

// TestForceDoesNotAcceptStaleInFlightResult は finding #24 の中核を検証する。
// invalidate() で世代を進めた直後の force 取得が、invalidate より前に開始された
// （古い世代の）in-flight fetch の結果で満たされてはならない。
//
// 構成: 古い世代の in-flight fetch をシミュレートし、force 要求者が待機 →
// 待機解除後に「古い世代 entry を fresh と認めず、新規 fetch を起こす」ことを確認する。
func TestForceDoesNotAcceptStaleInFlightResult(t *testing.T) {
	c := &modelsCache{}

	// 世代 0 の in-flight fetch が走っている状態を作る（A: 非force 起点）。
	oldFetch := &ollamaTagsFetch{done: make(chan struct{}), startGen: 0}
	c.mu.Lock()
	c.localFetch = oldFetch
	c.mu.Unlock()

	// クライアントB が明示リフレッシュ: invalidate() で世代を 1 へ。
	c.invalidate()

	staleAt := time.Now().Add(-time.Hour)
	var (
		forceModels    []Model
		forceFetchedAt time.Time
		done           = make(chan struct{})
	)
	go func() {
		// force=true。c.localFetch!=nil なので待機 path に入る。
		forceModels, forceFetchedAt, _ = c.getOllamaLocal(true, "")
		close(done)
	}()

	// B が確実に待機 path へ入るまで少し待つ（startGen 取得＆<-wait 到達）。
	auditModelsWaitUntil(t, func() bool {
		c.mu.Lock()
		defer c.mu.Unlock()
		return c.localFetch == oldFetch
	})

	// A（古い世代 fetch）が「invalidate 前の」古い結果を書き込んで完了する様子を再現。
	c.mu.Lock()
	c.local = &ollamaTagsCacheEntry{
		models:     []Model{{ID: "old:latest", Label: "old"}},
		fetchedAt:  staleAt,
		err:        nil,
		tagsURL:    ollamaTagsURL(""),
		generation: oldFetch.startGen, // = 0（invalidate 前）
	}
	if c.localFetch == oldFetch {
		c.localFetch = nil
	}
	c.mu.Unlock()
	close(oldFetch.done)

	<-done

	// B は古い世代(0)の結果を受け取ってはならない。
	if forceFetchedAt.Equal(staleAt) {
		t.Fatalf("force request was satisfied by the stale generation-0 result (fetchedAt unchanged); finding #24 regressed")
	}
	if len(forceModels) == 1 && forceModels[0].ID == "old:latest" {
		t.Fatalf("force request returned the pre-invalidate model list; expected a fresh fetch instead")
	}

	// B が起こした新規 fetch の結果が世代 >= 1 で記録されていること。
	c.mu.Lock()
	gotGen := c.local.generation
	c.mu.Unlock()
	if gotGen < 1 {
		t.Fatalf("post-force cache entry generation = %d, want >= 1 (the invalidate generation)", gotGen)
	}
}

// TestNonForceServesFreshCachedEntry は通常動作（非force・TTL 内）が不変であることを確認する。
// 非force は世代に関係なく TTL 内なら即座にキャッシュを返し、実 fetch を起こさない。
func TestNonForceServesFreshCachedEntry(t *testing.T) {
	c := &modelsCache{}
	freshAt := time.Now()
	seeded := auditModelsSeedEntry(c, 3, freshAt)

	models, fetchedAt, err := c.getOllamaLocal(false, "")
	if err != nil {
		t.Fatalf("unexpected error from cached non-force get: %v", err)
	}
	if !fetchedAt.Equal(freshAt) {
		t.Fatalf("non-force get did not serve the cached entry (fetchedAt=%v want=%v); a network fetch likely ran", fetchedAt, freshAt)
	}
	if len(models) != 1 || models[0].ID != seeded.models[0].ID {
		t.Fatalf("non-force get returned %v, want cached %v", models, seeded.models)
	}
}

// TestForceAcceptsCurrentGenerationEntry は dedup の維持を確認する。
// invalidate 後に開始された（= 現世代の）fetch 結果は、別の force 要求者が
// 再 fetch せずに受け取れる（thundering を起こさない）。
func TestForceAcceptsCurrentGenerationEntry(t *testing.T) {
	c := &modelsCache{}
	c.invalidate() // 世代 1 へ

	// 現世代(1)で完了した entry を植える。
	at := time.Now()
	auditModelsSeedEntry(c, 1, at) // generation=1, fetchedAt=now

	models, fetchedAt, err := c.getOllamaLocal(true, "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !fetchedAt.Equal(at) {
		t.Fatalf("force get re-fetched despite a current-generation entry (fetchedAt=%v want=%v); dedup not preserved", fetchedAt, at)
	}
	if len(models) != 1 || models[0].ID != "stale:latest" {
		t.Fatalf("force get returned %v, want the current-generation cached entry", models)
	}
}

// auditModelsWaitUntil は cond が true になるまで短時間ポーリングで待つ。
func auditModelsWaitUntil(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatalf("condition not met within timeout")
}
