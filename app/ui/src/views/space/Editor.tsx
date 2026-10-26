// A codespace's editor, made once per codespace tab: the file you're on, editable, with what changed
// since the last commit marked in place, added lines tinted and removed
// ones shown struck through where they were. Ctrl S saves. A file that
// changes on disk while yours has nothing unsaved just shows the new text;
// when yours has unsaved changes, a bar asks which to keep.

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
import { Compartment, EditorState, type Extension } from '@codemirror/state';
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
import { ErrorNote } from '../../components/Note';
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
  kts: () => import('@codemirror/lang-java').then((m) => m.java()),
  json: () => import('@codemirror/lang-json').then((m) => m.json()),
  md: () => import('@codemirror/lang-markdown').then((m) => m.markdown()),
  css: () => import('@codemirror/lang-css').then((m) => m.css()),
  html: () => import('@codemirror/lang-html').then((m) => m.html()),
  yml: () => import('@codemirror/lang-yaml').then((m) => m.yaml()),
  yaml: () => import('@codemirror/lang-yaml').then((m) => m.yaml()),
};

const colours = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], color: '#c7a1e0' },
  { tag: [t.string, t.special(t.string), t.regexp], color: '#a8cf8e' },
  { tag: [t.number, t.bool, t.null, t.atom], color: '#e0b15c' },
  { tag: [t.comment, t.lineComment, t.blockComment], color: '#80868c', fontStyle: 'italic' },
  { tag: [t.typeName, t.className, t.namespace], color: '#7fc4d8' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: '#8fb8e8' },
  { tag: [t.definition(t.variableName), t.propertyName], color: '#e8e6e1' },
  { tag: [t.heading], color: '#e8e6e1', fontWeight: '600' },
  { tag: [t.link, t.url], color: '#7cc4a8' },
  { tag: [t.meta, t.annotation], color: '#e9968b' },
  { tag: t.invalid, color: '#e9968b' },
]);

