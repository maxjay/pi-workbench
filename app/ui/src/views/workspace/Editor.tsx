// A workspace's editor, made once per workspace tab: the file you are on,
// editable, with what changed since the last commit marked in place. Ctrl S
// saves. A file that changes on disk while yours has nothing unsaved just
// shows the new text. When yours has unsaved changes, a bar asks which to keep.

import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import {
  bracketMatching,
  HighlightStyle,
  indentOnInput,
  type LanguageSupport,
  syntaxHighlighting,
} from '@codemirror/language';
import { unifiedMergeView } from '@codemirror/merge';
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
import { Compartment, EditorState, type Extension, type TransactionSpec } from '@codemirror/state';
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from '@codemirror/view';
import { tags as t } from '@lezer/highlight';
import { createEffect, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { buffers, keepMine, load, save, setBuffers, takeTheirs } from '../../state/editor';
import { original } from '../../state/repo';

/** Each language, loaded the first time a file needs it. */
const languages: Record<string, () => Promise<LanguageSupport>> = {
  js: () => import('@codemirror/lang-javascript').then((m) => m.javascript()),
  jsx: () => import('@codemirror/lang-javascript').then((m) => m.javascript({ jsx: true })),
  ts: () => import('@codemirror/lang-javascript').then((m) => m.javascript({ typescript: true })),
  tsx: () => import('@codemirror/lang-javascript').then((m) => m.javascript({ typescript: true, jsx: true })),
  mjs: () => import('@codemirror/lang-javascript').then((m) => m.javascript()),
  rs: () => import('@codemirror/lang-rust').then((m) => m.rust()),
  py: () => import('@codemirror/lang-python').then((m) => m.python()),
  java: () => import('@codemirror/lang-java').then((m) => m.java()),
  kt: () => import('@codemirror/lang-java').then((m) => m.java()),
  json: () => import('@codemirror/lang-json').then((m) => m.json()),
  md: () => import('@codemirror/lang-markdown').then((m) => m.markdown()),
  css: () => import('@codemirror/lang-css').then((m) => m.css()),
  html: () => import('@codemirror/lang-html').then((m) => m.html()),
  yml: () => import('@codemirror/lang-yaml').then((m) => m.yaml()),
  yaml: () => import('@codemirror/lang-yaml').then((m) => m.yaml()),
};

// The colours are theme.css's tokens, so none is written twice.
const colours = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], color: 'var(--syn-keyword)' },
  { tag: [t.string, t.special(t.string), t.regexp], color: 'var(--syn-string)' },
  { tag: [t.number, t.bool, t.null, t.atom], color: 'var(--syn-number)' },
  { tag: [t.comment, t.lineComment, t.blockComment], color: 'var(--dim)', fontStyle: 'italic' },
  { tag: [t.typeName, t.className, t.namespace], color: 'var(--syn-type)' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: 'var(--syn-function)' },
  { tag: [t.heading], fontWeight: '600' },
  { tag: [t.link, t.url], textDecoration: 'underline' },
  { tag: [t.meta, t.annotation], color: 'var(--syn-number)' },
  { tag: t.invalid, color: 'var(--failed)' },
]);

