// Markdown, drawn: an agent's reply, a pull request's description, a
// review comment. What was written is made safe before it is put in the page.

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

/** A picture kept on Azure DevOps: it is only given to someone signed in, which the page is not. */
const SIGNED_IN_ONLY = /^https:\/\/(dev\.azure\.com|[\w-]+\.visualstudio\.com)\//;

/**
 * A picture the page can't load would show as a broken one. It becomes a
 * link that says what it is and opens it in your own browser, where you
 * are signed in.
 */
function withReachablePictures(html: string) {
  const box = document.createElement('template');
  box.innerHTML = html;
  for (const picture of box.content.querySelectorAll('img')) {
    if (!SIGNED_IN_ONLY.test(picture.src)) continue;
    const link = document.createElement('a');
    link.href = picture.src;
    link.className = 'picture-link';
    link.textContent = `Picture: ${picture.alt || 'no description'} · opens in your browser`;
    picture.replaceWith(link);
  }
  return box.innerHTML;
}

/** HTML that came from somewhere else, drawn: a ticket's text as Azure DevOps keeps it. Made safe first. */
export function Html(props: { html: string }) {
  const html = () => withReachablePictures(DOMPurify.sanitize(props.html));
  return <div class="markdown" innerHTML={html()} ref={keepLinksHere} />;
}

export function Markdown(props: { text: string }) {
  const html = () =>
    withReachablePictures(DOMPurify.sanitize(marked.parse(props.text, { async: false, breaks: true })));
  return <div class="markdown" innerHTML={html()} ref={keepLinksHere} />;
}
