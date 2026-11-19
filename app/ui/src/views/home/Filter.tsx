// Choosing whose pull requests, and in which repos, show under "To review" on
// Home. They are asked for when this closes, and only what was chosen.
// One field searches the organization's people, repos and projects, a few
// at a time, so it works the same with ten of them or ten thousand. With
// nothing typed it lists what you chose and the teams you are in: a team
// adds everyone in it at once.

import { createEffect, createMemo, createResource, createSignal, For, on, Show } from 'solid-js';
import { ask } from '../../daemon';
import type { Person } from '../../protocol/Person';
import type { RepoPick } from '../../protocol/RepoPick';
import { filter, filtering, repoName, setFilter, setFiltering } from '../../state/pulls';

const open = filtering;
const setOpen = setFiltering;

export const openFilter = () => setOpen(true);

/** One line of the list: what it is, and what Enter or a click on it does. */
interface Line {
  /** A heading over the lines after it, when this is the first of its group. */
  group: string;
  kind: string;
  title: string;
  detail?: string;
  /** The word at the right: what a click does, or that it is already chosen. */
  word: string;
  run: () => void;
}

const samePerson = (a: { id: string }) => (b: { id: string }) => a.id === b.id;
const sameRepo = (a: RepoPick) => (b: RepoPick) =>
  a.project.toLowerCase() === b.project.toLowerCase() &&
  (a.repo ?? '').toLowerCase() === (b.repo ?? '').toLowerCase();