const look = EditorView.theme(
  {
    '&': { height: '100%', backgroundColor: 'transparent', color: '#d6d4cf' },
    '.cm-scroller': { fontFamily: 'var(--mono)', fontSize: '13px', lineHeight: '22px' },
    '.cm-content': { caretColor: '#e8e6e1', padding: '0 0 24px' },
    '.cm-cursor': { borderLeftColor: '#e8e6e1' },
    '.cm-gutters': { backgroundColor: 'transparent', border: 'none', color: '#8a9096' },
    '.cm-lineNumbers .cm-gutterElement': { padding: '0 16px 0 20px', minWidth: '52px' },
    '.cm-activeLine': { backgroundColor: 'rgba(255, 255, 255, 0.03)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: '#c9c7c2' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'rgba(124, 196, 168, 0.22) !important',
    },
    '.cm-selectionMatch': { backgroundColor: 'rgba(255, 255, 255, 0.08)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-panels': { backgroundColor: '#363b40', color: '#e8e6e1' },
    '.cm-searchMatch': { backgroundColor: 'rgba(224, 177, 92, 0.25)' },
    // Since the last commit: added lines tinted, removed ones struck through.
    '.cm-changedLine': { backgroundColor: 'rgba(124, 196, 168, 0.07)' },
    '.cm-changedText': { backgroundColor: 'transparent' },
    '.cm-deletedChunk': {
      backgroundColor: 'rgba(224, 122, 110, 0.06)',
      color: '#8c6f6b',
      textDecoration: 'line-through',
    },
    '.cm-deletedChunk .cm-deletedText': { backgroundColor: 'transparent' },
    '.cm-changeGutter': { width: '3px', paddingLeft: '0' },
    '.cm-changedLineGutter': { backgroundColor: '#7cc4a8' },
    '.cm-deletedLineGutter': { backgroundColor: '#e07a6e' },
  },
  { dark: true },
);

const ext = (path: string) => path.split('/').pop()?.split('.').pop()?.toLowerCase() ?? '';

export function createEditor(props: {
  path: () => string | undefined;
  /** Changes when git does, so changes are marked again. */
  gitKey: () => string;
  /** Whether Pi wrote a file, for saying who changed it. */
  byPi: (path: string) => boolean;
  onSaved: () => void;
}) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  /** Each file's editor state, kept while its tab is open, and what's in it. */
  const states = new Map<string, EditorState>();
  const parts = new Map<string, { language: Compartment; changes: Compartment; readOnly: Compartment }>();
  const [loading, setLoading] = createSignal(false);
  const saving = createAction((path: string, text: string) => save(path, text));
  let shown: string | undefined;

  const extensionsFor = (path: string): Extension[] => {
    const part = { language: new Compartment(), changes: new Compartment(), readOnly: new Compartment() };
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
      part.readOnly.of(EditorState.readOnly.of(false)),
    ];
  };

  /** Puts new text in a file's editor, keeping its history. */
  const replace = (path: string, text: string) => {
    const state = states.get(path);
    if (!state) return;
    const change = { changes: { from: 0, to: state.doc.length, insert: text } };
    if (view && shown === path) view.dispatch(change);
    else states.set(path, state.update(change).state);
  };

  /** Marks the changes since the last commit, against the file as it has it. */
  const markChanges = async (path: string) => {
    const was = await original(path).catch(() => null);
    const part = parts.get(path);
    if (!part) return;
    const effect = part.changes.reconfigure(
      was === null || was === undefined
        ? []
        : unifiedMergeView({
            original: was,
            mergeControls: false,
            highlightChanges: false,
            gutter: true,
            syntaxHighlightDeletions: true,
          }),
    );
    if (view && shown === path) view.dispatch({ effects: effect });
    else {
      const state = states.get(path);
      if (state) states.set(path, state.update({ effects: effect }).state);
    }
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
    if (buffer.content.type !== 'text' || props.path() !== path) return;
    let state = states.get(path);
    if (!state) {
      state = EditorState.create({ doc: buffer.disk, extensions: extensionsFor(path) });
      states.set(path, state);
      const part = parts.get(path);
      if (part && buffer.content.truncated)
        state = state.update({ effects: part.readOnly.reconfigure(EditorState.readOnly.of(true)) }).state;
      states.set(path, state);
      const language = languages[ext(path)];
      if (language)
        void language().then((support) => {
          const p = parts.get(path);
          if (!p) return;
          const effect = p.language.reconfigure(support);
          if (view && shown === path) view.dispatch({ effects: effect });
          else {
            const s = states.get(path);
            if (s) states.set(path, s.update({ effects: effect }).state);
          }
        });
      void markChanges(path);
    }
    if (!view) return;
    shown = path;
    view.setState(state);
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

  // Files no longer open let go of their editors.
  const drop = (path: string) => {
    states.delete(path);
    parts.delete(path);
  };

  const buffer = () => {
    const path = props.path();
    return path ? buffers[path] : undefined;
  };
  const pi = () => {
    const path = props.path();
    return path ? props.byPi(path) : false;
  };
  const kind = () => buffer()?.content.type;

  return {
    replace,
    drop,
    save: () => {
      const path = shown;
      if (path && view) void saving.run(path, view.state.doc.toString()).then(() => props.onSaved());
    },
    goToLine: (n: number) => {
      if (!view) return;
      const line = view.state.doc.line(Math.max(1, Math.min(n, view.state.doc.lines)));
      view.dispatch({ selection: { anchor: line.from }, scrollIntoView: true });
      view.focus();
    },
    view: (
      <>
        <Show when={buffer()?.changedOnDisk != null}>
          <div class="editor-bar-note" role="alert">
            <span>
              {pi() ? 'Pi changed this file' : 'This file changed on disk'} while yours had unsaved changes.
            </span>
            <button
              type="button"
              class="button button-small"
              onClick={() => {
                const path = props.path();
                const text = path ? takeTheirs(path) : undefined;
                if (path && text !== undefined) replace(path, text);
              }}
            >
              {pi() ? 'Take Pi’s' : 'Take the new one'}
            </button>
            <button
              type="button"
              class="button button-text button-small"
              onClick={() => {
                const path = props.path();
                if (path) keepMine(path);
              }}
            >
              Keep mine
            </button>
          </div>
        </Show>
        <ErrorNote error={saving.error()} />
        <Show
          when={(() => {
            const content = buffer()?.content;
            return content?.type === 'text' && content.truncated;
          })()}
        >
          <p class="editor-note">Only the start of this large file is shown, so it can’t be edited here.</p>
        </Show>
        <div class="editor-body" ref={host} data-hidden={!props.path() || kind() !== 'text'} />
        <Show when={props.path() && kind() && kind() !== 'text'}>
          <div class="editor-empty">
            <Show
              when={buffer()?.content.type === 'image' && buffer()?.content}
              fallback={
                <>
                  <strong class="editor-empty-title">
                    {kind() === 'missing' ? 'This file isn’t there any more' : 'This file isn’t text'}
                  </strong>
                  <span>
                    {kind() === 'missing' ? 'It was moved or deleted.' : 'It can’t be shown or edited here.'}
                  </span>
                </>
              }
            >
              {(image) => {
                const content = image() as { mimeType: string; data: string };
                return (
                  <img class="editor-image" alt="" src={`data:${content.mimeType};base64,${content.data}`} />
                );
              }}
            </Show>
          </div>
        </Show>
        <Show when={!props.path()}>
          <div class="editor-empty">
            <strong class="editor-empty-title">No file open</strong>
            <span>
              Pick one on the left, or press <span class="mono editor-empty-key">Ctrl P</span>.
            </span>
          </div>
        </Show>
        <Show when={loading()}>
          <div class="editor-loading" />
        </Show>
      </>
    ),
  };
}
