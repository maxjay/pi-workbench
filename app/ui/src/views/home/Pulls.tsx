// Home's two lists of pull requests. "Pull requests" has the ones you opened,
// from dova, in the whole organization of each folder you have open. A row
// that needs you says why under it: the check that failed, the thread still
// open. When dova is missing or signed out, the section says what to run.
// When there is nothing to list, it says what was asked and what was not.
//
// "To review" has the ones you or a team of yours were asked to review.
// There can be a great many, so each is one line, and Pick chooses the
// projects, repos and people whose rows show.

import { createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { pullHue } from '../../components/hues';
import { openFolderDialog } from '../../components/OpenFolder';
import { openUrl } from '../../components/open';
import { DaemonError } from '../../daemon';
import type { PullRow } from '../../protocol/PullRow';
import { health, recheck } from '../../state/health';
import {
  asked,
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

/** The projects, repos and people among the rows to review, each a switch. */
function Pick(props: { rows: PullRow[] }) {
  const projects = createMemo(() => {
    const names = [...new Set(props.rows.map((row) => row.project))].sort((a, b) => a.localeCompare(b));
    return names.map((project) => ({
      project,
      repos: counted(props.rows.filter((row) => row.project === project).map((row) => row.repo)),
    }));
  });
  const people = createMemo(() => counted(props.rows.map((row) => row.author ?? '').filter(Boolean)));
  const whole = (project: string) => picked().repos.includes(`${project}/*`);
  const on = (project: string, repo: string) =>
    whole(project) || picked().repos.includes(`${project}/${repo}`);

  const toggleProject = (project: string) => {
    const others = picked().repos.filter((key) => !key.startsWith(`${project}/`));
    setPicked({ ...picked(), repos: whole(project) ? others : [...others, `${project}/*`] });
  };
  const toggleRepo = (project: string, repo: string, all: string[]) => {
    const others = picked().repos.filter((key) => !key.startsWith(`${project}/`));
    // The repos of this project that are on now, by name.
    const now = all.filter((name) => on(project, name));
    const next = now.includes(repo) ? now.filter((name) => name !== repo) : [...now, repo];
    setPicked({ ...picked(), repos: [...others, ...next.map((name) => `${project}/${name}`)] });
  };
  const togglePerson = (name: string) => {
    const now = picked().people;
    setPicked({
      ...picked(),
      people: now.includes(name) ? now.filter((other) => other !== name) : [...now, name],
    });
  };

  return (
    <div class="watch">
      <For each={projects()}>
        {(group) => (
          <div class="watch-line">
            <button
              type="button"
              class="quiet watch-project"
              aria-pressed={whole(group.project)}
              title="The whole project"
              onClick={() => toggleProject(group.project)}
            >
              {group.project}
            </button>
            <For each={group.repos}>
              {([repo, count]) => (
                <button
                  type="button"
                  class="quiet"
                  aria-pressed={on(group.project, repo)}
                  onClick={() =>
                    toggleRepo(
                      group.project,
                      repo,
                      group.repos.map(([name]) => name),
                    )
                  }
                >
                  {repo} <span class="watch-count">{count}</span>
                </button>
              )}
            </For>
          </div>
        )}
      </For>
      <div class="watch-line">
        <span class="watch-word dim">by</span>
        <For each={people()}>
          {([name, count]) => (
            <button
              type="button"
              class="quiet"
              aria-pressed={picked().people.includes(name)}
              onClick={() => togglePerson(name)}
            >
              {name} <span class="watch-count">{count}</span>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

function ToReview() {
  const [picking, setPicking] = createSignal(false);
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
            All
          </button>
        </Show>
        <Show when={all().length > 0}>
          <button type="button" class="quiet" aria-pressed={picking()} onClick={() => setPicking(!picking())}>
            Pick
          </button>
        </Show>
      </div>
      <Show when={picking()}>
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
