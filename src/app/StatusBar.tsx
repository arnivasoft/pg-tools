import type { Explorer } from "../features/explorer/explorer-store";
import { statusText } from "./status";

/** Derleme anında `vite.config.ts` → `define` ile gömülür (src-tauri/tauri.conf.json sürümü). */
declare const __APP_VERSION__: string;

export function StatusBar(props: { explorer: Explorer }) {
  return (
    <>
      <span class="truncate">{statusText(props.explorer.state())}</span>
      <span class="ml-auto shrink-0 pl-2 tabular-nums">v{__APP_VERSION__}</span>
    </>
  );
}
