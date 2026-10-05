import { Show, type JSX } from "solid-js";

export const inputClass =
  "h-7 w-full rounded border border-border bg-bg px-2 text-[13px] outline-none focus:border-accent disabled:opacity-60";

export function Field(props: { label: string; hint?: string; children: JSX.Element }) {
  return (
    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted">{props.label}</span>
      {props.children}
      <Show when={props.hint}>
        <span class="text-xs text-muted">{props.hint}</span>
      </Show>
    </label>
  );
}
