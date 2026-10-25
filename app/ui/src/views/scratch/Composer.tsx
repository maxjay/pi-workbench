// Where you talk to Pi, as in a terminal: Enter sends, and while Pi works
// it steers the run; Alt+Enter waits for the run to finish instead; Escape
// stops it. `@` finds a file, `/` one of Pi's commands, and `!` runs the
// shell. The model and thinking level for this conversation open from the
// chip below.

import { createEffect, createSignal, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { abort, draftKey, drafts, send, setDrafts } from '../../state/sessions';
import { learnModels, ModelMenu, modelLabel } from './ModelMenu';
import { createPicker } from './Picker';

export function Composer(props: { session: Session }) {
  const key = () => draftKey(props.session);
  const text = () => drafts[key()] ?? '';
  const setText = (value: string) => setDrafts(key(), value);
  const [menu, setMenu] = createSignal(false);
  let box: HTMLTextAreaElement | undefined;
  let mirror!: HTMLDivElement;
  const pick = createPicker({ session: () => props.session, text, setText, box: () => box });
  createEffect(() => {
    if (props.session.id) learnModels(props.session.id).catch(() => {});
  });
  const busy = () => props.session.state.busy;
  const stopped = () => props.session.state.stopped !== null;
  const sending = createAction(async (later: boolean) => {
    const message = text().trim();
    if (!message) return;
    const was = key();
    await send(props.session, message, later);
    setDrafts(was, '');
  });
  // What was waiting comes back into the box, to send again or not.
  const stopping = createAction(async () => {
    const unsent = await abort(props.session.id);
    if (unsent.length > 0) setText([text(), ...unsent].filter((t) => t.trim()).join('\n'));
  });

  const onKey = (e: KeyboardEvent) => {
    if (pick.onKey(e)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void sending.run(e.altKey);
    } else if (e.key === 'Escape' && busy()) {
      e.preventDefault();
      e.stopPropagation();
      void stopping.run();
    }
  };

  const placeholder = () => {
    if (busy()) return 'Steer it, or ask something new…';
    if (props.session.state.done) return 'Ask a follow-up…';
    return 'Message Pi…';
  };

  return (
    <div class="composer">
      <div class="composer-box">
        <Show when={menu()}>
          <ModelMenu session={props.session} onClose={() => setMenu(false)} />
        </Show>
        {pick.view()}
        {/* The box's text is drawn again behind it, so picked mentions can be highlighted. */}
        <div class="composer-field">
          <div class="composer-mirror" aria-hidden="true" ref={mirror}>
            {pick.mirror()}
            {'\u200b'}
          </div>
          <textarea
            ref={box}
            class="composer-input"
            rows={2}
            aria-label="Message Pi"
            placeholder={placeholder()}
            value={text()}
            disabled={stopped()}
            onInput={(e) => {
              setText(e.currentTarget.value);
              pick.follow(e.currentTarget);
              pick.reset();
            }}
            onKeyUp={(e) => pick.follow(e.currentTarget)}
            onClick={(e) => pick.follow(e.currentTarget)}
            onKeyDown={onKey}
            onScroll={(e) => {
              mirror.scrollTop = e.currentTarget.scrollTop;
            }}
          />
        </div>
        <div class="composer-bar">
          <button
            type="button"
            class="chip"
            aria-haspopup="dialog"
            aria-expanded={menu()}
            title="Change the model"
            onClick={() => setMenu(!menu())}
          >
            <span class="chip-dot" aria-hidden="true" />
            <span class="chip-name">{modelLabel(props.session.state.model)}</span>
            <Show when={props.session.state.thinking && props.session.state.levels.length > 1}>
              <span class="chip-detail">· {props.session.state.thinking}</span>
            </Show>
            <span class="chevron" data-open={menu()}>
              <Icon name="chevron" size={10} />
            </span>
          </button>
          <Show
            when={busy()}
            fallback={
              <>
                <span class="composer-hint">
                  <span class="mono">/</span> commands · <span class="mono">@</span> files ·{' '}
                  <span class="mono">!</span> shell
                </span>
                <button
                  type="button"
                  class="send"
                  aria-label="Send"
                  disabled={stopped() || sending.busy() || !text().trim()}
                  onClick={() => void sending.run(false)}
                >
                  <Icon name="send" size={16} />
                </button>
              </>
            }
          >
            <span class="composer-hint">Enter steers · Alt+Enter waits for the run · Esc stops</span>
            <button
              type="button"
              class="send send-stop"
              aria-label="Stop Pi (Esc)"
              title="Stop Pi (Esc)"
              disabled={stopping.busy()}
              onClick={() => void stopping.run()}
            >
              <Icon name="stop" size={13} />
            </button>
          </Show>
        </div>
      </div>
      <ErrorNote error={sending.error() ?? stopping.error()} />
    </div>
  );
}
