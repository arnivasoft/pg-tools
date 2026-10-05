import type { Explorer } from "../features/explorer/explorer-store";
import { statusText } from "./status";

export function StatusBar(props: { explorer: Explorer }) {
  return <span class="truncate">{statusText(props.explorer.state())}</span>;
}
