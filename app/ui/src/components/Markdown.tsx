// Pi's replies are markdown. They're rendered to HTML and sanitized, since
// a model can write anything, and links don't navigate the window away.

import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { createMemo } from 'solid-js';

marked.use({ gfm: true });

export function Markdown(props: { text: string }) {
  const html = createMemo(() => DOMPurify.sanitize(marked.parse(props.text, { async: false })));
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: only stops link clicks; the links stay focusable
    // biome-ignore lint/a11y/noStaticElementInteractions: as above
    <div
      class="markdown"
      innerHTML={html()}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest('a')) e.preventDefault();
      }}
    />
  );
}
