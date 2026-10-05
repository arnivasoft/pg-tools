import { Show } from "solid-js";
import type { Explorer } from "../features/explorer/explorer-store";
import { serverOf } from "../features/explorer/tree-model";
import { t } from "./strings";

export function StatusBar(props: { explorer: Explorer; notice: string | null }) {
  const server = () => serverOf(props.explorer.state(), props.explorer.state().selected);
  const text = () => {
    const s = server();
    if (!s) return t.status.ready;
    const parts = [s.label];
    if (s.detail) parts.push(`PostgreSQL ${s.detail}`);
    parts.push(s.flags.connected ? t.status.connected : t.status.notConnected);
    return parts.join(" · ");
  };
  return (
    <Show when={props.notice} fallback={<span class="truncate">{text()}</span>}>
      <span class="truncate text-danger">{props.notice}</span>
    </Show>
  );
}
