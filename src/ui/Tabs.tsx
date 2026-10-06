import { Tabs as K } from "@kobalte/core/tabs";
import { For, type JSX } from "solid-js";

export function Tabs(props: {
  value: string;
  onChange: (value: string) => void;
  tabs: { value: string; label: string; muted?: boolean }[];
  children: JSX.Element;
  class?: string;
}) {
  return (
    <K value={props.value} onChange={props.onChange} class={`flex min-h-0 flex-col ${props.class ?? ""}`}>
      <K.List class="flex shrink-0 gap-1 border-b border-border px-2">
        <For each={props.tabs}>
          {(tab) => (
            <K.Trigger
              value={tab.value}
              class="border-b-2 border-transparent px-2 py-1.5 text-[13px] text-muted data-[selected]:border-accent data-[selected]:text-fg"
              classList={{ "opacity-60": tab.muted }}
            >
              {tab.label}
            </K.Trigger>
          )}
        </For>
      </K.List>
      {props.children}
    </K>
  );
}

export const TabPanel = K.Content;
