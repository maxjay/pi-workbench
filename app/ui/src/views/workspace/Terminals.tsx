// The terminals under the editor: a bar with one tab per terminal, and the
// one shown drawn by xterm.js. Each stays alive while it is open, so
// switching between them keeps what they showed.

import { FitAddon } from '@xterm/addon-fit';
import { Terminal as Xterm } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import type { Terminal } from '../../protocol/Terminal';
import { closeTerminal, draw, openTerminal, printed, resize, terminalsIn, type } from '../../state/terminals';

/** A token from theme.css, as the colour it stands for. xterm takes real values, not names. */
const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

export function Terminals(props: { dir: string }) {
  const list = () => terminalsIn(props.dir);
  const [picked, setPicked] = createSignal<string>();
  const [hidden, setHidden] = createSignal(false);
  /** The terminal shown: the one picked, or the newest. */
  const shown = () => list().find((t) => t.id === picked()) ?? list().at(-1);
  const add = createAction(async () => {
    const terminal = await openTerminal(props.dir);
    setPicked(terminal.id);
    setHidden(false);
  });

  return (
    <section class="terminals" aria-label="Terminals">
      <div class="bar strip terminals-bar">
        <For each={list()}>
          {(t) => (
            <span class="strip-tab strip-tab-closable" data-current={shown()?.id === t.id && !hidden()}>
              <button
                type="button"
                class="strip-open"
                onClick={() => {
                  setPicked(t.id);
                  setHidden(false);
                }}
              >
                {t.title}
                <Show when={t.running}>
                  <span class="dot" data-state="working" role="img" aria-label="Running" />
                </Show>
              </button>
              <button
                type="button"
                class="strip-close"
                aria-label={`Close ${t.title}`}
                onClick={() => void closeTerminal(t.id)}
              >
                ×
              </button>
            </span>
          )}
        </For>
        <button type="button" class="strip-tab" disabled={add.busy()} onClick={() => void add.run()}>
          New terminal
        </button>
        <span class="top-gap" />
        <Show when={add.error()}>{(error) => <span class="strip-note note-failed">{error()}</span>}</Show>
        <Show when={list().length > 0}>
          <button type="button" class="strip-tab" onClick={() => setHidden(!hidden())}>
            {hidden() ? 'Show' : 'Hide'}
          </button>
        </Show>
      </div>
      <div class="terminals-body" hidden={hidden() || list().length === 0}>
        <For each={list()}>{(t) => <View terminal={t} shown={shown()?.id === t.id && !hidden()} />}</For>
      </div>
    </section>
  );
}

function View(props: { terminal: Terminal; shown: boolean }) {
  let host!: HTMLDivElement;
  const id = props.terminal.id;
  const term = new Xterm({
    fontFamily: token('--mono'),
    fontSize: 13,
    lineHeight: 1.3,
    cursorBlink: true,
    scrollback: 5000,
    theme: {
      background: token('--paper'),
      foreground: token('--ink'),
      cursor: token('--ink'),
      selectionBackground: token('--term-selection'),
      black: token('--line'),
      red: token('--term-red'),
      green: token('--term-green'),
      yellow: token('--term-yellow'),
      blue: token('--term-blue'),
      magenta: token('--term-magenta'),
      cyan: token('--term-cyan'),
      white: token('--ink'),
      brightBlack: token('--dim'),
      brightRed: token('--term-red'),
      brightGreen: token('--term-green'),
      brightYellow: token('--term-yellow'),
      brightBlue: token('--term-blue'),
      brightMagenta: token('--term-magenta'),
      brightCyan: token('--term-cyan'),
      brightWhite: token('--ink'),
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  /** Fits the terminal to its box, and tells the shell its new size. */
  const refit = () => {
    if (!props.shown || host.clientWidth === 0) return;
    fit.fit();
    resize(id, term.cols, term.rows);
  };

  onMount(async () => {
    // The font has to be loaded before xterm measures a character.
    await document.fonts.load(`13px ${token('--mono').split(',')[0]}`).catch(() => {});
    term.open(host);
    term.onData((data) => type(id, data));
    // What it printed before this page drew it, then what it prints from now on.
    term.write(await printed(id).catch(() => ''));
    const stop = draw(id, (data) => term.write(data));
    const watcher = new ResizeObserver(refit);
    watcher.observe(host);
    refit();
    if (props.shown) term.focus();
    onCleanup(() => {
      stop();
      watcher.disconnect();
      term.dispose();
    });
  });

  createEffect(() => {
    if (!props.shown) return;
    requestAnimationFrame(() => {
      refit();
      term.focus();
    });
  });

  return <div class="terminal" ref={host} hidden={!props.shown} />;
}
