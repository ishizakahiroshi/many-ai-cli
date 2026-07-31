import { describe, expect, test } from 'bun:test';
import { resolveCurrentReviewLoad } from '../src/app/review-load-generation.ts';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('resolveCurrentReviewLoad', () => {
  test('ignores an older failure after a newer request succeeds', async () => {
    let currentGeneration = 1;
    const oldRequest = deferred<string>();
    const oldResult = resolveCurrentReviewLoad(
      1,
      () => currentGeneration,
      () => oldRequest.promise,
    );

    currentGeneration = 2;
    const newResult = await resolveCurrentReviewLoad(
      2,
      () => currentGeneration,
      async () => 'turn 2',
    );
    expect(newResult).toEqual({ stale: false, value: 'turn 2' });

    oldRequest.reject(new Error('turn 1 was evicted'));
    await expect(oldResult).resolves.toEqual({ stale: true });
  });

  test('propagates a failure from the current generation', async () => {
    await expect(resolveCurrentReviewLoad(
      3,
      () => 3,
      async () => {
        throw new Error('current request failed');
      },
    )).rejects.toThrow('current request failed');
  });
});
