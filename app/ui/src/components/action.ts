// Something a button does that can take a moment or fail: whether it is
// going, and what went wrong the last time.

import { createSignal } from 'solid-js';

export function createAction<A extends unknown[], R>(work: (...args: A) => Promise<R>) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string>();
  return {
    busy,
    error,
    /** Runs it. A failure is kept in `error`, not thrown. */
    async run(...args: A): Promise<R | undefined> {
      setBusy(true);
      setError();
      try {
        return await work(...args);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
        return undefined;
      } finally {
        setBusy(false);
      }
    },
  };
}
