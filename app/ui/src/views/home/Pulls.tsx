// Home's two lists of pull requests. "Pull requests" has the ones you opened,
// from dova, in the whole organization of each folder you have open. A row
// that needs you says why under it: the check that failed, the thread still
// open. When dova is missing or signed out, the section says what to run.
// When there is nothing to list, it says what was asked and what was not.
//
// "To review" has the ones you or a team of yours were asked to review.
// There can be a great many, so each is one line, and under the heading
// you add the repos, projects and people whose rows show.

import { createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { pullHue } from '../../components/hues';
import { openFolderDialog } from '../../components/OpenFolder';
import { openUrl } from '../../components/open';
import { DaemonError } from '../../daemon';
import type { PullRow } from '../../protocol/PullRow';
import { health, recheck } from '../../state/health';
import {
  asked,
  byPerson,
  inRepo,
  yours as listed,
  picked,
  pulls,
  refreshPulls,
  repoKey,
  searched,
  setPicked,
  shows,
  toReview,
} from '../../state/pulls';
import { open } from '../../state/tabs';

const INSTALL = 'curl -LO https://dova.maxjay.dev/dova.deb && sudo apt install ./dova.deb';

/** A command to run yourself in WSL, with a way to copy it and to look again after. */
function Run(props: { command: string; onDone: () => void }) {
  const [copied, setCopied] = createSignal(false);
  return (
    <div class="pulls-run">
      <code class="pulls-command">{props.command}</code>
      <button
        type="button"
        class="button"
        onClick={() => void navigator.clipboard.writeText(props.command).then(() => setCopied(true))}
      >
        {copied() ? 'Copied' : 'Copy'}
      </button>
      <button type="button" class="button" onClick={props.onDone}>
        Check again
      </button>
    </div>
  );
}

function Row(props: { row: PullRow }) {
  const row = () => props.row;
  const state = () => (row().isDraft ? 'draft' : row().status);
  return (
    <div class="pull">
      <button
        type="button"
        class="row pull-row"
        disabled={!row().workspace}
        onClick={() => {
          const workspace = row().workspace;
          if (!workspace) return;
          open(`pr:${workspace}:${row().id}`);
        }}
      >
        <span style={{ color: pullHue(row().id) }}>#{row().id}</span>
        <span class="pull-title">{row().title}</span>
        <span>{row().repo}</span>
        <span class="dim">
          {row().sourceBranch} -&gt; {row().targetBranch}
        </span>
        <span classList={{ dim: row().isDraft }}>{state()}</span>
        <span class="pull-check" data-result={row().check?.result ?? 'none'}>
          {row().check ? (row().check?.result ?? 'running') : 'none'}
        </span>
        <span class="pull-threads" data-open={row().openThreads > 0}>
          {row().openThreads > 0 ? `${row().openThreads} open` : '0'}
        </span>
      </button>
      <Show when={row().failed}>
        {(why) => (
          <p class="pull-why">
            <span class="note-failed">{row().check?.name} failed.</span> {why()}
          </p>
        )}
      </Show>
      <Show when={row().thread}>
        {(thread) => (
          <p class="pull-why">
            <span class="pull-thread">#{thread().id}</span>{' '}
            <Show when={thread().location}>
              {(at) => (
                <>
                  {at().file}
                  {at().line ? `:${at().line}` : ''} ·{' '}
                </>
              )}
            </Show>
            {thread().lastAuthor}: {thread().lastComment}
          </p>
        )}
      </Show>
      <Show when={!row().workspace}>
        <p class="pull-why">
          The {row().repo} repo is not open here.{' '}
          <button type="button" class="pull-open" onClick={openFolderDialog}>
            Open its folder
          </button>
        </p>
      </Show>
    </div>
  );
}

/** One to review: a line. It opens here when its repo is open here, and on Azure DevOps when not. */
function ReviewRow(props: { row: PullRow }) {
  const row = () => props.row;
  return (
    <button
      type="button"
      class="row pull-row pull-review"
      title={row().workspace ? undefined : `The ${row().repo} repo is not open here. Opens on Azure DevOps.`}
      onClick={() => {
        const workspace = row().workspace;
        if (workspace) open(`pr:${workspace}:${row().id}`);
        else openUrl(row().url);
      }}
    >
      <span style={{ color: pullHue(row().id) }}>#{row().id}</span>
      <span class="pull-title">{row().title}</span>
      <span title={repoKey(row())}>{row().repo}</span>
      <span class="dim">{row().author}</span>
      <span classList={{ dim: row().isDraft }}>{row().isDraft ? 'draft' : row().status}</span>
      <span classList={{ dim: row().review?.vote === 'none' }}>{row().review?.vote}</span>
      <span class="dim">{row().review?.team}</span>
    </button>
  );
}

/** Every name in a list with how many rows have it, the most first. */
function counted(names: string[]) {
  const counts = new Map<string, number>();
  for (const name of names) counts.set(name, (counts.get(name) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/**
 * One list of what you picked, and a field to add to it. Typing shows the few names among the
 * rows that have what you typed. Enter takes the first, or what you typed when none has it: a
 * person with nothing open now can be added that way.
 */
function Adder(props: {
  word: string;
  hint: string;
  picked: string[];
  /** Every name there is to add, with how many rows have it. */
  names: [string, number][];
  /** How a name is shown. */
  shown: (name: string) => string;
  /** How many rows a picked name has now. */
  count: (name: string) => number;
  onChange: (picked: string[]) => void;
}) {
  const [text, setText] = createSignal('');
  const found = createMemo(() => {
    const typed = text().trim().toLowerCase();
    if (!typed) return [];
    const has = (name: string) => props.picked.some((other) => other.toLowerCase() === name.toLowerCase());
    return props.names.filter(([name]) => name.toLowerCase().includes(typed) && !has(name)).slice(0, 8);
  });
  const add = (name: string) => {
    if (name && !props.picked.includes(name)) props.onChange([...props.picked, name]);
    setText('');
  };

  return (
    <>
      <span class="watch-word dim">{props.word}</span>
      <div class="watch-line">
        <For each={props.picked}>
          {(name) => (
            <button
              type="button"
              class="watch-picked"
              title="Take it out"
              onClick={() => props.onChange(props.picked.filter((other) => other !== name))}
            >
              {props.shown(name)} <span class="watch-count">{props.count(name)}</span>{' '}
              <span aria-hidden="true">×</span>
            </button>
          )}
        </For>
        <input
          class="field watch-field"
          placeholder={props.hint}
          aria-label={props.hint}
          autocomplete="off"
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') add(found()[0]?.[0] ?? text().trim());
            if (e.key === 'Escape') setText('');
          }}
        />
        <For each={found()}>
          {([name, count]) => (
            <button type="button" class="watch-found" onClick={() => add(name)}>
              {props.shown(name)} <span class="watch-count">{count}</span>
            </button>
          )}
        </For>
      </div>
    </>
  );
}

/** What to see of the rows to review: the repos and the people you added. */
function Pick(props: { rows: PullRow[] }) {
  /** Every repo as `project/repo`, and every project as `project/`, the most rows first. */
  const repos = createMemo(() => [
    ...counted(props.rows.map(repoKey)),
    ...counted(props.rows.map((row) => `${row.project}/`)),
  ]);
  const people = createMemo(() => counted(props.rows.map((row) => row.author ?? '').filter(Boolean)));
  return (
    <div class="watch">
      <Adder
        word="Repos"
        hint="add a repo or project"
        picked={picked().repos}
        names={repos()}
        shown={(name) => (name.endsWith('/') ? `all of ${name.slice(0, -1)}` : name)}
        count={(name) => props.rows.filter((row) => inRepo(name, row)).length}
        onChange={(repos) => setPicked({ ...picked(), repos })}
      />
      <Adder
        word="People"
        hint="add a person"
        picked={picked().people}
        names={people()}
        shown={(name) => name}
        count={(name) => props.rows.filter((row) => byPerson(name, row)).length}
        onChange={(people) => setPicked({ ...picked(), people })}
      />
    </div>
  );
}

function ToReview() {
  /** The ones to review that you did not open yourself. */
  const all = createMemo(() => {
    const mine = listed() ?? [];
    const same = (a: PullRow) => (b: PullRow) => a.id === b.id && repoKey(a) === repoKey(b);
    return (toReview.latest ?? []).filter((row) => !mine.some(same(row)));
  });
  const shown = createMemo(() => all().filter((row) => shows(picked(), row)));
  const any = () => picked().repos.length + picked().people.length > 0;
  const failure = () => {
    const error: unknown = toReview.error;
    return error instanceof Error ? error.message : String(error);
  };

  return (
    <section class="section" aria-labelledby="to-review">
      <div class="section-head">
        <h2 id="to-review">To review</h2>
        <Show when={toReview.latest}>
          <span class="dim">{any() ? `${shown().length} of ${all().length}` : all().length}</span>
        </Show>
        <span class="top-gap" />
        <Show when={any()}>
          <button type="button" class="quiet" onClick={() => setPicked({ repos: [], people: [] })}>
            Clear
          </button>
        </Show>
      </div>
      <Show when={all().length > 0}>
        <Pick rows={all()} />
      </Show>
      <Switch>
        <Match when={toReview.latest}>
          <div class="rows rows-review">
            <Show
              when={all().length > 0}
              fallback={
                <p class="rows-none dim">None open that you or a team of yours were asked to review.</p>
              }
            >
              <div class="pull-row pull-head dim">
                <span>#</span>
                <span>Title</span>
                <span>Repo</span>
                <span>By</span>
                <span>State</span>
                <span>Your vote</span>
                <span>Asked as</span>
              </div>
              <For each={shown()}>{(row) => <ReviewRow row={row} />}</For>
              <Show when={shown().length === 0}>
                <p class="rows-none dim">None in what you picked.</p>
              </Show>
            </Show>
          </div>
        </Match>
        {/* The section above already says when dova is missing or signed out. */}
        <Match when={toReview.error && !pulls.error}>
          <p class="rows-none note-failed">{failure()}</p>
        </Match>
        <Match when={toReview.loading}>
          <p class="rows-none dim">Asking Azure DevOps</p>
        </Match>
      </Switch>
    </section>
  );
}

function Yours() {
  const signedOut = () => pulls.error instanceof DaemonError && pulls.error.kind === 'notSignedIn';
  /** Why dova couldn't be asked, in its own words. */
  const failure = () => {
    const error: unknown = pulls.error;
    return error instanceof Error ? error.message : String(error);
  };
  const rows = () => listed() ?? [];
  const needYou = () => rows().filter((row) => row.failed || row.thread).length;
  /** The folders that were passed over: their remote is not on Azure Repos, or they have none. */
  const passed = () => asked().filter((folder) => !folder.project);

  return (
    <section class="section" aria-labelledby="pulls">
      <div class="section-head">
        <h2 id="pulls">Pull requests</h2>
        <Show when={rows().length}>
          <span class="dim">
            {rows().length} yours{needYou() > 0 ? `, ${needYou()} need you` : ''}
          </span>
        </Show>
      </div>
      <Switch>
        <Match when={health() && !health()?.dova}>
          <div class="pulls-state">
            <p>dova is not installed in {health()?.distro ?? 'WSL'}. Pull requests come from it.</p>
            <Run command={INSTALL} onDone={() => void recheck()} />
          </div>
        </Match>
        <Match when={signedOut()}>
          <div class="pulls-state">
            <p>
              <span class="note-failed">Not signed in.</span> dova has no sign-in to Azure DevOps.
            </p>
            <Run command="az login" onDone={() => void refreshPulls()} />
          </div>
        </Match>
        <Match when={pulls.error}>
          <div class="pulls-state">
            <p class="note-failed">{failure()}</p>
            <button type="button" class="button" onClick={() => void refreshPulls()}>
              Try again
            </button>
          </div>
        </Match>
        <Match when={listed()}>
          <div class="rows">
            <div class="pull-row pull-head dim">
              <span>#</span>
              <span>Title</span>
              <span>Repo</span>
              <span>Branch</span>
              <span>State</span>
              <span>Check</span>
              <span>Threads</span>
            </div>
            <For each={rows()}>{(row) => <Row row={row} />}</For>
            {/* Nothing to list: say what was asked, so it is plain whether that is right. */}
            <Show when={rows().length === 0}>
              <div class="rows-none dim pulls-why">
                <Show
                  when={asked().length > 0}
                  fallback={
                    <p>
                      No folder is open yet. Pull requests are asked for from a folder cloned from Azure
                      Repos.{' '}
                      <button type="button" class="pull-open" onClick={openFolderDialog}>
                        Open a folder
                      </button>
                    </p>
                  }
                >
                  <Show
                    when={searched().length > 0}
                    fallback={
                      <p>None of your folders is cloned from Azure Repos, so there was nothing to ask.</p>
                    }
                  >
                    <p>None open that you opened. Asked: {searched().join(', ')}.</p>
                  </Show>
                </Show>
              </div>
            </Show>
            {/* A folder that was passed over is said even when others gave rows: it may be the one you expected. */}
            <Show when={passed().length > 0}>
              <div class="rows-none dim pulls-why">
                <For each={passed()}>
                  {(folder) => (
                    <p>
                      {folder.name} was not asked:{' '}
                      {folder.remote
                        ? `its remote, ${folder.remote}, is not on dev.azure.com or visualstudio.com.`
                        : 'it has no remote called origin.'}
                    </p>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Match>
        <Match when={pulls.loading}>
          <p class="rows-none dim">Asking dova</p>
        </Match>
      </Switch>
    </section>
  );
}

export function Pulls() {
  // Asked again each minute while Home is on screen.
  onMount(() => {
    const timer = setInterval(() => document.hidden || void refreshPulls(), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  return (
    <>
      <Yours />
      <ToReview />
    </>
  );
}
