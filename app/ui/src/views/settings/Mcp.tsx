// mcp.json: the MCP servers Pi can use, each one on or off.

import { For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Empty, ErrorNote } from '../../components/Note';
import { Switch } from '../../components/Switch';
import type { McpServer } from '../../protocol/McpServer';
import { servers, setServerEnabled } from '../../state/settings';

export function Mcp() {
  const toggle = createAction(setServerEnabled);
  const list = () => (servers.error ? [] : (servers.latest ?? []));

  return (
    <section class="settings-section">
      <header class="section-head">
        <div>
          <h1>MCP servers</h1>
          <p class="section-sub">Every session can use the servers that are on.</p>
        </div>
      </header>
      <ErrorNote error={servers.error ?? toggle.error()} />
      <Show when={servers.state === 'ready' && list().length === 0}>
        <Empty>No servers in mcp.json yet. Add them under “mcpServers”.</Empty>
      </Show>
      <ul class="rows">
        <For each={list()}>
          {(server) => (
            <ServerRow server={server} busy={toggle.busy()} onToggle={(on) => toggle.run(server.name, on)} />
          )}
        </For>
      </ul>
    </section>
  );
}

function ServerRow(props: { server: McpServer; busy: boolean; onToggle: (on: boolean) => void }) {
  // Only the names: values are often tokens.
  const env = () => props.server.env.map((pair) => pair.split('=')[0]);
  return (
    <li class="row" data-off={!props.server.enabled}>
      <div class="row-main">
        <div class="row-title">
          <span>{props.server.name}</span>
          <span class="tag">{props.server.kind}</span>
        </div>
        <Show when={props.server.description}>
          <p class="row-detail">{props.server.description}</p>
        </Show>
        <p class="row-detail mono">{props.server.runs}</p>
        <Show when={env().length > 0}>
          <p class="row-detail">env: {env().join(', ')}</p>
        </Show>
      </div>
      <Switch
        on={props.server.enabled}
        label={`${props.server.name} on`}
        onChange={(on) => !props.busy && props.onToggle(on)}
      />
    </li>
  );
}
