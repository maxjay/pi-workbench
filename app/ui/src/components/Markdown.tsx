// Markdown, drawn: an agent's reply, a pull request's description, a
// review comment. What was written is made safe before it is put in the
// page. A block of code is set apart, says its language, and has Copy.
// While a reply is still being written it is drawn again at most a few
// times a second, and a block gets its colours only once it is in view.

import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { createEffect, onCleanup } from 'solid-js';
import { coloured, colouredBefore, copy } from './code';
import { openUrl } from './open';

/** A link must not take the window away from the app: it opens in your own browser. Copy copies its block. */
function onClicks(box: HTMLDivElement) {
  box.addEventListener('click', (e) => {
    const target = e.target as Element;
    const button = target.closest<HTMLButtonElement>('.code-copy');
    if (button) {
      const code = button.closest('.code')?.querySelector('pre')?.textContent ?? '';
      void copy(code.replace(/\n$/, '')).then((did) => {
        button.textContent = did ? 'Copied' : 'Not copied';
        setTimeout(() => {
          button.textContent = 'Copy';
        }, 1500);
      });
      return;
    }
    const link = target.closest('a');
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
function withReachablePictures(inside: DocumentFragment) {
  for (const picture of inside.querySelectorAll('img')) {
    if (!SIGNED_IN_ONLY.test(picture.src)) continue;
    const link = document.createElement('a');
    link.href = picture.src;
    link.className = 'picture-link';
    link.textContent = `Picture: ${picture.alt || 'no description'} · opens in your browser`;
    picture.replaceWith(link);
  }
}

/** Each block of code gets a line above it: its language, and Copy. */
function withCodeBlocks(inside: DocumentFragment) {
  for (const pre of inside.querySelectorAll('pre')) {
    const code = pre.querySelector('code');
    const language = code?.className.match(/language-([\w+#-]+)/)?.[1] ?? '';
    const block = document.createElement('div');
    block.className = 'code';
    block.dataset.language = language;
    const head = document.createElement('div');
    head.className = 'code-head';
    const name = document.createElement('span');
    name.textContent = language || 'code';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'quiet code-copy';
    button.textContent = 'Copy';
    head.append(name, button);
    pre.replaceWith(block);
    block.append(head, pre);
    // Colours worked out before are put in at once, so drawing again does not flicker.
    const known = code && language ? colouredBefore(code.textContent ?? '', language) : undefined;
    if (code && known !== undefined) {
      code.innerHTML = known;
      block.dataset.coloured = '';
    }
  }
}

/** Safe HTML as what is put in the page. */
function prepared(html: string) {
  const box = document.createElement('template');
  box.innerHTML = html;
  withReachablePictures(box.content);
  withCodeBlocks(box.content);
  return box.content;
}

/** Colours each block of code when it comes into view. One watcher for the whole app. */
const inView = new IntersectionObserver((seen) => {
  for (const entry of seen) {
    if (!entry.isIntersecting) continue;
    const block = entry.target as HTMLElement;
    inView.unobserve(block);
    const code = block.querySelector('code');
    const language = block.dataset.language;
    if (!code || !language) continue;
    const text = code.textContent ?? '';
    void coloured(text, language).then((html) => {
      // The block may have been drawn again, with other words, while its colours were worked out.
      if (html === undefined || !block.isConnected || code.textContent !== text) return;
      code.innerHTML = html;
      block.dataset.coloured = '';
    });
  }
});

/**
 * Draws what `source()` is, made into safe HTML by `html`, into a box. It is drawn again when the
 * source changes, but not more often than `every` ms: a reply that is being written changes many
 * times a second.
 */
function drawn(source: () => string, html: (source: string) => string, every: number) {
  let box: HTMLDivElement | undefined;
  let last = 0;
  let waiting: ReturnType<typeof setTimeout> | undefined;
  let watched: Element[] = [];
  // What is in the box now. It is the same text the conversation holds, so keeping it costs nothing.
  let shown: string | undefined;
  const draw = () => {
    waiting = undefined;
    const next = source();
    if (!box || next === shown) return;
    shown = next;
    last = performance.now();
    for (const block of watched) inView.unobserve(block);
    box.replaceChildren(prepared(html(next)));
    watched = [...box.querySelectorAll('.code[data-language]:not([data-language=""]):not([data-coloured])')];
    for (const block of watched) inView.observe(block);
  };
  createEffect(() => {
    if (source() === shown || waiting !== undefined) return;
    const since = performance.now() - last;
    if (since >= every) draw();
    else waiting = setTimeout(draw, every - since);
  });
  onCleanup(() => {
    clearTimeout(waiting);
    for (const block of watched) inView.unobserve(block);
  });
  return (el: HTMLDivElement) => {
    box = el;
    onClicks(el);
    draw();
  };
}

/** HTML that came from somewhere else, drawn: a ticket's text as Azure DevOps keeps it. Made safe first. */
export function Html(props: { html: string }) {
  return (
    <div
      class="markdown html"
      ref={drawn(
        () => props.html,
        (html) => DOMPurify.sanitize(html),
        0,
      )}
    />
  );
}

export function Markdown(props: { text: string }) {
  const html = (text: string) => DOMPurify.sanitize(marked.parse(text, { async: false, breaks: true }));
  return <div class="markdown" ref={drawn(() => props.text, html, 120)} />;
}
