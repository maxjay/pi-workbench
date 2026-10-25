// Pi waiting on you: an extension's dialog (pick one, yes or no, type or
// edit something), or questions from the `ask_user` tool, one or several
// in tabs, each answered by picking a choice or writing your own. The run
// carries on once it's answered; Dismiss, or Escape, answers nothing.

import { createSignal, For, Match, Show, Switch } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import type { Ask } from '../../protocol/Ask';
import type { AskReply } from '../../protocol/AskReply';
import type { Question } from '../../protocol/Question';
import { answer } from '../../state/sessions';

export function AskCard(props: { session: string; ask: Ask }) {
  const reply = createAction((value: AskReply) => answer(props.session, props.ask.id, value));
  const dismiss = () => void reply.run({ type: 'cancelled' });
  const kind = () => props.ask.kind;

  return (
    <section
      class="ask-card"
      aria-label={props.ask.title}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          dismiss();
        }
      }}
    >
      <Switch>
        <Match when={kind().type === 'questions' && kind()}>
          {(questions) => (
            <Questions
              questions={(questions() as { questions: Question[] }).questions}
              busy={reply.busy()}
              onAnswer={(answers) => void reply.run({ type: 'answers', answers })}
              onDismiss={dismiss}
            />
          )}
        </Match>
        <Match when={kind().type !== 'questions'}>
          <p class="ask-title">{props.ask.title}</p>
          <Show when={props.ask.message}>
            <p class="ask-message">{props.ask.message}</p>
          </Show>
          <Dialog
            ask={props.ask}
            busy={reply.busy()}
            onReply={(value) => void reply.run(value)}
            onDismiss={dismiss}
          />
        </Match>
      </Switch>
      <ErrorNote error={reply.error()} />
    </section>
  );
}

function DismissButton(props: { busy: boolean; onDismiss: () => void }) {
  return (
    <button type="button" class="button button-text" disabled={props.busy} onClick={props.onDismiss}>
      Dismiss
    </button>
  );
}

/** An extension's dialog, as Pi's RPC mode sends them. */
function Dialog(props: {
  ask: Ask;
  busy: boolean;
  onReply: (reply: AskReply) => void;
  onDismiss: () => void;
}) {
  const initial = () => (props.ask.kind.type === 'editor' ? props.ask.kind.prefill : '');
  const [text, setText] = createSignal(initial());
  const submit = () => props.onReply({ type: 'value', value: text() });

  return (
    <Switch>
      <Match when={props.ask.kind.type === 'select' && props.ask.kind}>
        {(kind) => (
          <div class="ask-options">
            <For each={kind().options}>
              {(option, i) => (
                <button
                  type="button"
                  class="button"
                  classList={{ 'button-primary': i() === 0 }}
                  disabled={props.busy}
                  onClick={() => props.onReply({ type: 'value', value: option })}
                >
                  {option}
                </button>
              )}
            </For>
            <DismissButton busy={props.busy} onDismiss={props.onDismiss} />
          </div>
        )}
      </Match>
      <Match when={props.ask.kind.type === 'confirm'}>
        <div class="ask-options">
          <button
            type="button"
            class="button button-primary"
            disabled={props.busy}
            onClick={() => props.onReply({ type: 'confirmed', confirmed: true })}
          >
            Yes
          </button>
          <button
            type="button"
            class="button"
            disabled={props.busy}
            onClick={() => props.onReply({ type: 'confirmed', confirmed: false })}
          >
            No
          </button>
          <DismissButton busy={props.busy} onDismiss={props.onDismiss} />
        </div>
      </Match>
      <Match when={props.ask.kind.type === 'input' && props.ask.kind}>
        {(kind) => (
          <div class="ask-options">
            <input
              class="ask-input"
              aria-label={props.ask.title}
              placeholder={kind().placeholder ?? ''}
              value={text()}
              onInput={(e) => setText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  submit();
                }
              }}
            />
            <button type="button" class="button button-primary" disabled={props.busy} onClick={submit}>
              Send
            </button>
            <DismissButton busy={props.busy} onDismiss={props.onDismiss} />
          </div>
        )}
      </Match>
      <Match when={props.ask.kind.type === 'editor'}>
        <textarea
          class="ask-input ask-editor mono"
          aria-label={props.ask.title}
          rows={6}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
        />
        <div class="ask-options">
          <button type="button" class="button button-primary" disabled={props.busy} onClick={submit}>
            Save
          </button>
          <DismissButton busy={props.busy} onDismiss={props.onDismiss} />
        </div>
      </Match>
    </Switch>
  );
}

