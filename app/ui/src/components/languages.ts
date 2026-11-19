// The languages the app can read: for the editor, and for code in what
// the agent writes. Each is loaded the first time something needs it.

import type { LanguageSupport } from '@codemirror/language';

/** By a file's ending. */
export const languages: Record<string, () => Promise<LanguageSupport>> = {
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

/** What a block of code is called where it is written, as the ending that names it here. */
const NAMED: Record<string, string> = {
  javascript: 'js',
  node: 'js',
  typescript: 'ts',
  rust: 'rs',
  python: 'py',
  kotlin: 'kt',
  jsonc: 'json',
  markdown: 'md',
  scss: 'css',
  xml: 'html',
  svg: 'html',
};

/** The language a block of code says it is in, if the app can read it. */
export function languageNamed(name: string) {
  const known = name.trim().toLowerCase();
  return languages[NAMED[known] ?? known];
}
