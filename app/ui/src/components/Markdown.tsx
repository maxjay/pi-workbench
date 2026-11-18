// An agent's reply, drawn from its markdown. What it wrote is made safe
// before it is put in the page.

import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { openUrl } from './open';

/** A link must not take the window away from the app: it opens in your own browser. */
function keepLinksHere(box: HTMLDivElement) {
  box.addEventListener('click', (e) => {
    const link = (e.target as Element).closest('a');
    if (!link) return;
    e.preventDefault();
    openUrl(link.href);
  });
}

export function Markdown(props: { text: string }) {
  const html = () => DOMPurify.sanitize(marked.parse(props.text, { async: false, breaks: true }));
  return <div class="markdown" innerHTML={html()} ref={keepLinksHere} />;
}
