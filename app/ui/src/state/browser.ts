// The browser, as wb-agent follows it: every page, yours and Pi's, and
// downloads. Your pages are web tabs in the strip; a page Pi opens isn't,
// until you open it from Pi's card. The address bar's suggestions come
// from your history.

import { createRoot, createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { ask, onEvent, status } from '../agent';
import type { Browser } from '../protocol/Browser';
import type { Visit } from '../protocol/Visit';
import type { WebTab } from '../protocol/WebTab';
import { active, close as closeTab, kindOf, open, type TabId, tabs } from './tabs';

const empty: Browser = { connected: false, tabs: [], downloads: [], server: false, connections: 0 };
const [browser, setBrowser] = createStore<Browser>(empty);

export { browser };

export const webTab = (id: TabId) => (id.startsWith('web:') ? id.slice(4) : undefined);

export function pageById(id: string): WebTab | undefined {
  return browser.tabs.find((tab) => tab.id === id);
}

/** The page the active tab shows, if it's a web tab. */
export function activePage(): WebTab | undefined {
  const id = webTab(active());
  return id ? pageById(id) : undefined;
}

/** What a page is called: its title, or its address while it has none. */
export function pageTitle(page: WebTab | undefined) {
  if (!page) return 'Web';
  const title = page.title.trim();
  return title && title !== page.url ? title : page.url.replace(/^https?:\/\//, '') || 'New tab';
}

/** An address as typed: a URL with its scheme, or a search when it isn't one. */
export function addressOf(typed: string) {
  const text = typed.trim();
  if (/^[a-z]+:\/\//i.test(text) || text.startsWith('about:') || text.startsWith('chrome:')) return text;
  if (/^[^\s]+\.[^\s]+$/.test(text) || /^localhost(:\d+)?(\/|$)/.test(text))
    return `https://${text}`.replace(/^https:\/\/(localhost|127\.0\.0\.1)/, 'http://$1');
  return `https://duckduckgo.com/?q=${encodeURIComponent(text)}`;
}

/** Your strip has a tab for every page that's shown, and none for a page that's gone. */
function keepStrip(next: Browser) {
  const pages = new Set(next.tabs.map((tab) => tab.id));
  for (const tab of tabs()) {
    const id = webTab(tab);
    if (id && !pages.has(id) && next.connected) closeTab(tab);
  }
  for (const page of next.tabs) {
    const tab: TabId = `web:${page.id}`;
    if (page.shown && !tabs().includes(tab)) open(tab, { focus: false });
  }
}

createRoot(() => {
  const load = async () => {
    if (status() !== 'ready') return;
    const next = (await ask({ method: 'browser.get' }, 'browser')) as unknown as Browser;
    setBrowser(reconcile(next));
    keepStrip(next);
  };
  onEvent((event) => {
    if (event.event === 'ready') void load();
    if (event.event === 'browser') {
      setBrowser(reconcile(event.browser));
      keepStrip(event.browser);
    }
  });
  void load();
});

/** Opens a page in a new tab of yours, with `session` beside it. */
export async function openPage(url: string, session: string | null = null) {
  const page = (await ask(
    { method: 'browser.open', params: { url: addressOf(url), session } },
    'webTab',
  )) as WebTab;
  open(`web:${page.id}`);
  return page;
}

/** Opens a page Pi has, from its card: it joins your strip. */
export async function showPage(id: string) {
  await ask({ method: 'browser.show', params: { tab: id } }, 'done');
  open(`web:${id}`);
}

export const navigate = (tab: string, typed: string) =>
  ask({ method: 'browser.navigate', params: { tab, url: addressOf(typed) } }, 'done');
export const goBack = (tab: string) => ask({ method: 'browser.go', params: { tab, by: -1 } }, 'done');
export const goForward = (tab: string) => ask({ method: 'browser.go', params: { tab, by: 1 } }, 'done');
export const reload = (tab: string) => ask({ method: 'browser.reload', params: { tab } }, 'done');
export const closePage = (tab: string) => ask({ method: 'browser.close', params: { tab } }, 'done');
export const setPageSession = (tab: string, session: string | null) =>
  ask({ method: 'browser.setSession', params: { tab, session } }, 'done');
export const shot = (tab: string) => ask({ method: 'browser.shot', params: { tab } }, 'shot');
export const addServer = () => ask({ method: 'browser.addServer' }, 'browser');

// Suggestions for what's typed in the address bar, newest answer wins.
const [suggestions, setSuggestions] = createSignal<Visit[]>([]);
let asked = 0;

export { suggestions };

export async function suggest(query: string) {
  const mine = ++asked;
  if (!browser.connected) return setSuggestions([]);
  const found = await ask({ method: 'browser.history', params: { query } }, 'visits').catch(() => null);
  if (found && mine === asked) setSuggestions(found.list);
}

/** Whether the active tab is a web tab. */
export const onWeb = () => kindOf(active()) === 'web';

// Whether Pi's chat sits beside web pages, or they're just a browser.
const [chatHidden, setChatHidden] = createSignal(false);

export { chatHidden };
export const toggleChat = () => setChatHidden(!chatHidden());

/** A web tab closed: its page closes with it, unless it's already gone. */
export function tabClosed(id: string) {
  if (pageById(id)) void closePage(id).catch(() => {});
}
