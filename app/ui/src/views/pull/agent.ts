// The agent in a pull request: what it is told with each question, and how
// its answers are read. It has one conversation for the whole pull request.
// A question asked under some lines is a turn of that same conversation,
// marked with where it was asked, so the chat on the right can show it as
// one line and not say the answer twice.

import type { Place } from '../../protocol/Place';
import type { Review } from '../../protocol/Review';
import type { Thread } from '../../protocol/Thread';
import { linesAt } from './parts';

const MARK = { context: ' ', added: '+', removed: '-' } as const;

/** What a question is about, written in front of it for the agent. `under` marks one asked under the lines. */
export function header(where: string, context: string, under: boolean) {
  return `<about where="${where.replace(/"/g, "'")}"${under ? ' under="yes"' : ''}>\n${context.trim()}\n</about>\n\n`;
}

/** What you wrote, without what was put in front of it: where it was asked, and whether under the lines. */
export function shown(text: string): { where?: string; under: boolean; text: string } {
  const found = text.match(/^<about where="([^"]*)"( under="yes")?>\n[\s\S]*?\n<\/about>\n\n/);
  if (!found) return { under: false, text };
  return { where: found[1], under: Boolean(found[2]), text: text.slice(found[0].length) };
}

/** The lines a place is on, the way the diff has them. */
export function code(review: Review, place: Place) {
  const file = review.files.find((f) => f.path === place.file);
  const lines = file ? linesAt(file.lines, place) : [];
  if (lines.length === 0) return '';
  return `The lines, as the diff has them:\n\n\`\`\`\n${lines.map((l) => `${MARK[l.kind]}${l.text}`).join('\n')}\n\`\`\``;
}

/** A thread, as the agent is told it. */
export function threadText(thread: Thread) {
  const said = thread.comments.map((c) => `- ${c.author ?? 'Someone'}: ${c.text}`).join('\n');
  return `Review thread #${thread.id} (${thread.status}):\n${said}`;
}

export const DRAFT_REPLY =
  'Write my reply to this thread. Use what we have said about it. Answer with only the text of the reply, ready to post, and nothing else.';

export const DRAFT_COMMENT =
  'Write the review comment I should post on these lines. Use what we have said about them. Answer with only the text of the comment, ready to post, and nothing else.';

export function reviewPrompt(review: Review) {
  return `Review this pull request the way a careful reviewer would.

Read the whole change (\`git diff ${review.base.slice(0, 9)} ${review.head.slice(0, 9)}\` in this folder) and the code around it. Look for bugs, cases that are not handled, security problems, and tests that are missing or wrong. Leave style alone unless it hides a bug. Do not repeat what a review thread already says.

Answer with a summary of a few lines, then one block exactly like this, and nothing after it:

\`\`\`findings
[{ "file": "path/from/the/repo/root.ts", "from": 10, "line": 12, "old": false, "text": "What is wrong here, in a sentence or two." }]
\`\`\`

"from" and "line" are the first and last line a finding is on, counted in the new file, or in the old file when "old" is true. They must be lines the diff shows. Give an empty list if nothing is worth saying.`;
}

const FINDINGS = /```(?:findings|json)?\s*(\[[\s\S]*?\])\s*```\s*$/;

/** What the agent wrote, without the list that is for the app. */
export const withoutFindings = (text: string) => text.replace(FINDINGS, '').trimEnd();

export interface Finding {
  place: Place;
  text: string;
}

/** The agent's review as its summary and what it found. An answer with no list is all summary. */
export function readReview(answer: string): { summary: string; findings: Finding[] } {
  const block = answer.match(FINDINGS);
  if (!block) return { summary: answer.trim(), findings: [] };
  let list: unknown;
  try {
    list = JSON.parse(block[1]);
  } catch {
    return { summary: answer.trim(), findings: [] };
  }
  const findings: Finding[] = [];
  for (const entry of Array.isArray(list) ? list : []) {
    const { file, from, line, old, text, comment } = entry as Record<string, unknown>;
    const words = typeof text === 'string' ? text : comment;
    if (typeof file !== 'string' || typeof words !== 'string' || !words.trim()) continue;
    const last = Number(line ?? from);
    const first = Number(from ?? line);
    if (!Number.isInteger(last) || !Number.isInteger(first) || last < 1) continue;
    findings.push({
      place: {
        file: file.replace(/^\.?\//, ''),
        from: Math.min(first, last),
        line: Math.max(first, last),
        old: old === true,
        outdated: false,
      },
      text: words.trim(),
    });
  }
  return { summary: withoutFindings(answer).trim() || 'The agent gave no summary.', findings };
}
