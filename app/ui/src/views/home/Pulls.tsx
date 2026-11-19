// Home's list of the pull requests you opened, from dova. A row that needs
// you says why under it: the check that failed, the thread still open.
// When dova is missing or signed out, the section says what to run.

import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { openFolderDialog } from '../../components/OpenFolder';
import { DaemonError } from '../../daemon';
import type { PullRow } from '../../protocol/PullRow';
import { health, recheck } from '../../state/health';
import { pulls, refreshPulls } from '../../state/pulls';
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
        <span class="dim">#{row().id}</span>
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
  const needYou = () => (pulls.latest ?? []).filter((row) => row.failed || row.thread).length;

  return (
    <section class="section" aria-labelledby="pulls">
      <div class="section-head">
        <h2 id="pulls">Pull requests</h2>
        <Show when={pulls.latest?.length}>
          {(n) => (
            <span class="dim">
              {n()} open{needYou() > 0 ? `, ${needYou()} need you` : ''}
            </span>
          )}
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
              <For
                each={rows()}
                fallback={
                  <p class="rows-none dim">
                    None open. They show here for every workspace cloned from Azure Repos.
                  </p>
                }
              >
                {(row) => <Row row={row} />}
              </For>
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
