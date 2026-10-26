// Language servers, such as jvmd, plug into a codespace's editor bar: a
// dot for how each one is, and what it can run, such as an app's run
// configurations. None is live yet, so the bar shows none; this is where
// they'll come in, and Ctrl P's `@` symbols with them.

import type { CodespaceView } from '../../protocol/CodespaceView';

export interface Integration {
  name: string;
  state: 'starting' | 'ready' | 'failed';
  detail?: string;
  /** What it can run, such as Run OrderApp. */
  actions: { label: string; run: () => void }[];
}

/** The integrations a codespace has. */
export function integrations(_view: CodespaceView): Integration[] {
  return [];
}