export function Filter() {
  const [text, setText] = createSignal('');
  const [at, setAt] = createSignal(0);
  // What is searched for is what you typed a moment ago: one call when you pause, not one for each letter.
  const [asked, setAsked] = createSignal('');
  let pause: ReturnType<typeof setTimeout> | undefined;
  const type = (value: string) => {
    setText(value);
    setAt(0);
    clearTimeout(pause);
    pause = setTimeout(() => setAsked(value.trim()), 200);
  };
  const close = () => {
    setOpen(false);
    type('');
    setAsked('');
  };

  const [teams] = createResource(open, () =>
    ask({ method: 'pulls.teams' }, 'teams').then((got) => got.teams),
  );
  const [found] = createResource(
    () => (open() && asked().length > 1 ? asked() : false),
    (wanted) => ask({ method: 'pulls.find', params: { text: wanted } }, 'found'),
  );

  const addPeople = (people: Person[]) => {
    const next = [...filter().people];
    for (const person of people)
      if (!next.some(samePerson(person))) next.push({ id: person.id, name: person.name });
    setFilter({ ...filter(), people: next });
  };
  const dropPeople = (people: { id: string }[]) =>
    setFilter({ ...filter(), people: filter().people.filter((one) => !people.some(samePerson(one))) });
  const toggleRepo = (pick: RepoPick) => {
    const has = filter().repos.some(sameRepo(pick));
    const repos = has ? filter().repos.filter((one) => !sameRepo(pick)(one)) : [...filter().repos, pick];
    setFilter({ ...filter(), repos });
  };

  const personLine = (
    group: string,
    person: Person | { id: string; name: string; mail?: string | null },
  ): Line => {
    const chosen = filter().people.some(samePerson(person));
    return {
      group,
      kind: 'person',
      title: person.name,
      detail: person.mail ?? undefined,
      word: chosen ? 'remove' : 'add',
      run: () => (chosen ? dropPeople([person]) : addPeople([{ mail: null, ...person }])),
    };
  };
  const repoLine = (group: string, pick: RepoPick): Line => {
    const chosen = filter().repos.some(sameRepo(pick));
    return {
      group,
      kind: pick.repo === null ? 'project' : 'repo',
      title: repoName(pick),
      word: chosen ? 'remove' : 'add',
      run: () => toggleRepo(pick),
    };
  };
  const teamLine = (group: string, team: { name: string; project: string; members: Person[] }): Line => {
    const inIt = team.members.filter((member) => filter().people.some(samePerson(member))).length;
    const all = team.members.length > 0 && inIt === team.members.length;
    return {
      group,
      kind: 'team',
      title: team.name,
      detail: `${team.project} · ${team.members.length} ${team.members.length === 1 ? 'person' : 'people'}${inIt > 0 ? `, ${inIt} added` : ''}`,
      word: team.members.length === 0 ? '' : all ? 'remove all' : 'add all',
      run: () => (all ? dropPeople(team.members) : addPeople(team.members)),
    };
  };

  const lines = createMemo((): Line[] => {
    const typed = text().trim().toLowerCase();
    if (!typed) {
      return [
        ...filter().people.map((person) => personLine('Showing', person)),
        ...filter().repos.map((pick) => repoLine('Showing', pick)),
        ...(teams.latest ?? []).map((team) => teamLine('Your teams', team)),
      ];
    }
    const got = found.latest;
    return [
      ...(teams.latest ?? [])
        .filter((team) => team.name.toLowerCase().includes(typed))
        .map((team) => teamLine('Your teams', team)),
      ...(got?.people ?? []).map((person) => personLine('People', person)),
      ...(got?.repos ?? []).map((repo) => repoLine('Repos', repo)),
      ...(got?.projects ?? []).map((project) => repoLine('Projects', { project, repo: null })),
    ];
  });

  let list: HTMLDivElement | undefined;
  createEffect(
    on(at, (i) => list?.querySelectorAll('.palette-row')[i]?.scrollIntoView({ block: 'nearest' }), {
      defer: true,
    }),
  );
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'ArrowDown') setAt(Math.min(at() + 1, lines().length - 1));
    else if (e.key === 'ArrowUp') setAt(Math.max(at() - 1, 0));
    else if (e.key === 'Enter') lines()[at()]?.run();
    else if (e.key === 'Escape') close();
    else return;
    e.preventDefault();
  };

  const both = () => filter().people.length > 0 && filter().repos.length > 0;
  const searching = () => text().trim().length > 1 && (found.loading || asked() !== text().trim());
  const failure = () => {
    const error: unknown = found.error ?? teams.error;
    return error instanceof Error ? error.message : String(error);
  };

  return (
    <Show when={open()}>
      <div class="scrim">
        <button type="button" class="scrim-close" aria-label="Close" onClick={close} />
        <div class="palette filter" role="dialog" aria-label="What to review">
          <input
            ref={(el) => requestAnimationFrame(() => el.focus())}
            class="palette-input"
            placeholder="Add a person, a team, a repo or a project"
            aria-label="Add a person, a team, a repo or a project"
            autocomplete="off"
            value={text()}
            onInput={(e) => type(e.currentTarget.value)}
            onKeyDown={onKey}
          />
          <div class="palette-rows" ref={list}>
            <For each={lines()}>
              {(line, i) => (
                <>
                  <Show when={i() === 0 || lines()[i() - 1].group !== line.group}>
                    <p class="filter-head dim">{line.group}</p>
                  </Show>
                  <button
                    type="button"
                    class="row palette-row"
                    aria-current={at() === i() ? 'true' : undefined}
                    onMouseEnter={() => setAt(i())}
                    onClick={line.run}
                  >
                    <span class="filter-kind dim">{line.kind}</span>
                    <span class="palette-title">{line.title}</span>
                    <span class="palette-detail dim">{line.detail}</span>
                    <span class="filter-word" data-word={line.word}>
                      {line.word}
                    </span>
                  </button>
                </>
              )}
            </For>
            <Show when={found.error || teams.error}>
              <p class="palette-none note-failed">{failure()}</p>
            </Show>
            <Show when={!text().trim() && filter().people.length + filter().repos.length === 0}>
              <p class="palette-none dim">
                Nothing is chosen, so nothing is loaded. Type a name, or add a team.
              </p>
            </Show>
            <Show when={!text().trim() && teams.loading}>
              <p class="palette-none dim">Asking for your teams</p>
            </Show>
            <Show when={text().trim().length === 1}>
              <p class="palette-none dim">Type one more letter.</p>
            </Show>
            <Show when={searching()}>
              <p class="palette-none dim">Searching</p>
            </Show>
            <Show when={text().trim().length > 1 && !searching() && !found.error && lines().length === 0}>
              <p class="palette-none dim">No person, team, repo or project has that in its name.</p>
            </Show>
          </div>
          <div class="folder-foot">
            <Show when={both()}>
              <span class="dim">A pull request shows when</span>
              <button
                type="button"
                class="quiet"
                aria-pressed={!filter().either}
                title="By one of the people, and in one of the repos"
                onClick={() => setFilter({ ...filter(), either: false })}
              >
                both match
              </button>
              <button
                type="button"
                class="quiet"
                aria-pressed={filter().either}
                title="By one of the people, or in one of the repos"
                onClick={() => setFilter({ ...filter(), either: true })}
              >
                either matches
              </button>
            </Show>
            <span class="top-gap" />
            <Show when={filter().people.length + filter().repos.length > 0}>
              <button
                type="button"
                class="button"
                onClick={() => setFilter({ people: [], repos: [], either: filter().either })}
              >
                Clear
              </button>
            </Show>
            <button type="button" class="button button-primary" onClick={close}>
              Done
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
