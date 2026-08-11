export interface ReviewLoadOutcome<T> {
  stale: boolean;
  value?: T;
}

// resolveCurrentReviewLoad centralizes success and failure generation checks.
// A stale request resolves without a value even when its underlying operation
// rejects, so callers cannot render an obsolete error over a newer result.
export async function resolveCurrentReviewLoad<T>(
  generation: number,
  currentGeneration: () => number,
  operation: () => Promise<T>,
): Promise<ReviewLoadOutcome<T>> {
  try {
    const value = await operation();
    if (generation !== currentGeneration()) return { stale: true };
    return { stale: false, value };
  } catch (error) {
    if (generation !== currentGeneration()) return { stale: true };
    throw error;
  }
}
