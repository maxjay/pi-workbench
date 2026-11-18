// The 4x4 mark for a name, the same one maxjay.dev draws: the name is
// hashed (FNV-1a), and the low 16 bits say which of the 16 cells are filled.

import { For } from 'solid-js';

function fnv(text: string) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

/** The filled cells of a name's mark, as numbers 0 to 15, left to right, top to bottom. */
export function cells(name: string) {
  const hash = fnv(name);
  return Array.from({ length: 16 }, (_, i) => i).filter((i) => (hash >>> i) & 1);
}

export function Glyph(props: { name: string; size: number }) {
  return (
    <svg
      class="glyph"
      viewBox="0 0 4 4"
      width={props.size}
      height={props.size}
      shape-rendering="crispEdges"
      aria-hidden="true"
    >
      <For each={cells(props.name)}>{(i) => <rect x={i % 4} y={i >> 2} width="1" height="1" />}</For>
    </svg>
  );
}
