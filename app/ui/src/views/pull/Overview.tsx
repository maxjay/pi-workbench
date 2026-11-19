// A pull request's overview, in the middle: its title and what it says it
// does, which you can change when it is yours; its tickets; what must be
// true before it completes; and the threads about the whole of it.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Markdown } from '../../components/Markdown';
import { openUrl } from '../../components/open';
import { comment, describedAs, editPull, keepPicture, talksOf, threadTalk } from '../../state/review';
import { type Pull, ticketHue, voteWord } from './parts';
import { Said } from './Said';

/** A policy's state, as the colour of its square and a word. */
function policyState(status: string): { tone: string; word: string } {
  if (status === 'approved') return { tone: 'ok', word: '' };
  if (status === 'rejected') return { tone: 'failed', word: 'not met' };
  if (status === 'broken') return { tone: 'failed', word: 'broken' };
  if (status === 'running') return { tone: 'working', word: 'running' };
  if (status === 'queued') return { tone: 'working', word: 'waiting' };
  return { tone: 'faint', word: 'does not apply' };
}

export function Overview(props: { p: Pull; onChecks: () => void }) {
  const p = () => props.p;
  const review = () => p().review;
  const general = () => review().threads.filter((thread) => !thread.place);
  /** What only you and the agent said about lines the pull request no longer changes. */
  const adrift = () =>
    talksOf(p().key).filter(
      (talk) =>
        talk.thread === undefined && talk.place && !review().files.some((f) => f.path === talk.place?.file),
    );
  const applying = () => review().policies.filter((policy) => policy.status !== 'notApplicable');
  const met = () => applying().filter((policy) => policy.status === 'approved').length;

  // Changing the title and the description.
  const [draft, setDraft] = createSignal<{ title: string; description: string }>();
  let body: HTMLTextAreaElement | undefined;
  let chooser: HTMLInputElement | undefined;
  const starting = createAction(async () => {
    // The description as it was written: what is shown has its pictures put in place of their links.
    setDraft({ title: review().pr.title, description: await describedAs(p().dir, p().pr) });
  });
  const saving = createAction(async () => {
    const now = draft();
    if (!now?.title.trim()) return;
    await editPull(p().dir, p().pr, now.title.trim(), now.description);
    await p().reload();
    setDraft();
  });
  /** Keeps a picture where dova can read it, and writes it into the description where the cursor is. */
  const adding = createAction(async (picture: File) => {
    const now = draft();
    if (!now) return;
    const bytes = new Uint8Array(await picture.arrayBuffer());
    let binary = '';
    for (let at = 0; at < bytes.length; at += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    }
    const path = await keepPicture(picture.name || 'picture.png', btoa(binary));
    const name = (picture.name || 'picture').replace(/\.[^.]*$/, '');
    const at = body?.selectionStart ?? now.description.length;
    const text = now.description;
    const lead = at > 0 && text[at - 1] !== '\n' ? '\n\n' : '';
    setDraft({ ...now, description: `${text.slice(0, at)}${lead}![${name}](${path})\n${text.slice(at)}` });
  });

  // A comment on the whole pull request.
  const [words, setWords] = createSignal('');
  const posting = createAction(async () => {
    const text = words().trim();
    if (!text) return;
    await comment(p().dir, p().pr, text, null);
    setWords('');
  });

  return (
    <div class="overview">
      <div class="overview-col">
        <Show
          when={draft()}
          fallback={
            <section class="overview-what">
              <div class="overview-title">
                <h2>{review().pr.title}</h2>
                <Show when={review().mine}>
                  <button
                    type="button"
                    class="quiet"
                    disabled={starting.busy()}
                    onClick={() => void starting.run()}
                  >
                    Edit
                  </button>
                </Show>
              </div>
              <Show when={starting.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
              <Show when={review().description.trim()} fallback={<span class="dim">No description.</span>}>
                <Markdown text={review().description} />
              </Show>
            </section>
          }
        >
          {(now) => (
            <section class="overview-what">
              <input
                class="overview-title-input"
                type="text"
                aria-label="Title"
                value={now().title}
                onInput={(e) => setDraft({ ...now(), title: e.currentTarget.value })}
              />
              <textarea
                class="overview-body-input"
                aria-label="Description, as Markdown"
                rows={12}
                ref={body}
                value={now().description}
                onInput={(e) => setDraft({ ...now(), description: e.currentTarget.value })}
                onPaste={(e) => {
                  const picture = [...(e.clipboardData?.files ?? [])].find((f) =>
                    f.type.startsWith('image/'),
                  );
                  if (!picture) return;
                  e.preventDefault();
                  void adding.run(picture);
                }}
              />
              <Show when={saving.error() ?? adding.error()}>
                {(error) => <p class="note-failed">{error()}</p>}
              </Show>
              <div class="overview-edit-foot">
                <input
                  type="file"
                  accept="image/*"
                  hidden
                  ref={chooser}
                  onChange={(e) => {
                    const picture = e.currentTarget.files?.[0];
                    e.currentTarget.value = '';
                    if (picture) void adding.run(picture);
                  }}
                />
                <button type="button" class="quiet" disabled={adding.busy()} onClick={() => chooser?.click()}>
                  Picture
                </button>
                <span class="dim overview-hint">or paste one into the text</span>
                <span class="top-gap" />
                <button type="button" class="quiet" onClick={() => setDraft()}>
                  Cancel
                </button>
                <button
                  type="button"
                  class="soft"
                  data-tone="accent"
                  disabled={saving.busy() || !now().title.trim()}
                  onClick={() => void saving.run()}
                >
                  Save
                </button>
              </div>
            </section>
          )}
        </Show>

        <Show when={review().workItems.length > 0}>
          <section class="overview-part" aria-label="Tickets">
            <span class="overview-label">Tickets</span>
            <For each={review().workItems}>
              {(item) => (
                <button type="button" class="row overview-row" onClick={() => p().openTicket(item.id)}>
                  <span style={{ color: ticketHue(item.type) }}>#{item.id}</span>
                  <span class="overview-row-name">{item.title}</span>
                  <span class="dim">{[item.type, item.state].filter(Boolean).join(' · ')}</span>
                </button>
              )}
            </For>
          </section>
        </Show>

        <Show when={review().pr.status === 'active' && applying().length > 0}>
          <section class="overview-part" aria-label="To complete">
            <span class="overview-label">
              To complete ·{' '}
              <span data-state={review().ready ? 'ok' : undefined}>
                {review().ready ? 'ready' : `${met()} of ${applying().length}`}
              </span>
            </span>
            <For each={applying()}>
              {(policy) => (
                <button
                  type="button"
                  class="row overview-row"
                  onClick={() =>
                    /build/i.test(policy.name) || !policy.blocking ? props.onChecks() : undefined
                  }
                >
                  <span class="sq" data-tone={policyState(policy.status).tone} />
                  <span class="overview-row-name">{policy.name}</span>
                  <span class="dim">
                    {[policyState(policy.status).word, policy.blocking ? '' : 'optional']
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                </button>
              )}
            </For>
          </section>
        </Show>

        <Show when={review().reviewers.length > 0}>
          <section class="overview-part" aria-label="Reviewers">
            <span class="overview-label">Reviewers</span>
            <For each={review().reviewers}>
              {(reviewer) => (
                <div class="overview-row">
                  <span class="sq" data-tone={reviewer.vote === 0 ? 'faint' : voteWord(reviewer.vote).tone} />
                  <span class="overview-row-name">{reviewer.name}</span>
                  <span class="dim">
                    {[
                      reviewer.vote === 0 ? 'no vote' : voteWord(reviewer.vote).word.toLowerCase(),
                      reviewer.required ? 'required' : '',
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                </div>
              )}
            </For>
          </section>
        </Show>

        <section class="overview-part overview-comments" aria-label="Comments">
          <span class="overview-label">Comments</span>
          <For each={general()}>
            {(thread) => (
              <Said
                p={p()}
                thread={thread}
                talk={threadTalk(p().key, thread.id)}
                place={null}
                open={p().opened(`t${thread.id}`)}
                onOpen={() => {
                  p().setOpened(`t${thread.id}`, true);
                  p().setAbout({ kind: 'thread', thread: thread.id });
                }}
                onClose={() => p().setOpened(`t${thread.id}`, false)}
              />
            )}
          </For>
          <For each={adrift()}>
            {(talk) => (
              <Said
                p={p()}
                talk={talk}
                place={talk.place ?? null}
                showPlace
                gone
                open={p().opened(`k${talk.id}`)}
                onOpen={() => p().setOpened(`k${talk.id}`, true)}
                onClose={() => p().setOpened(`k${talk.id}`, false)}
              />
            )}
          </For>
          <Show when={posting.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
          <input
            class="overview-comment"
            type="text"
            aria-label="Comment on the pull request"
            placeholder="Comment"
            value={words()}
            disabled={posting.busy()}
            onInput={(e) => setWords(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key !== 'Enter') return;
              e.preventDefault();
              void posting.run();
            }}
          />
        </section>

        <p class="overview-foot">
          <button type="button" class="link" onClick={() => openUrl(review().pr.url)}>
            Azure DevOps
          </button>
        </p>
      </div>
    </div>
  );
}
