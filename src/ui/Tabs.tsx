import { Tabs as K } from "@kobalte/core/tabs";
import { For, Show, type JSX } from "solid-js";

export function Tabs(props: {
  value: string;
  onChange: (value: string) => void;
  /** `muted`: soluk ama seçilebilir sekme (italik; `mutedHint` ekran okuyucuya ve ipucuna nedeni söyler). */
  tabs: { value: string; label: string; muted?: boolean; mutedHint?: string }[];
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
              classList={{ italic: tab.muted }}
              title={tab.muted ? tab.mutedHint : undefined}
            >
              {tab.label}
              <Show when={tab.muted && tab.mutedHint}>
                <span class="sr-only">{` (${tab.mutedHint})`}</span>
              </Show>
            </K.Trigger>
          )}
        </For>
      </K.List>
      {props.children}
    </K>
  );
}

export const TabPanel = K.Content;
