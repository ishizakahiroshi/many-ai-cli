export type UserPrefsPutOperation = () => Promise<void>;

export interface UserPrefsPutQueue {
  request(): Promise<void>;
}

// Coalesce callers while one PUT is running, but remember that another PUT is
// needed when localStorage changes before the current operation completes.
// The returned promise covers every queued operation, so a flush cannot finish
// before the latest value has been sent.
export function createUserPrefsPutQueue(operation: UserPrefsPutOperation): UserPrefsPutQueue {
  let inFlight: Promise<void> | null = null;
  let queued = false;

  function request(): Promise<void> {
    queued = true;
    if (inFlight) return inFlight;

    const run = (async () => {
      while (queued) {
        queued = false;
        await operation();
      }
    })();
    inFlight = run;
    const clearInFlight = (): void => {
      if (inFlight === run) inFlight = null;
    };
    void run.then(clearInFlight, clearInFlight);
    return run;
  }

  return { request };
}
