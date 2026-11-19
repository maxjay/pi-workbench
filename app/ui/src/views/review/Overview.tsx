// A pull request's overview, in the middle: what it says it does, what the
// agent made of it if you asked, and the comments that are not on code.
// Its tickets and its checks are on the left, and the votes are in the bar.

import { For, Show } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import { openUrl } from '../../components/open';
import { hideResolved, notesOf, setHideResolved } from '../../state/review';
import { type Reviewing, voteWords } from './parts';
import { NewComment, NoteBox, ThreadBox } from './Threads';

export function Overview(props: { r: Reviewing }) {
  const review = () => props.r.review;
  const general = () => review().threads.filter((thread) => !thread.place);
  const shownGeneral = () => general().filter((thread) => !(hideResolved() && !thread.unresolved));
  const resolved = () => general().filter((thread) => !thread.unresolved).length;
  const draftFor = (thread: number) => notesOf(props.r.pull).find((note) => note.thread === thread);
  const summary = () => notesOf(props.r.pull).find((note) => note.summary);

  return (
    <div class="overview">
      <section class="overview-part" aria-labelledby="overview-description">
        <h2 id="overview-description" class="pull-label">
          Description
        </h2>
        <Show when={review().description.trim()} fallback={<span class="dim">None.</span>}>
          <Markdown text={review().description} />
        </Show>
      </section>

      <Show when={summary()}>
        {(note) => (
          <section class="overview-part" aria-label="The agent's review">
            <NoteBox note={note()} pull={props.r.pull} label="agent review" />
          </section>
        )}
      </Show>

      <section class="overview-part" aria-labelledby="overview-comments">
        <h2 id="overview-comments" class="pull-label overview-head">
          <span>Comments</span>
          <Show when={resolved() > 0}>
            <button
              type="button"
              class="quiet"
              title="Show the threads that are resolved"
              aria-pressed={!hideResolved()}
              onClick={() => setHideResolved(!hideResolved())}
            >
              {resolved()} resolved
            </button>
          </Show>
        </h2>
        <For each={shownGeneral()}>
          {(thread) => <ThreadBox r={props.r} thread={thread} draft={draftFor(thread.id)} />}
        </For>
        <NewComment r={props.r} />
      </section>

      <p class="overview-foot dim">
        <Show when={review().reviewers.length > 0}>
          Reviewers:{' '}
          {review()
            .reviewers.map((r) => `${r.name} ${voteWords(r.vote).words}${r.required ? ', required' : ''}`)
            .join(' · ')}
          {' · '}
        </Show>
        <button type="button" class="pull-open" onClick={() => openUrl(review().pr.url)}>
          Azure DevOps
        </button>
      </p>
    </div>
  );
}
