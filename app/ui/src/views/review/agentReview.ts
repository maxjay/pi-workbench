// The agent's review of a whole pull request: what it is asked, and how
// its answer is read into comments drafted on lines. Nothing it writes is
// posted. Each draft waits under its line for you to post or discard.

import type { Place } from '../../protocol/Place';
import type { Review } from '../../protocol/Review';

/** Marks a note as a draft from a review of the whole pull request. */
export const FROM_REVIEW = 'review';

export function reviewPrompt(review: Review, focus: string) {
  return `Review this pull request the way a careful reviewer would.

Read the whole change (\`git diff ${review.base.slice(0, 9)} ${review.head.slice(0, 9)}\` in this folder) and the code around it. Look for bugs, cases that are not handled, security problems, and tests that are missing or wrong. Leave style alone unless it hides a bug. Do not repeat what a review thread already says.${
    focus ? `\n\nLook at this above all: ${focus}` : ''
  }

Answer with a summary of a few lines, then one JSON block, and nothing after it:

\`\`\`json
[{ "file": "path/from/the/repo/root.ts", "from": 10, "line": 12, "old": false, "comment": "The comment, written to the author, ready to post." }]
\`\`\`

"from" and "line" are the first and last line the comment is on, counted in the new file, or in the old file when "old" is true. They must be lines the diff shows. Give an empty list if nothing is worth a comment.`;
}

export interface Drafted {
  place: Place;
  text: string;
}

/** The agent's answer as its summary and the comments it drafted. An answer with no list is all summary. */
export function readReview(answer: string): { summary: string; comments: Drafted[] } {
  const blocks = [...answer.matchAll(/```(?:json)?\s*(\[[\s\S]*?\])\s*```/g)];
  const last = blocks.at(-1);
  if (!last || last.index === undefined) return { summary: answer.trim(), comments: [] };
  let list: unknown;
  try {
    list = JSON.parse(last[1]);
  } catch {
    return { summary: answer.trim(), comments: [] };
  }
  const comments: Drafted[] = [];
  for (const entry of Array.isArray(list) ? list : []) {
    const { file, from, line, old, comment } = entry as Record<string, unknown>;
    if (typeof file !== 'string' || typeof comment !== 'string' || !comment.trim()) continue;
    const last = Number(line ?? from);
    const first = Number(from ?? line);
    if (!Number.isInteger(last) || !Number.isInteger(first) || last < 1) continue;
    comments.push({
      place: {
        file: file.replace(/^\.?\//, ''),
        from: Math.min(first, last),
        line: Math.max(first, last),
        old: old === true,
        outdated: false,
      },
      text: comment.trim(),
    });
  }
  const summary = answer.slice(0, last.index).trim();
  return { summary: summary || 'The agent gave no summary.', comments };
}
