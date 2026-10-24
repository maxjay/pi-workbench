// What a page says when it has nothing to show, or something went wrong.

import { type JSX, Show } from 'solid-js';
import { describe } from './action';

export function ErrorNote(props: { error: unknown }) {
  return (
    <Show when={props.error}>
      <p class="note note-error" role="alert">
        {describe(props.error)}
      </p>
    </Show>
  );
}

export function Empty(props: { children: JSX.Element }) {
  return <p class="note">{props.children}</p>;
}
