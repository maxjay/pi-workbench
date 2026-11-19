// The theme: System, Light or Dark, as picked in Settings and kept on this
// machine. System follows Windows. theme.css has the colours of both.

import { createSignal } from 'solid-js';

export type Theme = 'system' | 'light' | 'dark';

const kept = localStorage.getItem('theme');
const [theme, setThemeNow] = createSignal<Theme>(kept === 'light' || kept === 'dark' ? kept : 'system');

export { theme };

/** Puts the picked theme on the page, where theme.css reads it. With none there, the page follows Windows. */
export function showTheme() {
  if (theme() === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme();
}

export function setTheme(to: Theme) {
  localStorage.setItem('theme', to);
  // The page changes first, so whoever hears of the change reads the new colours.
  setThemeNow(to);
  showTheme();
  setLight(isLight());
}

const windows = matchMedia('(prefers-color-scheme: light)');
const isLight = () => theme() === 'light' || (theme() === 'system' && windows.matches);

/**
 * Whether the page is light now. What cannot read the tokens by itself,
 * like a terminal, takes its colours again when this changes.
 */
const [light, setLight] = createSignal(isLight());
windows.addEventListener('change', () => setLight(isLight()));

export { light };
