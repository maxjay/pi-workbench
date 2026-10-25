// What you were writing in each scratchpad and haven't sent. Only the
// window needs it, so it's kept in the window's own storage, by the
// session's file, and is still there after the app restarts.

import { createStore } from 'solid-js/store';
import type { Session } from '../protocol/Session';

const STORAGE = 'pi-workbench.drafts';

function load(): Record<string, string> {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE) ?? '{}');
    return typeof saved === 'object' && saved !== null ? saved : {};
  } catch {
    return {};
  }
}

const [drafts, setDrafts] = createStore<Record<string, string>>(load());

export { drafts };

/** Where a session's draft is kept: its file once Pi has one, so it outlives Pi and the app. */
export const draftKey = (session: Session) => session.state.file ?? session.id;

let saving: ReturnType<typeof setTimeout> | undefined;

/** Keeps a draft, and saves them all a moment later. A session without a file yet isn't saved. */
export function setDraft(key: string, text: string) {
  setDrafts(key, text);
  clearTimeout(saving);
  saving = setTimeout(() => {
    const kept = Object.fromEntries(
      Object.entries(drafts).filter(([key, text]) => key.startsWith('/') && text.trim() !== ''),
    );
    try {
      localStorage.setItem(STORAGE, JSON.stringify(kept));
    } catch {
      // Storage can be full or off; the draft still stays for this run.
    }
  }, 300);
}
