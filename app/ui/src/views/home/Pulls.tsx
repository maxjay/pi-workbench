// Home's two lists of pull requests. "Pull requests" has the ones you opened,
// in the whole organization of each folder you have open. "To review" has
// the open pull requests of other people that you chose to see: by these
// people, in these repos. Nothing of those is asked for until you choose,
// because an organization can have thousands.
//
// Both are Home's one table, as the tickets and the codespaces are. The
// rows are under their repo, and a repo can be shut. A row is its number,
// its title, and two facts. Of yours: its check and its open threads. Of
// the ones to review: who opened it and your vote. What a check said and
// what a thread says are in the pull request's own tab, not here.
//
// When dova is missing or signed out, the section says what to run. When
// there is nothing to list, it says what was asked and what was not.

import { createMemo, createSignal, For, type JSX, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { createStore } from 'solid-js/store';
import { pullHue } from '../../components/hues';
import { openFolderDialog } from '../../components/OpenFolder';
import { openUrl } from '../../components/open';
import { DaemonError } from '../../daemon';
import type { PullRow } from '../../protocol/PullRow';
import { health, recheck } from '../../state/health';
import {
  asked,
  filter,
  filtering,
  yours as listed,
  pulls,
  refreshOld,
  refreshPulls,
  refreshReview,
  searched,
  toReview,
} from '../../state/pulls';
import { open } from '../../state/tabs';
import { Filter, openFilter } from './Filter';

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

/** The repos you shut, in either list. Kept in the window's own storage. */
const [shut, setShut] = createStore<Record<string, boolean>>(
  (() => {
    try {
      return JSON.parse(localStorage.getItem('pulls-shut') ?? '{}') ?? {};
    } catch {
      return {};
    }
  })(),
);

function toggle(key: string) {
  setShut(key, !shut[key]);
  localStorage.setItem('pulls-shut', JSON.stringify(shut));
}

interface Group {
  key: string;
  project: string;
  repo: string;
  rows: PullRow[];
}

/** Rows under their repo, the repos by project and then by name. The rows of one repo stay in the order they came. */
function byRepo(rows: PullRow[]): Group[] {
  const groups = new Map<string, Group>();
  for (const row of rows) {
    const key = `${row.project}/${row.repo}`.toLowerCase();
    const group = groups.get(key) ?? { key, project: row.project, repo: row.repo, rows: [] };
    group.rows.push(row);
    groups.set(key, group);
  }
  return [...groups.values()].sort(
    (a, b) => a.project.localeCompare(b.project) || a.repo.localeCompare(b.repo),
  );
}

/**
 * Pull requests under their repos, as rows of Home's table. `facts` gives a row its two facts.
 * A repo's project is said only when the rows are in more than one.
 */
function Table(props: { list: 'yours' | 'review'; rows: PullRow[]; facts: (row: PullRow) => JSX.Element }) {
  const groups = createMemo(() => byRepo(props.rows));
  const projects = () => new Set(props.rows.map((row) => row.project.toLowerCase())).size;
  /** A row opens here when its repo is open here or cloned in your repos folder, and on Azure DevOps when not. */
  const go = (row: PullRow) => (row.codespace ? open(`pr:${row.codespace}:${row.id}`) : openUrl(row.url));
  return (
    <For each={groups()}>
      {(group) => {
        const key = `${props.list}:${group.key}`;
        return (
          <>
            <button
              type="button"
              class="row table-group"
              aria-expanded={!shut[key]}
              onClick={() => toggle(key)}
            >
              <Show when={projects() > 1}>
                <span class="dim">{group.project} /</span>
              </Show>
              <span>{group.repo}</span>
              <span class="dim">{group.rows.length}</span>
            </button>
            <Show when={!shut[key]}>
              <For each={group.rows}>
                {(row) => (
                  <button type="button" class="row table-row" onClick={() => go(row)}>
                    <span style={{ color: pullHue(row.id) }}>#{row.id}</span>
                    <span class="table-name">
                      {row.title}
                      <Show when={row.isDraft}>
                        <span class="table-aside">draft</span>
                      </Show>
                    </span>
                    {props.facts(row)}
                  </button>
                )}
              </For>
            </Show>
          </>
        );
      }}
    </For>
  );
}

/** One of yours: its latest check, and how many threads are open. */
function yoursFacts(row: PullRow) {
  const check = () => (row.check ? (row.check.result ?? 'running') : '');
  const threads = () => (row.openThreads === 1 ? '1 thread' : `${row.openThreads} threads`);
  return (
    <>
      <span data-state={check()}>{check() === 'succeeded' ? 'passed' : check()}</span>
      <span>{row.openThreads > 0 ? threads() : ''}</span>
    </>
  );
}

/** One to review: who opened it, and your vote. The team is said when you were asked through one. */
function reviewFacts(row: PullRow) {
  const vote = () => (row.review ? (row.review.vote === 'none' ? 'no vote' : row.review.vote) : 'not asked');
  const quiet = () => !row.review || row.review.vote === 'none';
  return (
    <>
      <span class="dim">{row.author}</span>
      <span
        classList={{ dim: quiet() }}
        title={row.review?.team ? `Asked through ${row.review.team}` : undefined}
      >
        {vote()}
      </span>
    </>
  );
}

function ToReview() {
  const any = () => filter().repos.length + filter().people.length > 0;
  const rows = () => (any() ? (toReview.latest ?? []) : []);
  /** What is chosen, in a few words: "3 people, 2 repos". */
  const chosen = () => {
    const count = (n: number, one: string, many: string) => (n === 0 ? [] : [`${n} ${n === 1 ? one : many}`]);
    const projects = filter().repos.filter((pick) => pick.repo === null).length;
    return [
      ...count(filter().people.length, 'person', 'people'),
      ...count(filter().repos.length - projects, 'repo', 'repos'),
      ...count(projects, 'project', 'projects'),
    ].join(', ');
  };
  /** Where was looked, for when nothing was found. */
  const where = () => {
    if (filter().repos.length === 0) return 'by the people';
    if (filter().people.length === 0) return 'in the repos';
    return filter().either ? 'by the people or in the repos' : 'by the people and in the repos';
  };
  const failure = () => {
    const error: unknown = toReview.error;
    return error instanceof Error ? error.message : String(error);
  };

  return (
    <section class="section" aria-labelledby="to-review">
      <div class="section-head">
        <h2 id="to-review">To review</h2>
        <Show when={any()}>
          <span class="dim">
            {toReview.latest ? `${rows().length} · ` : ''}
            {chosen()}
          </span>
        </Show>
        <span class="top-gap" />
        <button type="button" class="button" onClick={openFilter}>
          {any() ? 'Change' : 'Choose'}
        </button>
      </div>
      <div class="rows">
        <Switch>
          {/* Nothing is asked for until you choose: an organization can have thousands open. */}
          <Match when={!any()}>
            <p class="rows-none dim">
              Nothing is loaded here yet. Choose the people, teams, repos or projects whose pull requests you
              want to see.
            </p>
          </Match>
          <Match when={toReview.error && !pulls.error}>
            <p class="rows-none note-failed">{failure()}</p>
          </Match>
          <Match when={toReview.latest}>
            <Show
              when={rows().length > 0}
              fallback={<p class="rows-none dim">None open {where()} you chose, apart from your own.</p>}
            >
              <Table list="review" rows={rows()} facts={reviewFacts} />
            </Show>
          </Match>
          <Match when={toReview.loading || filtering()}>
            <p class="rows-none dim">Asking Azure DevOps</p>
          </Match>
        </Switch>
      </div>
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
  /** The folders that were passed over: their remote is not on Azure Repos, or they have none. */
  const passed = () => asked().filter((folder) => !folder.project);

  return (
    <section class="section" aria-labelledby="pulls">
      <div class="section-head">
        <h2 id="pulls">Pull requests</h2>
        <Show when={rows().length}>
          <span class="dim">{rows().length}</span>
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
            <Table list="yours" rows={rows()} facts={yoursFacts} />
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
  // Asked again when Home comes back on screen and when the window is gone back to, and while
  // Home is on screen: yours each minute, the ones you chose to see every five.
  onMount(() => {
    refreshOld();
    window.addEventListener('focus', refreshOld);
    const timer = setInterval(() => document.hidden || void refreshPulls(), 60_000);
    const slower = setInterval(() => document.hidden || void refreshReview(), 300_000);
    onCleanup(() => {
      window.removeEventListener('focus', refreshOld);
      clearInterval(timer);
      clearInterval(slower);
    });
  });
  return (
    <>
      <Yours />
      <ToReview />
      <Filter />
    </>
  );
}
