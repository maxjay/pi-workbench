// An agent's reply, drawn from its markdown. What it wrote is made safe
// before it is put in the page.

import DOMPurify from 'dompurify';
import { marked } from 'marked';

/** A link must not take the window away from the app. Opening links comes in a later step. */
function keepLinksHere(box: HTMLDivElement) {
  box.addEventListener('click', (e) => {
    if ((e.target as Element).closest('a')) e.preventDefault();
  });
}

export function Markdown(props: { text: string }) {
  const html = () => DOMPurify.sanitize(marked.parse(props.text, { async: false, breaks: true }));
  return <div class="markdown" innerHTML={html()} ref={keepLinksHere} />;
}
