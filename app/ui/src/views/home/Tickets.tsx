// Home's list of your tickets, from dova: the ones assigned to you that
// are not finished. Type in the field to find any ticket by its title. A
// ticket opens in a tab of its own.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ticketHue } from '../../components/hues';
import { pulls } from '../../state/pulls';
import { open } from '../../state/tabs';
import { fileTicket, findTickets, tickets, words } from '../../state/tickets';
import { workspaceById } from '../../state/workspaces';

export function Tickets() {
  const [title, setTitle] = createSignal('');
  /** A folder to file a new ticket from: one that a ticket or a pull request was listed from. */
  const from = () => tickets.latest?.[0]?.workspace ?? pulls.latest?.find((row) => row.workspace)?.workspace;
  /** The type last asked for, kept so a team can be picked for it. */
  const [kind, setKind] = createSignal('Task');
  const filing = createAction(async (type: string, team: string | null = null) => {
    const [workspace, name] = [from(), title().trim()];
    const dir = workspace && workspaceById(workspace)?.workspace.path;
    if (!workspace || !dir || !name) return;
    setKind(type);
    const id = await fileTicket(dir, name, type, team);
    setTitle('');
    open(`wi:${workspace}:${id}`);
  });
  /** A project with more than one team: dova says which there are, and one is picked here. */
  const teams = () => {
    const listed = filing.error()?.match(/Which team[\s\S]*?Available: ([^(]+)/)?.[1];
    return listed
      ? listed
          .split(',')
          .map((team) => team.trim())
          .filter(Boolean)
      : [];
  };

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
              <span style={{ color: ticketHue(row.type) }}>#{row.id}</span>
              <span class="dim">{row.type}</span>
              <span class="pull-title">{row.title}</span>
              <span class="dim">{row.state}</span>
            </button>
          )}
        </For>
        <Show when={from()}>
          <div class="row ticket-new">
            <input
              class="ticket-new-input"
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
      <Show
        when={teams().length > 0}
        fallback={<Show when={filing.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>}
      >
        <p class="ticket-teams">
          <span class="dim">Which team's?</span>
          <For each={teams()}>
            {(team) => (
              <button
                type="button"
                class="quiet"
                disabled={filing.busy()}
                onClick={() => void filing.run(kind(), team)}
              >
                {team}
              </button>
            )}
          </For>
        </p>
      </Show>
    </section>
  );
}
