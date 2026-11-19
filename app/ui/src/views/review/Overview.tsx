// A pull request's overview, in the middle: the agent's review, what the
// pull request says it does, who voted how, and the comments that are not
// on code.

import { createSignal, For, Show } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import { openUrl } from '../../components/open';
import { hideResolved, notesOf, setHideResolved } from '../../state/review';
import { FROM_REVIEW } from './agentReview';
import { placeName, type Reviewing, voteWords } from './parts';
import { NewComment, NoteBox, ThreadBox } from './Threads';

export function Overview(props: { r: Reviewing }) {
  const review = () => props.r.review;
  const general = () => review().threads.filter((thread) => !thread.place);
  const shownGeneral = () => general().filter((thread) => !(hideResolved() && !thread.unresolved));
  const resolved = () => review().threads.filter((thread) => !thread.unresolved).length;
  const draftFor = (thread: number) => notesOf(props.r.pull).find((note) => note.thread === thread);

  // The agent's review of the whole pull request: a summary here, and comments drafted on lines.
  const [focus, setFocus] = createSignal('');
  const summary = () => notesOf(props.r.pull).find((note) => note.summary);
  const drafts = () => notesOf(props.r.pull).filter((note) => note.question === FROM_REVIEW && note.place);
  return (
    <div class="overview">
      <section class="overview-part" aria-labelledby="overview-agent">
        <h2 id="overview-agent" class="pull-label">
          The agent's review
        </h2>
        <Show when={summary()}>
          {(note) => (
            <NoteBox note={note()} pull={props.r.pull} label="The agent's review · only you see it" />
          )}
        </Show>
        <Show when={drafts().length > 0}>
          <span class="dim">
            {drafts().length} {drafts().length === 1 ? 'comment' : 'comments'} drafted. Each waits under its
            line for you to post or discard.
          </span>
          <For each={drafts()}>
            {(note) => (
              <button
                type="button"
                class="row overview-line"
                onClick={() =>
                  note.place &&
                  props.r.showFile(
                    note.place.file,
                    note.place.old ? undefined : (note.place.line ?? undefined),
                  )
                }
              >
                <span class="accent">{note.place ? placeName(note.place) : ''}</span>
                <span class="tree-name">{note.answer}</span>
              </button>
            )}
          </For>
        </Show>
        <form
          class="thread-box"
          onSubmit={(e) => {
            e.preventDefault();
            props.r.reviewAll(focus().trim());
          }}
        >
          <div class="thread-foot thread-alone">
            <input
              class="thread-input"
              placeholder="What to look at above all. Leave empty for everything"
              aria-label="What the review should look at above all"
              value={focus()}
              onInput={(e) => setFocus(e.currentTarget.value)}
            />
            <button type="submit" class="button review-go" disabled={props.r.reviewingAll}>
              {props.r.reviewingAll
                ? 'Reviewing'
                : summary()
                  ? 'Review it again'
                  : 'Review it with the agent'}
            </button>
          </div>
        </form>
      </section>

      <section class="overview-part" aria-labelledby="overview-description">
        <h2 id="overview-description" class="pull-label">
          Description
        </h2>
        <Show when={review().description.trim()} fallback={<span class="dim">None.</span>}>
          <Markdown text={review().description} />
        </Show>
      </section>

      <section class="overview-part" aria-labelledby="overview-reviewers">
        <h2 id="overview-reviewers" class="pull-label">
          Reviewers
        </h2>
        <For each={review().reviewers} fallback={<span class="dim">None yet.</span>}>
          {(reviewer) => (
            <div class="row overview-line">
              <span class="tree-name">{reviewer.name}</span>
              <Show when={reviewer.required}>
                <span class="dim">required</span>
              </Show>
              <span class="mark" data-state={voteWords(reviewer.vote).state}>
                {voteWords(reviewer.vote).words}
              </span>
            </div>
          )}
        </For>
      </section>

      <section class="overview-part" aria-labelledby="overview-comments">
        <h2 id="overview-comments" class="pull-label overview-head">
          <span>Comments not on code</span>
          <Show when={resolved() > 0}>
            <button type="button" class="quiet" onClick={() => setHideResolved(!hideResolved())}>
              {hideResolved() ? `Show resolved · ${resolved()} in this pull request` : 'Hide resolved'}
            </button>
          </Show>
        </h2>
        <For each={shownGeneral()}>
          {(thread) => <ThreadBox r={props.r} thread={thread} draft={draftFor(thread.id)} />}
        </For>
        <NewComment r={props.r} />
      </section>

      <button type="button" class="pull-open overview-out" onClick={() => openUrl(review().pr.url)}>
        Open it on Azure DevOps
      </button>
    </div>
  );
}
