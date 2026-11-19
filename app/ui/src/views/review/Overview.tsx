// A pull request's overview, in the middle: what it says it does, the work
// items it is for, who voted how, and the comments that are not on code.

import { For, Show } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import { openUrl } from '../../components/open';
import { notesOf } from '../../state/review';
import { type Reviewing, voteWords } from './parts';
import { NewComment, ThreadBox } from './Threads';

export function Overview(props: { r: Reviewing }) {
  const review = () => props.r.review;
  const general = () => review().threads.filter((thread) => !thread.place);
  const draftFor = (thread: number) => notesOf(props.r.pull).find((note) => note.thread === thread);
  /** A work item's page: beside the pull request's, in the same project. */
  const itemUrl = (id: number) => `${review().pr.url.split('/_git/')[0]}/_workitems/edit/${id}`;

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

      <section class="overview-part" aria-labelledby="overview-items">
        <h2 id="overview-items" class="pull-label">
          Work items
        </h2>
        <For each={review().workItems} fallback={<span class="dim">None linked.</span>}>
          {(item) => (
            <button
              type="button"
              class="row overview-line"
              title="Opens in your browser"
              onClick={() => openUrl(itemUrl(item.id))}
            >
              <span class="dim">#{item.id}</span>
              <span class="tree-name">{item.title}</span>
              <span class="dim">{item.state}</span>
            </button>
          )}
        </For>
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
        <h2 id="overview-comments" class="pull-label">
          Comments not on code
        </h2>
        <For each={general()}>
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
