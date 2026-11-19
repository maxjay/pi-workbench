// Code as the agent writes it, in the colours the editor uses. The
// colours are worked out once for each block and kept, so a conversation
// drawn again costs nothing, and a very long block is left plain.

import { highlightTree, tags as t, tagHighlighter } from '@lezer/highlight';
import { languageNamed } from './languages';

/** The same kinds the editor colours, each as a class theme.css's tokens colour. */
const kinds = tagHighlighter([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], class: 'syn-keyword' },
  { tag: [t.string, t.special(t.string), t.regexp], class: 'syn-string' },
  { tag: [t.number, t.bool, t.null, t.atom], class: 'syn-number' },
  { tag: [t.comment, t.lineComment, t.blockComment], class: 'syn-comment' },
  { tag: [t.typeName, t.className, t.namespace], class: 'syn-type' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], class: 'syn-function' },
  { tag: [t.meta, t.annotation], class: 'syn-number' },
]);

/** Longer than this, a block is not coloured: reading it all would be felt. */
const MOST = 40_000;
/** How many blocks' colours are kept. The oldest goes first. */
const KEPT = 300;
const kept = new Map<string, string>();

const safe = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** A block's colours if they were worked out before, without waiting. */
export const colouredBefore = (code: string, language: string) => kept.get(`${language}\n${code}`);

/** A block of code as HTML with its colours. Nothing when its language can't be read or it is too long. */
export async function coloured(code: string, language: string): Promise<string | undefined> {
  const load = languageNamed(language);
  if (!load || code.length > MOST) return undefined;
  const key = `${language}\n${code}`;
  const known = kept.get(key);
  if (known !== undefined) return known;
  const tree = (await load()).language.parser.parse(code);
  let html = '';
  let at = 0;
  highlightTree(tree, kinds, (from, to, classes) => {
    html += `${safe(code.slice(at, from))}<span class="${classes}">${safe(code.slice(from, to))}</span>`;
    at = to;
  });
  html += safe(code.slice(at));
  if (kept.size >= KEPT) kept.delete(kept.keys().next().value as string);
  kept.set(key, html);
  return html;
}

/** Puts text where Ctrl V finds it. True when it did. */
export async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Some windows only let a page copy what is selected in a field.
    const field = document.createElement('textarea');
    field.value = text;
    field.style.position = 'fixed';
    field.style.opacity = '0';
    document.body.append(field);
    field.select();
    const did = document.execCommand('copy');
    field.remove();
    return did;
  }
}
