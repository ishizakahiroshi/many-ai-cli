package hub

import (
	"sync"
	"time"
)

// ttlCache は「GitHub 等から JSON を取得し、TTL 付きでキャッシュ。失敗時は
// 負キャッシュで再試行を抑制しつつフォールバック値を返す」共通ロジックを
// ジェネリックに抽出したもの。usageLinkCache / modelsRemoteCache の重複していた
// get() 実装を 1 本に統合する。
//
// 利用側は newXxxCache() で ttl / negativeTTL / fallback / fetch を束ねた
// インスタンスを生成する。
type ttlCache[T any] struct {
	mu        sync.Mutex
	data      *T
	fetchedAt time.Time
	failedAt  time.Time // 最後に fetch に失敗した時刻（負キャッシュ用）

	// fetchInflight は単一フライト制御用。fetch（外部ネットワーク I/O）はロックを
	// 離して実行するため、複数 caller が同時にキャッシュ切れに遭遇しても fetch を
	// 1 本に束ね、残りは fetchCond で待機して結果を共有する。
	fetchInflight bool
	fetchCond     *sync.Cond // mu に紐づく。初回 get 時に遅延初期化。

	ttl         time.Duration
	negativeTTL time.Duration // 失敗後の再試行抑制期間
	fallback    T             // fetch 未成功時に返す値
	fetch       func(sourceURL string) (T, error)
	// transform は fetch 成功値をキャッシュ格納前に加工する（例: fallback とのマージ）。
	// nil なら素通し。
	transform func(fetched T) T
}

// get は TTL 内ならキャッシュを返し、期限切れなら sourceURL から再取得する。
// 取得失敗時はフォールバック値を返し、失敗時刻を記録して負 TTL 内は再試行しない。
//
// 外部ネットワーク I/O である c.fetch はロックを保持したまま呼ばない。
// fetch が必要なときは選出された 1 caller だけがロックを離して fetch し、
// 残りの並行 caller は fetchCond で待機して結果を共有する（単一フライト）。
// これにより fetch 遅延時でもキャッシュヒット／負キャッシュのパスは即座に
// ロックを解放し、並行リクエストが直列化しない。
func (c *ttlCache[T]) get(sourceURL string) T {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.fetchCond == nil {
		c.fetchCond = sync.NewCond(&c.mu)
	}

	for {
		// 成功キャッシュが有効
		if c.data != nil && time.Since(c.fetchedAt) < c.ttl {
			return *c.data
		}
		// 負キャッシュが有効（失敗後の再試行抑制）
		if !c.failedAt.IsZero() && time.Since(c.failedAt) < c.negativeTTL {
			if c.data != nil {
				return *c.data
			}
			return c.fallback
		}
		// 別 caller が既に fetch 中なら、その完了を待って再評価する。
		// 完了後はキャッシュ／負キャッシュが更新されている可能性が高い。
		if c.fetchInflight {
			c.fetchCond.Wait()
			continue
		}
		break
	}

	// 自分が fetch を担当する。ロックを離してネットワーク I/O を行う。
	c.fetchInflight = true
	c.mu.Unlock()

	fetched, err := c.fetch(sourceURL)

	c.mu.Lock()
	c.fetchInflight = false
	// 待機中の caller を全員起こす（更新後のキャッシュ／負キャッシュで再評価させる）。
	c.fetchCond.Broadcast()

	if err != nil {
		c.failedAt = time.Now() // 負 TTL 開始
		if c.data != nil {
			return *c.data // 直近成功値があればそちらを返す
		}
		return c.fallback
	}
	if c.transform != nil {
		fetched = c.transform(fetched)
	}
	c.data = &fetched
	c.fetchedAt = time.Now()
	c.failedAt = time.Time{} // 負キャッシュをリセット
	return fetched
}

// invalidate は次回 get で必ず再 fetch されるようキャッシュを破棄する。
// 負キャッシュ（failedAt）も解除する。直近 fetch が失敗した状態で invalidate を
// 呼んでも、failedAt が残ると次の get が負キャッシュ分岐に入り再 fetch されず
// fallback を返してしまう（force refresh が negativeTTL 内は効かない）ため。
func (c *ttlCache[T]) invalidate() {
	c.mu.Lock()
	c.data = nil
	c.fetchedAt = time.Time{}
	c.failedAt = time.Time{}
	c.mu.Unlock()
}
