// Home's two lists of pull requests. "Pull requests" has the ones you opened,
// from dova, in the whole organization of each folder you have open. A row
// that needs you says why under it: the check that failed, the thread still
// open. When dova is missing or signed out, the section says what to run.
// When there is nothing to list, it says what was asked and what was not.
//
// "To review" has the open pull requests of other people that you chose to
// see: by these people, in these repos. Nothing is asked for until you
// choose, because an organization can have thousands. Each is one line and
// says whether you or a team of yours is a reviewer.

import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
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
      <span title={`${row().project}/${row().repo}`}>{row().repo}</span>
      <span class="dim">{row().author}</span>
      <span classList={{ dim: row().isDraft }}>{row().isDraft ? 'draft' : row().status}</span>
      <Show when={row().review} fallback={<span class="dim">not asked</span>}>
        {(review) => <span classList={{ dim: review().vote === 'none' }}>{review().vote}</span>}
      </Show>
      <span class="dim">{row().review ? (row().review?.team ?? 'you') : ''}</span>
    </button>
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
          <div class="rows rows-review">
            <Show
              when={rows().length > 0}
              fallback={<p class="rows-none dim">None open {where()} you chose, apart from your own.</p>}
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
              <For each={rows()}>{(row) => <ReviewRow row={row} />}</For>
            </Show>
          </div>
        </Match>
        <Match when={toReview.loading || filtering()}>
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
