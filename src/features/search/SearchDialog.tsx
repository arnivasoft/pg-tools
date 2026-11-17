// "Nesne ara…" diyaloğu (tembel parça; Plan 32). Durum `search-model`dedir; bu dosya yalnız çizer.
// Ana modülleri (`app/strings`, `explorer/*`, `tree-model`) içe aktarmaz: metin `strings-search`ten, bağlanma/gitme/bildirim
// App'ten props ile gelir. Arama metni ve nesne adları günlüğe ve bildirime yazılmaz.
// Klavye: odak arama kutusunda kalır (combobox); ↓/↑ etkin sonucu değiştirir, Enter etkin sonuca gider (yoksa hemen arar),
// Esc kapatır (Kobalte).
import { createEffect, createUniqueId, For, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { ts } from "../../app/strings-search";
import type { ToastKind } from "../../app/toast-model";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { SearchHit } from "../../ipc/bindings/SearchHit";
import type { SearchKind } from "../../ipc/bindings/SearchKind";
import { toPayload } from "../../ipc/errors";
import { Dialog } from "../../ui/Dialog";
import { inputClass } from "../../ui/Field";
import { searchObjects } from "./search-commands";
import { ALL_KINDS, createSearch } from "./search-model";

export type SearchTarget = { serverId: string; databaseOid: number; databaseName: string; serverName: string };

export type SearchDialogProps = {
  /** Açık diyaloğun hedefi; null = kapalı. */
  target: SearchTarget | null;
  connect: (serverId: string) => Promise<boolean>;
  /** Sonuca git: App ağaçta açar; `false` → `notify(ts.notFound, "error")` (ana pakete metin girmez). */
  onReveal: (hit: SearchHit, serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  onClose: () => void;
};

/** Ağaç türünün etiketi (`ts.kinds` arama türleriyle adlandırılır; bölüm ağaçta tablo türündedir). */
const kindLabel = (k: NodeKind) => ts.kinds[(k === "materializedView" ? "matview" : k) as SearchKind] ?? k;

export function SearchDialog(props: SearchDialogProps) {
  return (
    <Show when={props.target} keyed>
      {(target) => <SearchPanel target={target} props={props} />}
    </Show>
  );
}

function SearchPanel(p: { target: SearchTarget; props: SearchDialogProps }) {
  const props = untrack(() => p.props);
  const target = untrack(() => p.target);
  const search = createSearch(
    {
      search: searchObjects,
      connect: (id) => props.connect(id),
      setTimer: (fn, ms) => setTimeout(fn, ms),
      clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    },
    { serverId: target.serverId, databaseOid: target.databaseOid },
  );
  onCleanup(search.dispose);
  const s = search.state;
  const baseId = createUniqueId();
  const listId = `${baseId}-list`;
  const optId = (i: number) => `${baseId}-o${i}`;
  let inputEl: HTMLInputElement | undefined;

  // Etkin satır görünür kalır (liste kaydırılır; odak arama kutusunda).
  createEffect(
    on(
      () => s().active,
      (i) => {
        if (i >= 0) document.getElementById(optId(i))?.scrollIntoView({ block: "nearest" });
      },
    ),
  );

  const go = (hit: SearchHit) => {
    const { notify, onReveal } = props;
    props.onClose();
    void onReveal(hit, target.serverId).then(
      (ok) => ok || notify(ts.notFound, "error"),
      (e) => notify(toPayload(e).message, "error"),
    );
  };

  const statusText = () => {
    const cur = s();
    if (cur.status === "searching") return ts.searching;
    if (cur.status === "error") return cur.error ?? "";
    if (cur.status === "done") return cur.hits.length === 0 ? ts.none : ts.count(cur.hits.length);
    return cur.query.trim() === "" ? ts.hint : "";
  };

  const onKeyDown: JSX.EventHandler<HTMLInputElement, KeyboardEvent> = (e) => {
    if (e.isComposing || e.keyCode === 229 || e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      search.move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = search.activeHit();
      if (hit) go(hit);
      else search.searchNow();
    }
  };

  const check = "h-3.5 w-3.5 accent-accent";

  return (
    <Dialog
      open
      onOpenChange={(o) => o || props.onClose()}
      title={ts.title(target.databaseName, target.serverName)}
      width="640px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        inputEl?.focus();
        inputEl?.select();
      }}
    >
      <div class="flex flex-col gap-3">
        <input
          ref={inputEl}
          class={inputClass}
          type="text"
          role="combobox"
          aria-label={ts.placeholder}
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={s().hits.length > 0}
          aria-activedescendant={s().active >= 0 && s().active < s().hits.length ? optId(s().active) : undefined}
          placeholder={ts.placeholder}
          spellcheck={false}
          autocomplete="off"
          value={s().query}
          onInput={(e) => search.setQuery(e.currentTarget.value)}
          onKeyDown={onKeyDown}
        />
        <fieldset class="flex flex-wrap gap-x-3 gap-y-1 text-[12px]">
          <legend class="sr-only">{ts.filterLabel}</legend>
          <label class="flex items-center gap-1.5">
            <input
              type="checkbox"
              class={check}
              checked={s().kinds.length === 0}
              onChange={(e) => {
                search.setAllKinds();
                // "Tümü" işaretliyken tıklama onu kaldıramaz: durum değişmediği için kutu elle eşitlenir.
                e.currentTarget.checked = true;
              }}
            />
            {ts.all}
          </label>
          <For each={ALL_KINDS}>
            {(k) => (
              <label class="flex items-center gap-1.5">
                <input type="checkbox" class={check} checked={s().kinds.includes(k)} onChange={() => search.toggleKind(k)} />
                {ts.kinds[k]}
              </label>
            )}
          </For>
        </fieldset>
        <div role="status" aria-live="polite" class={`text-[12px] ${s().status === "error" ? "text-danger" : "text-muted"}`}>
          {statusText()}
          <Show when={s().status === "done" && s().truncated}>
            {" "}
            <span class="text-warn-text">{ts.truncated}</span>
          </Show>
        </div>
        <ul
          id={listId}
          role="listbox"
          aria-label={ts.listLabel}
          class="max-h-[50vh] min-h-[8rem] overflow-auto rounded border border-border"
        >
          <For each={s().hits}>
            {(hit, i) => (
              <li
                id={optId(i())}
                role="option"
                aria-selected={i() === s().active}
                class="flex cursor-default items-baseline gap-2 px-2 py-1 text-[13px]"
                classList={{ "bg-selected": i() === s().active, "hover:bg-hover": i() !== s().active, "text-muted": hit.system }}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => search.setActive(i())}
                onDblClick={() => go(hit)}
              >
                <span class="w-32 shrink-0 truncate text-[11px] text-muted">{kindLabel(hit.kind)}</span>
                <span class="min-w-0 truncate font-medium">{hit.name}</span>
                <span class="ml-auto min-w-0 shrink truncate text-[12px] text-muted">
                  {[hit.schema, hit.owner].filter(Boolean).join(".")}
                </span>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Dialog>
  );
}