const look = EditorView.theme(
  {
    '&': { height: '100%', backgroundColor: 'transparent', color: 'var(--ink)' },
    '.cm-scroller': { fontFamily: 'var(--mono)', fontSize: '13px', lineHeight: '22px' },
    '.cm-content': { caretColor: 'var(--ink)', padding: '16px 0 24px' },
    '.cm-cursor': { borderLeftColor: 'var(--ink)' },
    '.cm-gutters': { backgroundColor: 'transparent', border: 'none', color: 'var(--dim)' },
    '.cm-lineNumbers .cm-gutterElement': { padding: '0 14px 0 20px', minWidth: '56px' },
    '.cm-activeLine': { backgroundColor: 'var(--hover)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--ink)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'rgb(236 235 228 / 0.18) !important',
    },
    '.cm-selectionMatch': { backgroundColor: 'var(--picked)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-panels': {
      backgroundColor: 'var(--paper)',
      color: 'var(--ink)',
      borderTop: '1px solid var(--line)',
    },
    '.cm-searchMatch': { outline: '1px solid var(--working)' },
    // Since the last commit: added lines tinted, removed ones struck through.
    '.cm-changedLine': { backgroundColor: 'var(--hover)' },
    '.cm-changedText': { backgroundColor: 'transparent' },
    '.cm-deletedChunk': {
      backgroundColor: 'transparent',
      color: 'var(--dim)',
      textDecoration: 'line-through',
    },
    '.cm-deletedChunk .cm-deletedText': { backgroundColor: 'transparent' },
    '.cm-changeGutter': { width: '4px', paddingLeft: '0' },
    '.cm-changedLineGutter': { backgroundColor: 'var(--ok)' },
    '.cm-deletedLineGutter': { backgroundColor: 'var(--failed)' },
  },
  { dark: true },
);

const ext = (path: string) => path.split('/').pop()?.split('.').pop()?.toLowerCase() ?? '';

interface Parts {
  language: Compartment;
  changes: Compartment;
}

export function createEditor(props: {
  /** The file shown, as an absolute path. Undefined when the tab shown isn't a file. */
  path: () => string | undefined;
  /** Changes when git does, so changes are marked again. */
  gitKey: () => string;
  onSaved: () => void;
}) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  /** Each open file's editor state, and the parts of it that change later. */
  const states = new Map<string, EditorState>();
  const parts = new Map<string, Parts>();
  const [loading, setLoading] = createSignal(false);
  const saving = createAction((path: string, text: string) => save(path, text));
  let shown: string | undefined;
  /** A line to go to in a file, once that file is on screen. */
  let wanted: { path: string; line: number } | undefined;

  const goTo = (line: number) => {
    if (!view) return;
    const at = view.state.doc.line(Math.max(1, Math.min(line, view.state.doc.lines)));
    view.dispatch({ selection: { anchor: at.from }, scrollIntoView: true });
  };

  /** Changes a file's editor: the one on screen, or the state kept for later. */
  const apply = (path: string, spec: TransactionSpec) => {
    if (view && shown === path) view.dispatch(spec);
    else {
      const state = states.get(path);
      if (state) states.set(path, state.update(spec).state);
    }
  };

  const extensionsFor = (path: string): Extension[] => {
    const part = { language: new Compartment(), changes: new Compartment() };
    parts.set(path, part);
    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      syntaxHighlighting(colours),
      keymap.of([
        {
          key: 'Mod-s',
          preventDefault: true,
          run: (v) => {
            void saving.run(path, v.state.doc.toString()).then(() => props.onSaved());
            return true;
          },
        },
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        indentWithTab,
      ]),
      look,
      part.language.of([]),
      part.changes.of([]),
    ];
  };

  /** Puts new text in a file's editor, keeping its history. */
  const replace = (path: string, text: string) => {
    const state = states.get(path);
    if (state) apply(path, { changes: { from: 0, to: state.doc.length, insert: text } });
  };

  /** Marks the changes since the last commit, against the file as it has it. */
  const markChanges = async (path: string) => {
    const was = await original(path).catch(() => null);
    const part = parts.get(path);
    if (!part) return;
    const marked =
      was === null
        ? []
        : unifiedMergeView({
            original: was,
            mergeControls: false,
            highlightChanges: false,
            gutter: true,
            syntaxHighlightDeletions: true,
          });
    apply(path, { effects: part.changes.reconfigure(marked) });
  };

  const open = async (path: string) => {
    let buffer = buffers[path];
    if (!buffer) {
      setLoading(true);
      try {
        buffer = await load(path);
      } finally {
        setLoading(false);
      }
    }
    if (buffer.content.kind !== 'text' || props.path() !== path) return;
    let state = states.get(path);
    if (!state) {
      state = EditorState.create({
        doc: buffer.disk,
        extensions: [extensionsFor(path), EditorState.readOnly.of(buffer.content.truncated)],
      });
      states.set(path, state);
      void languages[ext(path)]?.().then((support) => {
        const part = parts.get(path);
        if (part) apply(path, { effects: part.language.reconfigure(support) });
      });
      void markChanges(path);
    }
    if (!view) return;
    shown = path;
    view.setState(states.get(path) ?? state);
    if (wanted?.path === path) goTo(wanted.line);
    wanted = undefined;
    view.focus();
  };

  onMount(() => {
    view = new EditorView({
      parent: host,
      dispatchTransactions(transactions, v) {
        v.update(transactions);
        if (!shown) return;
        states.set(shown, v.state);
        if (transactions.some((tr) => tr.docChanged) && buffers[shown])
          setBuffers(shown, 'dirty', v.state.doc.toString() !== buffers[shown].disk);
      },
    });
    onCleanup(() => view?.destroy());
    const first = props.path();
    if (first) void open(first);
  });

  createEffect(
    on(
      () => props.path(),
      (path) => {
        if (path && view) void open(path);
        if (!path) shown = undefined;
      },
      { defer: true },
    ),
  );

  // Git moved on, or a file was saved: mark changes against the commit again.
  createEffect(
    on(
      () => props.gitKey(),
      () => {
        for (const path of states.keys()) void markChanges(path);
      },
      { defer: true },
    ),
  );

  const buffer = () => {
    const path = props.path();
    return path ? buffers[path] : undefined;
  };
  const kind = () => buffer()?.content.kind;
  const truncated = () => {
    const content = buffer()?.content;
    return content?.kind === 'text' && content.truncated;
  };

  return {
    replace,
    /** Goes to a line of a file: now if it is the one shown, else when it opens. */
    goToLine: (path: string, line: number) => {
      if (shown === path) goTo(line);
      else wanted = { path, line };
    },
    /** A closed file lets go of its editor. */
    drop: (path: string) => {
      states.delete(path);
      parts.delete(path);
    },
    view: (
      <div class="editor" hidden={props.path() === undefined}>
        <Show when={buffer()?.changedOnDisk != null}>
          <div class="editor-note" role="alert">
            <span>This file changed on disk while yours had unsaved changes.</span>
            <button
              type="button"
              class="button"
              onClick={() => {
                const path = props.path();
                const text = path ? takeTheirs(path) : undefined;
                if (path && text !== undefined) replace(path, text);
              }}
            >
              Take the new one
            </button>
            <button
              type="button"
              class="button"
              onClick={() => {
                const path = props.path();
                if (path) keepMine(path);
              }}
            >
              Keep mine
            </button>
          </div>
        </Show>
        <Show when={saving.error()}>
          {(error) => (
            <div class="editor-note note-failed" role="alert">
              Couldn't save: {error()}
            </div>
          )}
        </Show>
        <Show when={truncated()}>
          <div class="editor-note">This file is too long to edit here. The start of it is shown.</div>
        </Show>
        <Show when={kind() === 'binary'}>
          <p class="editor-empty">Not a text file.</p>
        </Show>
        <Show when={kind() === 'missing'}>
          <p class="editor-empty">This file is not there any more.</p>
        </Show>
        <Show when={loading()}>
          <p class="editor-empty">Opening</p>
        </Show>
        <div class="editor-host" ref={host} data-hidden={kind() !== 'text'} />
      </div>
    ),
  };
}
