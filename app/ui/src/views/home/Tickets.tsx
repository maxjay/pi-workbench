// Home's list of your tickets, from dova: the ones assigned to you that
// are not finished. Type in the field to find any ticket by its title. A
// ticket opens in a tab of its own.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { pulls } from '../../state/pulls';
import { open } from '../../state/tabs';
import { fileTicket, findTickets, tickets, words } from '../../state/tickets';
import { workspaceById } from '../../state/workspaces';

export function Tickets() {
  const [title, setTitle] = createSignal('');
  /** A folder to file a new ticket from: one that a ticket or a pull request was listed from. */
  const from = () => tickets.latest?.[0]?.workspace ?? pulls.latest?.find((row) => row.workspace)?.workspace;
  const filing = createAction(async (kind: string) => {
    const [workspace, name] = [from(), title().trim()];
    const dir = workspace && workspaceById(workspace)?.workspace.path;
    if (!workspace || !dir || !name) return;
    const id = await fileTicket(dir, name, kind);
    setTitle('');
    open(`wi:${workspace}:${id}`);
  });

  return (
    <section class="section" aria-labelledby="tickets">
      <div class="section-head">
        <h2 id="tickets">Tickets</h2>
        <Show when={tickets.latest}>
          {(rows) => (
            <span class="dim">
              {rows().length} {words() ? 'found' : 'yours'}
            </span>
          )}
        </Show>
        <span class="top-gap" />
        <input
          class="field tickets-find"
          placeholder="find"
          aria-label="Find a ticket by its title"
          onInput={(e) => findTickets(e.currentTarget.value)}
        />
      </div>
      <Show when={tickets.error}>
        <p class="note-failed">{String(tickets.error?.message ?? tickets.error)}</p>
      </Show>
      <div class="rows">
        <For
          each={tickets.latest}
          fallback={
            <p class="rows-none dim">
              {tickets.loading ? 'Asking dova' : words() ? 'None found.' : 'None assigned to you.'}
            </p>
          }
        >
          {(row) => (
            <button
              type="button"
              class="row ticket-row"
              onClick={() => open(`wi:${row.workspace}:${row.id}`)}
            >
              <span class="dim">#{row.id}</span>
              <span class="dim">{row.type}</span>
              <span class="pull-title">{row.title}</span>
              <span class="dim">{row.state}</span>
            </button>
          )}
        </For>
        <Show when={from()}>
          <div class="row ticket-new">
            <input
              class="thread-input"
              placeholder="New ticket"
              aria-label="A new ticket's title"
              value={title()}
              onInput={(e) => setTitle(e.currentTarget.value)}
            />
            <Show when={title().trim()}>
              <button
                type="button"
                class="quiet"
                disabled={filing.busy()}
                onClick={() => void filing.run('Bug')}
              >
                Bug
              </button>
              <button
                type="button"
                class="quiet"
                disabled={filing.busy()}
                onClick={() => void filing.run('Task')}
              >
                Task
              </button>
            </Show>
          </div>
        </Show>
      </div>
      <Show when={filing.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
    </section>
  );
}
