// Home's list of pull requests, from dova: the ones you opened, and under
// their own word the ones assigned to you to review. They come from the
// whole organization of each folder you have open, not only that folder's
// project. One field keeps only the repos you name. A row of yours that
// needs you says why under it: the check that failed, the thread still
// open. When dova is missing or signed out, the section says what to run.
// When there is nothing to list, it says what was asked and what was not.

import { createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { pullHue } from '../../components/hues';
import { openFolderDialog } from '../../components/OpenFolder';
import { DaemonError } from '../../daemon';
import type { PullRow } from '../../protocol/PullRow';
import { health, recheck } from '../../state/health';
import { asked, pulls, refreshPulls, repos, searched, setRepos } from '../../state/pulls';
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

/**
 * Whether a pull request is in a repo you named. Names are split by commas. A name matches a repo
 * or a project that has it anywhere in its own, whatever the case. With `*` it must match the whole
 * of it: `web-*`. A name with a slash is a project and a repo: `Shop/web`.
 */
function named(text: string): (row: PullRow) => boolean {
  const tests = text
    .split(',')
    .map((name) => name.trim().toLowerCase())
    .filter(Boolean)
    .map((name) => {
      if (!name.includes('*')) return (place: string) => place.includes(name);
      const whole = new RegExp(
        `^${name
          .split('*')
          .map((part) => part.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
          .join('.*')}$`,
      );
      return (place: string) => whole.test(place);
    });
  if (tests.length === 0) return () => true;
  return (row) => {
    const [project, repo] = [row.project.toLowerCase(), row.repo.toLowerCase()];
    return tests.some((test) => test(repo) || test(project) || test(`${project}/${repo}`));
  };
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
      <Show when={row().review}>
        {(review) => (
          <p class="pull-why">
            {row().author ?? 'Someone'} opened it · your vote: {review().vote}
            {review().team ? ` · asked as ${review().team}` : ''}
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

export function Pulls() {
  // Asked again each minute while Home is on screen.
  onMount(() => {
    const timer = setInterval(() => document.hidden || void refreshPulls(), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  const signedOut = () => pulls.error instanceof DaemonError && pulls.error.kind === 'notSignedIn';
  /** Why dova couldn't be asked, in its own words. */
  const failure = () => {
    const error: unknown = pulls.error;
    return error instanceof Error ? error.message : String(error);
  };
  /** The ones in the repos you named, or all of them. */
  const kept = createMemo(() => (pulls.latest ?? []).filter(named(repos())));
  const leftOut = () => (pulls.latest ?? []).length - kept().length;
  const yours = () => kept().filter((row) => !row.review);
  const toReview = () => kept().filter((row) => row.review);
  const needYou = () => yours().filter((row) => row.failed || row.thread).length;
  /** The folders that were passed over: their remote is not on Azure Repos, or they have none. */
  const passed = () => asked().filter((folder) => !folder.project);

  return (
    <section class="section" aria-labelledby="pulls">
      <div class="section-head">
        <h2 id="pulls">Pull requests</h2>
        <Show when={pulls.latest?.length}>
          <span class="dim">
            {yours().length} yours{needYou() > 0 ? `, ${needYou()} need you` : ''}, {toReview().length} to
            review{leftOut() > 0 ? `, ${leftOut()} left out` : ''}
          </span>
        </Show>
        <span class="top-gap" />
        <input
          class="field tickets-find"
          placeholder="repos: web, api"
          title="Only the repos you name, split by commas. Empty: all of them."
          aria-label="Only these repos"
          value={repos()}
          onInput={(e) => setRepos(e.currentTarget.value)}
        />
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
        <Match when={pulls.latest}>
          {(rows) => (
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
              <Show when={rows().length > 0}>
                <p class="pull-group dim">Yours · {yours().length}</p>
                <For each={yours()}>{(row) => <Row row={row} />}</For>
                <p class="pull-group dim">To review · {toReview().length}</p>
                <For each={toReview()}>{(row) => <Row row={row} />}</For>
              </Show>
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
                      <p>
                        None open that you opened or were asked to review. Asked: {searched().join(', ')}.
                      </p>
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
          )}
        </Match>
        <Match when={pulls.loading}>
          <p class="rows-none dim">Asking dova</p>
        </Match>
      </Switch>
    </section>
  );
}
