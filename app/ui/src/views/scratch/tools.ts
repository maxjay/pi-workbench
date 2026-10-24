// A tool call in one line, the way Pi in a terminal shows it: what ran and
// on what, with the lines an edit or write added and removed.

import type { Tool } from '../../protocol/Tool';

export interface Summary {
  verb: string;
  target: string;
  added?: number;
  removed?: number;
}

type Args = Record<string, unknown>;

const str = (value: unknown) => (typeof value === 'string' ? value : '');

export function lines(text: string): number {
  if (!text) return 0;
  return text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length;
}

/** An edit's changes: one `{ oldText, newText }`, or a list of them. */
export function edits(args: Args): { oldText: string; newText: string }[] {
  const list = Array.isArray(args.edits) ? (args.edits as Args[]) : [args];
  return list.map((edit) => ({ oldText: str(edit.oldText), newText: str(edit.newText) }));
}

export function summarize(tool: Tool): Summary {
  const args = (tool.args ?? {}) as Args;
  switch (tool.name) {
    case 'bash':
      return { verb: '$', target: str(args.command) };
    case 'read':
      return { verb: 'read', target: str(args.path) };
    case 'write':
      return { verb: 'write', target: str(args.path), added: lines(str(args.content)) };
    case 'edit': {
      const changes = edits(args);
      return {
        verb: 'edit',
        target: str(args.path),
        added: changes.reduce((n, edit) => n + lines(edit.newText), 0),
        removed: changes.reduce((n, edit) => n + lines(edit.oldText), 0),
      };
    }
    case 'grep':
      return { verb: 'grep', target: [str(args.pattern), str(args.path)].filter(Boolean).join(' ') };
    case 'find':
    case 'ls':
      return { verb: tool.name, target: str(args.pattern) || str(args.path) };
    default: {
      const first = Object.values(args).find((value) => typeof value === 'string');
      return { verb: tool.name, target: str(first) };
    }
  }
}
