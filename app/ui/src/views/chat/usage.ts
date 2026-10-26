// What a session has used, the way Pi's status line puts it:
// tokens in and out, how full the context is, and the cost.

import type { Usage } from '../../protocol/Usage';

/** 41k, 6.2k, 1.2M. */
export function tokens(n: number) {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
}

/** `↑41k ↓6.2k · 23% of 200k · $0.38`, or nothing before anything's been used. */
export function usage(used: Usage | null) {
  if (!used || (used.input === 0 && used.output === 0 && !used.context)) return undefined;
  const parts = [`↑${tokens(used.input)} ↓${tokens(used.output)}`];
  if (used.context != null && used.window)
    parts.push(`${Math.round((used.context / used.window) * 100)}% of ${tokens(used.window)}`);
  if (used.cost > 0) parts.push(`$${used.cost.toFixed(2)}`);
  return parts.join(' · ');
}
