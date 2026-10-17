import { For, Show, createContext, createUniqueId, useContext, type JSX } from "solid-js";
import { nextTabIndex, panelId, tabId, type TabKey } from "./tabstrip-model";

// Kobalte'siz hafif ARIA sekme şeridi (açılışta görünen Inspector için; ana pakete Kobalte sokmaz).
// Görünüm `ui/Tabs` ile aynı sınıfları kullanır.
const Ctx = createContext<{ value: () => string; base: string }>();

const KEYS = new Set<string>(["ArrowLeft", "ArrowRight", "Home", "End"]);

export function TabStrip(props: {
  value: string;
  onChange: (value: string) => void;
  /** `muted`: soluk ama seçilebilir sekme (italik; `mutedHint` ekran okuyucuya ve ipucuna nedeni söyler). */
  tabs: { value: string; label: string; muted?: boolean; mutedHint?: string }[];
  children: JSX.Element;
  class?: string;
}) {
  const base = createUniqueId();
  let list: HTMLDivElement | undefined;

  const onKeyDown = (ev: KeyboardEvent) => {
    // IME bileşimi sırasında tuşlar yutulmaz.
    if (ev.isComposing || ev.keyCode === 229) return;
    if (!KEYS.has(ev.key) || ev.ctrlKey || ev.altKey || ev.metaKey || ev.shiftKey) return;
    const current = props.tabs.findIndex((x) => x.value === props.value);
    const next = props.tabs[nextTabIndex(Math.max(current, 0), props.tabs.length, ev.key as TabKey)];
    if (!next) return;
    ev.preventDefault();
    props.onChange(next.value);
    // Otomatik etkinleştirme: odak da yeni sekmeye taşınır.
    list?.querySelector<HTMLElement>(`[id="${tabId(base, next.value)}"]`)?.focus();
  };

  return (
    <Ctx.Provider value={{ value: () => props.value, base }}>
      <div class={`flex min-h-0 flex-col ${props.class ?? ""}`}>
        <div ref={list} role="tablist" class="flex shrink-0 gap-1 border-b border-border px-2" onKeyDown={onKeyDown}>
          <For each={props.tabs}>
            {(tab) => {
              const selected = () => props.value === tab.value;
              return (
                <button
                  type="button"
                  role="tab"
                  id={tabId(base, tab.value)}
                  aria-selected={selected()}
                  aria-controls={panelId(base, tab.value)}
                  tabindex={selected() ? 0 : -1}
                  class="border-b-2 px-2 py-1.5 text-[13px]"
                  classList={{
                    "border-accent text-fg": selected(),
                    "border-transparent text-muted": !selected(),
                    italic: tab.muted,
                  }}
                  title={tab.muted ? tab.mutedHint : undefined}
                  onClick={() => props.onChange(tab.value)}
                >
                  {tab.label}
                  <Show when={tab.muted && tab.mutedHint}>
                    <span class="sr-only">{` (${tab.mutedHint})`}</span>
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
        {props.children}
      </div>
    </Ctx.Provider>
  );
}

/** Kobalte `Tabs.Content` ile aynı: seçili olmayan panel DOM'da tutulmaz (içindeki durum kaybolur). */
export function TabStripPanel(props: { value: string; class?: string; children: JSX.Element }) {
  const ctx = useContext(Ctx);
  return (
    <Show when={ctx?.value() === props.value}>
      <div
        role="tabpanel"
        id={ctx ? panelId(ctx.base, props.value) : undefined}
        aria-labelledby={ctx ? tabId(ctx.base, props.value) : undefined}
        tabindex={0}
        class={props.class}
      >
        {props.children}
      </div>
    </Show>
  );
}
