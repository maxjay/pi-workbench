// A web tab: a page in the workbench's Chromium, with Pi beside it in the
// conversation that goes with the page. Pi's own pages arrive here from its
// card, with the session that was using them; a page of yours gets a
// conversation when you first ask about it. Hide Pi and it's a browser.
//
// In the app the page is Chromium's own view, laid over this tab's page
// area. In a plain browser, during development, the area shows the page as
// a picture, a moment behind.

import { createEffect, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../components/action';
import { ErrorNote } from '../components/Note';
import type { WebTab } from '../protocol/WebTab';
import { inApp } from '../state/app';
import { browser, chatHidden, pageById, pageTitle, setPageSession, shot } from '../state/browser';
import { follow, live, startBeside } from '../state/sessions';
import { place } from '../state/web-view';
import { Foot } from './chat/Foot';
import { Log } from './chat/Log';

export function WebPage(props: { id: string }) {
  const page = () => pageById(props.id);
  return (
    <div class="web">
      <Show
        when={browser.connected}
        fallback={
          <div class="web-page web-away">
            <p class="web-away-title">The browser isn’t running</p>
            <p class="web-away-text">
              {inApp
                ? 'It starts with the app. If it stopped, it starts again in a moment.'
                : 'wb-agent reaches a browser at WB_BROWSER, as host:port.'}
            </p>
            <Show when={browser.problem}>
              <p class="web-away-text">{browser.problem}</p>
            </Show>
          </div>
        }
      >
        <PageArea id={props.id} />
      </Show>
      <Show when={!chatHidden()}>
        <aside class="web-chat" aria-label="Pi beside the page">
          <Show when={page()} keyed>
            {(p) => <PageChat page={p} />}
          </Show>
        </aside>
      </Show>
    </div>
  );
}

/** Where the page shows: Chromium's view in the app, a picture of it elsewhere. */
function PageArea(props: { id: string }) {
  let area!: HTMLDivElement;
  const [picture, setPicture] = createSignal<string>();
  onMount(() => {
    if (inApp) {
      const stop = place(props.id, area);
      onCleanup(stop);
      return;
    }
    let going = true;
    const again = async () => {
      while (going) {
        const now = await shot(props.id).catch(() => null);
        const image = now?.image;
        if (image) setPicture(`data:${image.mimeType};base64,${image.data}`);
        await new Promise((r) => setTimeout(r, 1200));
      }
    };
    void again();
    onCleanup(() => {
      going = false;
    });
  });
  return (
    <div class="web-page" ref={area} data-pi={pageById(props.id)?.piDriving ?? false}>
      <Show when={!inApp}>
        <Show when={picture()} fallback={<p class="web-away-text">Loading…</p>}>
          {(src) => <img class="web-picture" alt={pageTitle(pageById(props.id))} src={src()} />}
        </Show>
      </Show>
    </div>
  );
}

/** The conversation beside a page: the session that goes with it, or a new one. */
function PageChat(props: { page: WebTab }) {
  const session = () => (props.page.session ? live[props.page.session] : undefined);
  // A session wb-agent runs that this page hasn't heard from yet.
  createEffect(() => {
    const id = props.page.session;
    if (id && !live[id]) void follow(id).catch(() => {});
  });
  return (
    <Show when={session()} keyed fallback={<StartBeside page={props.page} />}>
      {(s) => (
        <>
          <header class="web-chat-head">
            <span class="web-chat-title">{s.state.title || 'Pi'}</span>
            <span class="web-chat-meta">
              {props.page.piDriving ? 'Pi is using the page' : 'Pi can use this page'}
            </span>
          </header>
          <Log session={s} onOpenFile={() => {}} />
          <Foot session={s} />
        </>
      )}
    </Show>
  );
}

function StartBeside(props: { page: WebTab }) {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    const message = text().trim();
    if (!message) return;
    // Pi learns the page from its browser tools; this says which one.
    const asked = `On “${pageTitle(props.page)}” (${props.page.url}): ${message}`;
    const session = await startBeside(asked);
    await setPageSession(props.page.id, session.id);
    setText('');
  });
  const gone = () => props.page.session !== null && !live[props.page.session];
  return (
    <>
      <header class="web-chat-head">
        <span class="web-chat-title">Pi on this page</span>
      </header>
      <div class="log">
        <div class="log-content">
          <p class="log-empty">
            {gone()
              ? 'The conversation that went with this page has ended. Ask something to start another.'
              : 'Ask about this page, or tell Pi what to do on it. It uses the page through its browser tools.'}
          </p>
        </div>
      </div>
      <div class="conversation-foot">
        <ErrorNote error={start.error()} />
        <div class="composer">
          <div class="composer-box">
            <textarea
              class="composer-input composer-input-plain"
              rows={2}
              aria-label="Ask about this page"
              placeholder="Ask about this page, or tell Pi what to do on it…"
              value={text()}
              disabled={start.busy()}
              onInput={(e) => setText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  void start.run();
                }
              }}
            />
          </div>
        </div>
      </div>
    </>
  );
}
