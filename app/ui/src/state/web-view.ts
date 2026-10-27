// In the app, a web tab's page is Chromium's own view, which the app lays
// over the tab's page area. The page tells the app where that area is and
// whether it shows; anything drawn over it (Ctrl K, a menu) hides the view
// meanwhile, as nothing of the page's can draw over Chromium.

import { createEffect, createRoot, createSignal, onCleanup } from 'solid-js';

const [covers, setCovers] = createSignal(0);

/** Something is drawn over the page while `on`. */
export function cover(on: () => boolean) {
  createEffect(() => {
    if (!on()) return;
    setCovers((n) => n + 1);
    onCleanup(() => setCovers((n) => n - 1));
  });
}

interface Place {
  tab: string;
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
}

async function tell(place: Place) {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('browser_place', { place }).catch(() => {});
}

/** Keeps the app's view of page `tab` over `area` until the returned stop. */
export function place(tab: string, area: HTMLElement) {
  let last = '';
  const report = () => {
    const rect = area.getBoundingClientRect();
    const next: Place = {
      tab,
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      visible: covers() === 0 && rect.width > 0 && document.visibilityState === 'visible',
    };
    const key = JSON.stringify(next);
    if (key !== last) {
      last = key;
      void tell(next);
    }
  };
  const resized = new ResizeObserver(report);
  resized.observe(area);
  window.addEventListener('resize', report);
  document.addEventListener('visibilitychange', report);
  const dispose = createRoot((done) => {
    createEffect(() => {
      covers();
      report();
    });
    return done;
  });
  return () => {
    resized.disconnect();
    window.removeEventListener('resize', report);
    document.removeEventListener('visibilitychange', report);
    dispose();
    void tell({ tab, x: 0, y: 0, width: 0, height: 0, visible: false });
  };
}
