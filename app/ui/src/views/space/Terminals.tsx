// A codespace's terminals, under the editor: a tab each, with a dot while
// something runs in it, a new one in the folder, and a button to hide them.
// They're yours and Pi's alike: Pi reads them and runs commands in them.

import { FitAddon } from '@xterm/addon-fit';
import { Terminal as Xterm } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { createEffect, createSignal, For, on, onCleanup, onMount } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Terminal } from '../../protocol/Terminal';
import {
  attach,
  closeTerminal,
  loadTerminals,
  openTerminal,
  resize,
  terminalsIn,
  typeIn,
} from '../../state/terminals';
import { setUi, uiOf } from './ui';

/** Codespaces whose first terminal has been opened, so closing it doesn't open another. */
const started = new Set<string>();

export function Terminals(props: { space: string; dir: string }) {
  const ui = () => uiOf(props.space);
  const list = () => terminalsIn(props.dir);
  const current = () => list().find((t) => t.id === ui().terminal) ?? list()[0];
  const open = () => !ui().termShut && list().length > 0;
  const add = createAction(async () => {
    const terminal = await openTerminal(props.dir);
    setUi(props.space, { terminal: terminal.id, termShut: false });
  });
  const close = createAction((id: string) => closeTerminal(id));

  onMount(async () => {
    const known = await loadTerminals(props.dir).catch(() => []);
    if (known.length === 0 && !started.has(props.space)) {
      started.add(props.space);
      await add.run();
    }
    started.add(props.space);
  });

  return (
    <section class="terminals" aria-label="Terminals" data-open={open()}>
      <div class="terminals-bar" role="tablist" aria-label="Terminals">
        <For each={list()}>
          {(t) => (
            <div class="terminal-tab" data-current={current()?.id === t.id}>
              <button
                type="button"
                role="tab"
                class="terminal-tab-main"
                aria-selected={current()?.id === t.id}
                title={t.running ? `${t.title} is running` : 'A shell in this folder'}
                onClick={() => setUi(props.space, { terminal: t.id, termShut: false })}
              >
                <span class="terminal-dot" data-running={t.running} aria-hidden="true" />
                {t.title}
              </button>
              <button
                type="button"
                class="terminal-close"
                aria-label={`Close ${t.title}`}
                title={t.running ? 'Close it, stopping what runs in it' : 'Close it'}
                onClick={() => void close.run(t.id)}
              >
                <Icon name="close" size={9} />
              </button>
            </div>
          )}
        </For>
        <button
          type="button"
          class="icon-button icon-button-small"
          aria-label="New terminal"
          title="New terminal in this folder (Ctrl Shift `)"
          disabled={add.busy()}
          onClick={() => void add.run()}
        >
          <Icon name="plus" size={12} />
        </button>
        <span class="terminals-gap" />
        <span class="terminals-pi" title="Pi reads what they show and can run commands in them">
          Pi can see these
        </span>
        <button
          type="button"
          class="icon-button icon-button-small"
          aria-expanded={open()}
          aria-label={open() ? 'Hide the terminals' : 'Show the terminals'}
          title={open() ? 'Hide the terminals' : 'Show the terminals'}
          onClick={() => setUi(props.space, 'termShut', open())}
        >
          <span class="chevron" data-open={!open()}>
            <Icon name="chevron" size={10} />
          </span>
        </button>
      </div>
      <ErrorNote error={add.error() ?? close.error()} />
      <div class="terminals-body" data-hidden={!open()}>
        <For each={list()}>{(t) => <TermView terminal={t} shown={open() && current()?.id === t.id} />}</For>
      </div>
    </section>
  );
}

/** One terminal, drawn by xterm.js, kept while it's open so switching keeps what it showed. */
function TermView(props: { terminal: Terminal; shown: boolean }) {
  let host!: HTMLDivElement;
  const [size, setSize] = createSignal({ cols: 0, rows: 0 });
  const term = new Xterm({
    fontFamily: 'Cascadia Mono, Cascadia Code, Consolas, ui-monospace, monospace',
    fontSize: 12.5,
    lineHeight: 1.25,
    cursorBlink: true,
    allowProposedApi: false,
    scrollback: 5000,
    theme: {
      background: '#272a2e',
      foreground: '#d6d4cf',
      cursor: '#c9c7c2',
      selectionBackground: 'rgba(124, 196, 168, 0.3)',
      black: '#2e3236',
      red: '#e9968b',
      green: '#7cc4a8',
      yellow: '#e0b15c',
      blue: '#6fa8dc',
      magenta: '#c7a1e0',
      cyan: '#7fc4d8',
      white: '#d6d4cf',
      brightBlack: '#80868c',
      brightRed: '#f0aca2',
      brightGreen: '#93d4ba',
      brightYellow: '#ecc67e',
      brightBlue: '#8fbde6',
      brightMagenta: '#d6b8ea',
      brightCyan: '#9dd3e3',
      brightWhite: '#f4f2ed',
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // Ctrl C copies what's selected, as in an editor's terminal; with nothing
  // selected it goes to the shell. Ctrl Shift C always copies.
  term.attachCustomKeyEventHandler((e) => {
    const copy = e.type === 'keydown' && e.ctrlKey && e.key.toLowerCase() === 'c';
    if (copy && (e.shiftKey || term.hasSelection())) {
      void navigator.clipboard.writeText(term.getSelection()).catch(() => {});
      term.clearSelection();
      return false;
    }
    return true;
  });

  onMount(() => {
    term.open(host);
    let stop: (() => void) | undefined;
    void attach(props.terminal.id, (data) => term.write(data)).then((off) => {
      stop = off;
    });
    const typed = term.onData((data) => void typeIn(props.terminal.id, data).catch(() => {}));
    const sized = term.onResize(({ cols, rows }) => setSize({ cols, rows }));
    const watch = new ResizeObserver(() => {
      if (props.shown && host.clientWidth > 0) fit.fit();
    });
    watch.observe(host);
    onCleanup(() => {
      watch.disconnect();
      typed.dispose();
      sized.dispose();
      stop?.();
      term.dispose();
    });
  });

  // Fits when shown, and tells the shell its size, so lines wrap where they should.
  createEffect(
    on(
      () => props.shown,
      (shown) => {
        if (!shown) return;
        requestAnimationFrame(() => {
          if (host.clientWidth > 0) fit.fit();
          term.focus();
        });
      },
    ),
  );
  createEffect(
    on(size, ({ cols, rows }) => {
      if (cols > 0 && rows > 0) void resize(props.terminal.id, cols, rows).catch(() => {});
    }),
  );

  return <div class="terminal-view" ref={host} data-hidden={!props.shown} />;
}
