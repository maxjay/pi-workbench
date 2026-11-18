// The form for a new pull request, in the middle. It starts from what dova
// would use itself: the newest commit's subject as the title. The work
// items linked to the branch are attached by dova when it is created.

import { createEffect, createResource, createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { createPull, pullBefore, pullStatus, written } from '../../state/pull';

export function NewPull(props: { dir: string; onDone: () => void }) {
  const [before] = createResource(
    () => props.dir,
    (dir) => pullBefore(dir),
  );
  const [status] = createResource(
    () => [props.dir, written()] as const,
    ([dir]) => pullStatus(dir, null),
  );
  const [title, setTitle] = createSignal('');
  const [body, setBody] = createSignal('');
  // The title starts as the newest commit's subject, until you type your own.
  let typed = false;
  createEffect(() => {
    const first = before.latest?.commits[0]?.subject;
    if (first && !typed) setTitle(first);
  });
  const creating = createAction(async (draft: boolean) => {
    await createPull(props.dir, title().trim(), body(), draft);
    props.onDone();
  });
  const ready = () => title().trim() !== '' && !creating.busy();

  return (
    <div class="new-pull">
      <p class="dim">
        {status.latest?.branch ?? 'this branch'}
        <Show when={before.latest}>{(b) => <> into {b().base.replace(/^origin\//, '')}</>}</Show> · not posted
        until you create it
      </p>

      <div class="field">
        <label for="pull-title">Title</label>
        <input
          id="pull-title"
          class="new-pull-title"
          value={title()}
          onInput={(e) => {
            typed = true;
            setTitle(e.currentTarget.value);
          }}
        />
      </div>

      <div class="field">
        <label for="pull-body">Description · markdown</label>
        <textarea
          id="pull-body"
          class="new-pull-body"
          rows={9}
          value={body()}
          onInput={(e) => setBody(e.currentTarget.value)}
        />
        <span class="dim">A picture named as ![text](file.png) is uploaded with the pull request.</span>
      </div>

      <div class="field">
        <span class="dim">Attached</span>
        <For each={status.latest?.workItems} fallback={<span class="dim">No work items linked.</span>}>
          {(item) => (
            <span>
              #{item.id} {item.title}
            </span>
          )}
        </For>
      </div>

      <Show when={creating.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
      <div class="new-pull-buttons">
        <button
          type="button"
          class="button button-primary"
          disabled={!ready()}
          onClick={() => void creating.run(false)}
        >
          Create pull request
        </button>
        <button type="button" class="button" disabled={!ready()} onClick={() => void creating.run(true)}>
          Create as draft
        </button>
      </div>
    </div>
  );
}
