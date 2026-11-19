// A review thread, and what the agent answered you. Both are drawn where
// they belong: under a line of the diff, or in the overview. What the agent
// wrote is yours alone until you post it.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Markdown } from '../../components/Markdown';
import type { Thread } from '../../protocol/Thread';
import { comment, dropNote, type Note, reply, setThread } from '../../state/review';
import { placeName, type Reviewing, when } from './parts';

export function ThreadBox(props: {
  r: Reviewing;
  thread: Thread;
  /** The reply the agent drafted for this thread, if you asked for one. */
  draft: Note | undefined;
  /** Says where the thread is, when it isn't drawn under its line. */
  showPlace?: boolean;
}) {
  const [text, setText] = createSignal('');
  const thread = () => props.thread;
  const replying = createAction(async () => {
    const words = text().trim();
    if (!words) return;
    await reply(props.r.dir, props.r.pr, thread().id, words);
    setText('');
  });
  const setting = createAction((status: 'resolved' | 'active') =>
    setThread(props.r.dir, props.r.pr, thread().id, status),
  );
  const askForReply = () => {
    const place = thread().place;
    const said = thread()
      .comments.map((c) => `- ${c.author ?? 'Someone'}: ${c.text}`)
      .join('\n');
    props.r.askAgent(
      `Draft my reply to review thread #${thread().id}${place ? ` on ${placeName(place)}` : ''}.\n\nThe thread so far:\n${said}\n\nAnswer with only the text of the reply.`,
      { thread: thread().id, question: 'Draft a reply' },
    );
  };

  return (
    <div class="thread-box">
      <div class="thread-said">
        <span>
          <span class="thread-state" data-open={thread().unresolved}>
            #{thread().id} {thread().unresolved ? 'open' : 'resolved'}
          </span>
          <Show when={props.showPlace && thread().place}>
            {(place) => (
              <button
                type="button"
                class="thread-place"
                onClick={() =>
                  props.r.showFile(place().file, place().old ? undefined : (place().line ?? undefined))
                }
              >
                {' '}
                · {placeName(place())}
                {place().outdated ? ' · its lines are gone' : ''}
              </button>
            )}
          </Show>
        </span>
        <For each={thread().comments}>
          {(c) => (
            <div class="said">
              <span class="said-who">
                {c.author ?? 'Someone'} · {when(c.publishedAt)}
              </span>
              <Markdown text={c.text} />
            </div>
          )}
        </For>
      </div>

      <Show when={props.draft}>
        {(draft) => (
          <NoteBox
            note={draft()}
            pull={props.r.pull}
            label="Draft from the agent · only you see it"
            post="Post as my reply"
            onPost={(words) => reply(props.r.dir, props.r.pr, thread().id, words)}
          />
        )}
      </Show>

      <Show when={replying.error() ?? setting.error()}>
        {(error) => <p class="thread-foot note-failed">{error()}</p>}
      </Show>
      <form
        class="thread-foot"
        onSubmit={(e) => {
          e.preventDefault();
          void replying.run();
        }}
      >
        <input
          class="thread-input"
          placeholder="Reply"
          aria-label={`Reply to thread ${thread().id}`}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
        />
        <Show when={text().trim()}>
          <button type="submit" class="quiet" disabled={replying.busy()}>
            Reply
          </button>
        </Show>
        <button type="button" class="quiet" disabled={Boolean(props.draft)} onClick={askForReply}>
          Ask the agent
        </button>
        <button
          type="button"
          class="quiet"
          disabled={setting.busy()}
          onClick={() => void setting.run(thread().unresolved ? 'resolved' : 'active')}
        >
          {thread().unresolved ? 'Resolve' : 'Reopen'}
        </button>
      </form>
    </div>
  );
}

/** What the agent answered, with the way to post it, change it first, or drop it. */
export function NoteBox(props: {
  note: Note;
  pull: string;
  label: string;
  /** What posting it is called. Nothing when there is nowhere to post it. */
  post?: string;
  onPost?: (words: string) => Promise<unknown>;
}) {
  const [editing, setEditing] = createSignal<string>();
  const posting = createAction(async () => {
    const words = (editing() ?? props.note.answer ?? '').trim();
    if (!words || !props.onPost) return;
    await props.onPost(words);
    dropNote(props.pull, props.note.id);
  });
  return (
    <div class="note-box">
      <span class="note-label">{props.label}</span>
      <Show when={props.note.question}>
        <span class="dim">{props.note.question}</span>
      </Show>
      <Show
        when={props.note.answer}
        fallback={
          <Show
            when={props.note.failed}
            fallback={<span class="note-working">The agent is working on it</span>}
          >
            {(why) => <span class="note-failed">{why()}</span>}
          </Show>
        }
      >
        {(answer) => (
          <Show when={editing() !== undefined} fallback={<Markdown text={answer()} />}>
            <textarea
              class="note-edit"
              rows={4}
              aria-label="What to post"
              value={editing()}
              onInput={(e) => setEditing(e.currentTarget.value)}
            />
          </Show>
        )}
      </Show>
      <Show when={posting.error()}>{(error) => <span class="note-failed">{error()}</span>}</Show>
      <Show when={props.note.answer || props.note.failed}>
        <div class="note-buttons">
          <Show when={props.note.answer && props.post}>
            <button
              type="button"
              class="button button-accent"
              disabled={posting.busy()}
              onClick={() => void posting.run()}
            >
              {props.post}
            </button>
            <Show when={editing() === undefined}>
              <button type="button" class="quiet" onClick={() => setEditing(props.note.answer)}>
                Change it first
              </button>
            </Show>
          </Show>
          <button type="button" class="quiet" onClick={() => dropNote(props.pull, props.note.id)}>
            Discard
          </button>
        </div>
      </Show>
    </div>
  );
}

/** A new thread on the whole pull request. */
export function NewComment(props: { r: Reviewing }) {
  const [text, setText] = createSignal('');
  const posting = createAction(async () => {
    const words = text().trim();
    if (!words) return;
    await comment(props.r.dir, props.r.pr, words, null);
    setText('');
  });
  return (
    <>
      <div class="thread-box">
        <form
          class="thread-foot thread-alone"
          onSubmit={(e) => {
            e.preventDefault();
            void posting.run();
          }}
        >
          <input
            class="thread-input"
            placeholder="Write a comment on the pull request"
            aria-label="Write a comment on the pull request"
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
          />
          <Show when={text().trim()}>
            <button type="submit" class="quiet" disabled={posting.busy()}>
              Comment
            </button>
          </Show>
        </form>
      </div>
      <Show when={posting.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
    </>
  );
}
