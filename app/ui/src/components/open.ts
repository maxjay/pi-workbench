// Opens a link in your own browser. The workbench has none of its own.

export function openUrl(url: string) {
  if (!url.startsWith('https://')) return;
  if ('__TAURI_INTERNALS__' in window) {
    void import('@tauri-apps/api/core').then(({ invoke }) => invoke('open_url', { url }));
  } else {
    window.open(url, '_blank', 'noopener');
  }
}
