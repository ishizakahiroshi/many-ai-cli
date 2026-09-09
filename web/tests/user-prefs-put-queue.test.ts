import { describe, expect, test } from 'bun:test';
import { createUserPrefsPutQueue } from '../src/app/user-prefs-put-queue.ts';

describe('user prefs PUT queue', () => {
  test('sends a follow-up PUT when a newer value arrives during an in-flight PUT', async () => {
    let releaseFirst: () => void = () => {};
    const firstPut = new Promise<void>((resolve) => { releaseFirst = resolve; });
    let latestValue = 'old';
    const persistedValues: string[] = [];
    let putCount = 0;
    const queue = createUserPrefsPutQueue(async () => {
      persistedValues.push(latestValue);
      putCount += 1;
      if (putCount === 1) await firstPut;
    });

    const first = queue.request();
    latestValue = 'new';
    const flushed = queue.request();

    expect(persistedValues).toEqual(['old']);
    releaseFirst();
    await flushed;
    await first;
    expect(persistedValues).toEqual(['old', 'new']);
  });
});
