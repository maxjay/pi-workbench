// Downloads, from a button at the right of a web tab's toolbar: what's
// coming down, with how far it's got, and what came down lately. They go
// where Chromium puts them, your Downloads folder.

import { createSignal, For, Show } from 'solid-js';
import type { Download } from '../protocol/Download';
import { inApp } from '../state/app';
import { browser } from '../state/browser';
import { cover } from '../state/web-view';
import { Icon } from './Icon';

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function ago(at: number) {
  const minutes = Math.round((Date.now() - at) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  return hours < 24 ? `${hours} h ago` : new Date(at).toLocaleDateString();
}

function detail(d: Download) {
  if (d.state === 'cancelled') return 'Cancelled';
  if (d.state === 'going') return d.total > 0 ? `${size(d.received)} of ${size(d.total)}` : size(d.received);
  return `${size(d.total || d.received)} · ${ago(d.started)}`;
}

/** The Downloads folder, opened by the app on Windows. */
async function openFolder() {
  if (!inApp) return;
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('open_downloads').catch(() => {});
}

export function Downloads() {
  const [open, setOpen] = createSignal(false);
  cover(open);
  const going = () => browser.downloads.some((d) => d.state === 'going');
  return (
    <span class="downloads">
      <button
        type="button"
        class="icon-button"
        aria-label="Downloads"
        aria-expanded={open()}
        data-on={open()}
        onClick={() => setOpen(!open())}
      >
        <Icon name="download" size={16} />
        <Show when={going()}>
          <span class="downloads-dot" role="img" aria-label="Downloading" />
        </Show>
      </button>
      <Show when={open()}>
        <button
          type="button"
          class="scrim scrim-clear"
          aria-label="Close"
          tabIndex={-1}
          onClick={() => setOpen(false)}
        />
        <div class="menu downloads-menu" role="menu" aria-label="Downloads">
          <p class="menu-group">Downloads</p>
          <Show
            when={browser.downloads.length > 0}
            fallback={<p class="menu-empty">Nothing downloaded yet.</p>}
          >
            <For each={browser.downloads.slice(0, 8)}>
              {(d) => (
                <button type="button" role="menuitem" class="download" onClick={() => void openFolder()}>
                  <span class="download-kind mono" aria-hidden="true">
                    {(d.name.split('.').pop() ?? '').slice(0, 5).toUpperCase()}
                  </span>
                  <span class="download-text">
                    <span class="download-name">{d.name}</span>
                    <Show when={d.state === 'going' && d.total > 0}>
                      <span class="download-bar">
                        <span style={{ width: `${Math.round((d.received / d.total) * 100)}%` }} />
                      </span>
                    </Show>
                    <span class="download-detail">{detail(d)}</span>
                  </span>
                </button>
              )}
            </For>
          </Show>
          <Show when={inApp}>
            <div class="menu-line" />
            <button type="button" role="menuitem" class="menu-item" onClick={() => void openFolder()}>
              Open the Downloads folder
            </button>
          </Show>
        </div>
      </Show>
    </span>
  );
}
