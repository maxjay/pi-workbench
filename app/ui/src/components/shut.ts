// A list that is open shuts when you press somewhere else, or press Escape.

import { createEffect, onCleanup } from 'solid-js';

export function shutOnAway(open: () => boolean, box: () => HTMLElement | undefined, shut: () => void) {
  createEffect(() => {
    if (!open()) return;
    const away = (e: PointerEvent) => {
      if (!box()?.contains(e.target as Node)) shut();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') shut();
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', key);
    onCleanup(() => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', key);
    });
  });
}