/**
 * The `ask_user` tool's questions. With several, each has a tab: answering
 * one moves on to the next unanswered, and Submit sends them all once
 * every one has an answer. A single question is sent as soon as it's answered.
 */
function Questions(props: {
  questions: Question[];
  busy: boolean;
  onAnswer: (answers: string[]) => void;
  onDismiss: () => void;
}) {
  const [tab, setTab] = createSignal(0);
  const [answers, setAnswers] = createSignal<(string | undefined)[]>([]);
  const [typed, setTyped] = createSignal('');
  const question = () => props.questions[tab()];
  const several = () => props.questions.length > 1;
  const picked = (i: number) => answers()[i];
  /** What's written in the box for this question: its answer, if that isn't one of the choices. */
  const own = (i: number) => {
    const answer = picked(i);
    return answer !== undefined && !props.questions[i].choices.some((c) => c.label === answer) ? answer : '';
  };
  const go = (i: number) => {
    setTab(i);
    setTyped(own(i));
  };
  const lastOne = () => props.questions.every((_, i) => i === tab() || picked(i) !== undefined);

  const record = (answer: string) => {
    const next = [...answers()];
    next[tab()] = answer;
    setAnswers(next);
    if (props.questions.every((_, i) => next[i] !== undefined)) {
      if (!several()) props.onAnswer(next as string[]);
      return;
    }
    const after = props.questions.findIndex((_, i) => i > tab() && next[i] === undefined);
    go(after >= 0 ? after : props.questions.findIndex((_, i) => next[i] === undefined));
  };
  const send = () => {
    const text = typed().trim();
    const next = [...answers()];
    if (text) next[tab()] = text;
    if (props.questions.every((_, i) => next[i] !== undefined)) return props.onAnswer(next as string[]);
    if (text) return record(text);
    const other = props.questions.findIndex((_, i) => next[i] === undefined && i !== tab());
    if (other >= 0) go(other);
  };

  return (
    <>
      <Show when={several()}>
        <div class="ask-tabs" role="tablist" aria-label="Questions">
          <For each={props.questions}>
            {(q, i) => (
              <button
                type="button"
                role="tab"
                class="ask-tab"
                aria-selected={i() === tab()}
                data-answered={picked(i()) !== undefined}
                onClick={() => go(i())}
              >
                <span>{picked(i()) !== undefined ? '✓' : i() + 1}</span>
                <span>{q.label}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <p class="ask-title">{question().question}</p>
      <div class="ask-choices">
        <For each={question().choices}>
          {(choice, i) => (
            <button
              type="button"
              class="ask-choice"
              aria-pressed={picked(tab()) === choice.label}
              disabled={props.busy}
              onClick={() => record(choice.label)}
            >
              <span class="ask-choice-n">{picked(tab()) === choice.label ? '✓' : i() + 1}</span>
              <span class="ask-choice-text">
                <span>{choice.label}</span>
                <Show when={choice.description}>
                  <span class="ask-choice-description">{choice.description}</span>
                </Show>
              </span>
            </button>
          )}
        </For>
      </div>
      <div class="ask-options">
        <Show when={question().other}>
          <input
            class="ask-input"
            aria-label="Your own answer"
            placeholder="Type something…"
            value={typed()}
            onInput={(e) => setTyped(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                send();
              }
            }}
          />
        </Show>
        <Show when={question().other || several()}>
          <button type="button" class="button button-primary" disabled={props.busy} onClick={send}>
            {!several() ? 'Answer' : lastOne() ? 'Submit' : 'Next'}
          </button>
        </Show>
        <DismissButton busy={props.busy} onDismiss={props.onDismiss} />
      </div>
    </>
  );
}
