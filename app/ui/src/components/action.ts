// Something the user did that wb-agent has to carry out: while it runs the
// button that started it is busy, and if it fails the reason is shown.

import { createSignal } from 'solid-js';

export function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function createAction<A extends unknown[]>(work: (...args: A) => Promise<unknown>) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const run = async (...args: A) => {
    setBusy(true);
    setError();
    try {
      await work(...args);
    } catch (err) {
      setError(describe(err));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, run };
}
