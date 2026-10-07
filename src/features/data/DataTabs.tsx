// Sağ paneldeki veri sekme başlıkları ve panelleri (tembel parça; denetleyiciyle birlikte iner). Başlık kabuğu (`Head`)
// QueryTabs'tan gelir. Paket bütçesi (Plan 17 Karar 2): veri modülleri YALNIZCA bu parçadan statik erişilir; `src/ui/*`
// (ConfirmDialog/Dialog/Button) ve lucide simgeleri içe aktarılmaz (paylaşılan parça doğar, ana `__vite__mapDeps` büyür):
// onay diyaloğu küçük yerel bileşendir.
//
// Kirli sekmede "düzenlemeler atılır" onayı: kapatma (`closing`), F5/yeniden yükle (`askReload`, denetleyicide),
// sıralama/filtre ve tüm düzenlemeleri atma (yerel istek). Odak Vazgeç'te (Enter atmayı onaylamaz), Esc vazgeçer.
import { For, Show, createMemo, createSignal, createUniqueId, onCleanup, onMount, type Component, type JSX } from "solid-js";
import { tdata } from "../../app/strings-data";
import type { DataTab } from "../query/tabs-store";
import { ChangePanel } from "./ChangePanel";
import { DataBanner } from "./DataBanner";
import { DataGrid, type GridCommands } from "./DataGrid";
import { BTN, BTN_DANGER, DataToolbar } from "./DataToolbar";
import type { CellValue } from "../../ipc/bindings/CellValue";
import { EMPTY_EDIT, addRow as addRowEdit, changeCount, isDirty as editDirty, toChangeSet } from "./data-edit-model";
import type { DataModel } from "./data-controller";
import {
  editBlock, editableAt, gateReason, issueMaps, revertValue, rowCountOf, setValue, slotAt, toggleRowDelete, type Pos,
} from "./data-grid-model";
import type { DataController } from "./data-open";
import { cycleOrder, type SelectState } from "./data-select-model";

// Denetleyici ve açıcı bu parçayla iner (data-open.ts bağımlılıksız kalır; ana paket bütçesi, Plan 17 R1).
export { startData } from "./data-opener";

export type TabHeadComponent = Component<{ id: string; title: string; serverId: string | null; children: JSX.Element }>;

export function DataTabHeads(props: { data: DataController; tabs: DataTab[]; Head: TabHeadComponent }) {
  return (
    <For each={props.tabs}>
      {(tab) => {
        const m = () => props.data.model(tab.id);
        const down = () => m()?.status === "disconnected" || m()?.status === "gone";
        return (
          <props.Head id={tab.id} title={tab.title} serverId={tab.serverId}>
            <span class="truncate" classList={{ "opacity-60": down(), "line-through": m()?.status === "gone" }}>
              {tab.title}
            </span>
            <Show when={props.data.isDirty(tab.id)}>
              <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" title={tdata.tab.dirty}>
                <span class="sr-only">{tdata.sr.changed}</span>
              </span>
            </Show>
          </props.Head>
        );
      }}
    </For>
  );
}

/** Yerel onay isteği (sıralama/filtre ya da tüm düzenlemeleri atma). */
type Ask = { tabId: string; kind: "select"; next: SelectState } | { tabId: string; kind: "discard" };

interface AskView {
  title: string;
  message: string;
  confirm: string;
  answer: (ok: boolean) => void;
}

/**
 * Küçük erişilebilir onay: `role="alertdialog"`, `aria-modal`, başlık/ileti bağlı; odak açılışta Vazgeç'te, Esc vazgeçer,
 * Tab iki düğme arasında döner; kapanınca odak önceki öğeye döner.
 */
function AskDialog(props: { view: AskView }) {
  const uid = createUniqueId();
  let cancelEl!: HTMLButtonElement;
  let okEl!: HTMLButtonElement;
  const prev = document.activeElement as HTMLElement | null;
  onMount(() => cancelEl.focus());
  onCleanup(() => {
    if (prev?.isConnected) prev.focus();
  });
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      props.view.answer(false);
    } else if (e.key === "Tab") {
      e.preventDefault();
      (document.activeElement === cancelEl ? okEl : cancelEl).focus();
    }
  };
  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${uid}-t`}
        aria-describedby={`${uid}-m`}
        class="flex w-[400px] max-w-full flex-col rounded-md border border-border bg-bg text-[13px] text-fg shadow-xl"
        onKeyDown={onKeyDown}
      >
        <h2 id={`${uid}-t`} class="border-b border-border px-4 py-2.5 font-semibold">
          {props.view.title}
        </h2>
        <p id={`${uid}-m`} class="p-4">
          {props.view.message}
        </p>
        <div class="flex justify-end gap-2 border-t border-border px-4 py-2.5">
          <button ref={cancelEl} type="button" class={BTN} onClick={() => props.view.answer(false)}>
            {tdata.dirty.keep}
          </button>
          <button ref={okEl} type="button" class={BTN_DANGER} onClick={() => props.view.answer(true)}>
            {props.view.confirm}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Tek veri sekmesinin gövdesi: şeritler, araç çubuğu, ızgara, değişiklik paneli. */
function DataView(props: { data: DataController; m: DataModel; onAsk: (a: Ask) => void }) {
  const data = props.data;
  const id = () => props.m.tabId;
  const [active, setActive] = createSignal<Pos | null>(null);

  const blocked = createMemo(() => editBlock(props.m));
  // Alan hataları yalnızca güncel önizlemeye karşı (yoksa değişiklik indeksleri kaymış olabilir) hücreye yerleşir.
  const errors = createMemo(() => {
    const l = props.m.loaded;
    if (!l || !props.m.viewIsCurrent) return issueMaps(null, []);
    return issueMaps(props.m.issues, toChangeSet(l, props.m.edit).origin);
  });
  const dirty = () => editDirty(props.m.edit);
  const saveReason = () => gateReason(props.m.gate, props.m.confirm.checked);

  /** Etkin hücreye düzenleme uygular (kapalıysa hiçbir şey). */
  const onCell = (fn: CellValue | "revert") => {
    const p = active();
    if (!p || blocked()) return;
    data.edit(id(), (l, e) => {
      const s = slotAt(l, e, p.r);
      const col = l.source.columns[p.c];
      if (!s || !col) return e;
      return fn === "revert" ? revertValue(e, s, col.name) : setValue(l, e, s, col.name, fn);
    });
  };

  const requestSelect = (next: SelectState) => {
    if (dirty()) props.onAsk({ tabId: id(), kind: "select", next });
    else void data.setSelect(id(), next);
  };

  const cmd: GridCommands = {
    setNull: () => onCell({ kind: "null" }),
    setDefault: () => onCell({ kind: "default" }),
    revert: () => onCell("revert"),
    toggleDelete: () => {
      const p = active();
      if (!p || blocked()) return;
      data.edit(id(), (l, e) => {
        const s = slotAt(l, e, p.r);
        return s ? toggleRowDelete(l, e, s) : e;
      });
    },
    addRow: () => {
      if (blocked()) return;
      const box: { at: Pos | null } = { at: null };
      data.edit(id(), (l, e) => {
        const next = addRowEdit(l, e);
        if (next !== e) {
          const r = rowCountOf(l, next) - 1;
          const s = slotAt(l, next, r);
          // İlk yazılabilir kolon etkin olsun.
          const c = s ? l.source.columns.findIndex((col) => editableAt(l, next, s, col.name).editable) : -1;
          box.at = { r, c: Math.max(0, c) };
        }
        return next;
      });
      if (box.at) setActive(box.at);
    },
    sort: (column, additive) => {
      if (props.m.applying || props.m.loadingMore || props.m.status === "gone") return;
      requestSelect({ order: cycleOrder(props.m.select.order, column, additive), filter: props.m.select.filter });
    },
  };

  const activeDeleted = () => {
    const p = active();
    const l = props.m.loaded;
    const s = p && l ? slotAt(l, props.m.edit, p.r) : null;
    return s?.kind === "row" && props.m.edit.deleted.has(s.rowIndex);
  };

  const reload = () => void data.reloadNow(id());

  return (
    <div class="flex h-full min-h-0 flex-col">
      <DataBanner m={props.m} onReload={reload} onReconnect={() => void data.reconnect(id())} />
      <DataToolbar
        m={props.m}
        blocked={blocked()}
        activeDeleted={activeDeleted()}
        hasActive={active() !== null}
        dirty={dirty()}
        saveReason={saveReason()}
        cmd={cmd}
        onReload={reload}
        onDiscard={() => props.onAsk({ tabId: id(), kind: "discard" })}
        onSave={() => void data.apply(id())}
        onCancelApply={() => data.cancelApply(id())}
        onFilter={(filter) => {
          if (filter !== props.m.select.filter) requestSelect({ order: props.m.select.order, filter });
        }}
        onClearSort={() => requestSelect({ order: [], filter: props.m.select.filter })}
      />
      <div class="min-h-0 flex-1">
        <DataGrid data={data} m={props.m} active={active()} setActive={setActive} blocked={blocked()} errors={errors()} cmd={cmd} />
      </div>
      <ChangePanel
        m={props.m}
        saveReason={saveReason()}
        onConfirm={(checked) => data.setConfirm(id(), checked)}
        onSave={() => void data.apply(id())}
        onCancelApply={() => data.cancelApply(id())}
        onReload={reload}
      />
    </div>
  );
}


/** Her veri paneli bağlı kalır (kaydırma, düzenleme); gizlisi `invisible` (display:none kaydırmayı sıfırlar). */
export function DataPanels(props: {
  data: DataController;
  tabs: DataTab[];
  activeId: string;
  onClose: (tabId: string) => void;
  /** Kapatma onayı bekleyen (kirli) veri sekmesi; `onCloseAnswer(true)` = değişiklikleri at ve kapat. */
  closing: string | null;
  onCloseAnswer: (discard: boolean) => void;
}) {
  const [ask, setAsk] = createSignal<Ask | null>(null);
  const changes = (tabId: string) => {
    const m = props.data.model(tabId);
    return m?.loaded ? changeCount(m.loaded, m.edit) : 0;
  };
  // Tek diyalog; öncelik: kapatma > yerel istek > F5 (`askReload`). Anahtar değişince yeniden kurulur (odak Vazgeç'e).
  const which = createMemo((): string | null => {
    if (props.closing !== null) return `close:${props.closing}`;
    const a = ask();
    if (a) return `${a.kind}:${a.tabId}`;
    const r = props.tabs.find((t) => props.data.model(t.id)?.askReload);
    return r ? `reload:${r.id}` : null;
  });
  const viewOf = (key: string): AskView => {
    const sep = key.indexOf(":");
    const kind = key.slice(0, sep);
    const tabId = key.slice(sep + 1);
    if (kind === "close") {
      return {
        title: tdata.dirty.closeTitle, message: tdata.dirty.closeBody(changes(tabId)), confirm: tdata.dirty.closeDiscard,
        answer: (ok) => props.onCloseAnswer(ok),
      };
    }
    if (kind === "discard") {
      return {
        title: tdata.dirty.discardTitle, message: tdata.dirty.discardBody(changes(tabId)), confirm: tdata.dirty.discardConfirm,
        answer: (ok) => {
          setAsk(null);
          if (ok) props.data.edit(tabId, () => EMPTY_EDIT);
        },
      };
    }
    const a = ask();
    return {
      title: tdata.dirty.closeTitle, message: tdata.dirty.reloadBody, confirm: tdata.dirty.reload,
      answer: (ok) => {
        if (kind === "select") {
          setAsk(null);
          if (ok && a?.kind === "select") void props.data.setSelect(tabId, a.next, { discard: true });
        } else void props.data.answerReload(tabId, ok);
      },
    };
  };
  return (
    <>
      <For each={props.tabs}>
        {(tab) => (
          <div
            class="absolute inset-0 bg-bg"
            classList={{ "invisible pointer-events-none": props.activeId !== tab.id }}
            aria-hidden={props.activeId !== tab.id}
          >
            <Show when={props.data.model(tab.id)} fallback={<p class="p-3 text-muted">{tdata.tab.loading}</p>}>
              {(m) => <DataView data={props.data} m={m()} onAsk={setAsk} />}
            </Show>
          </div>
        )}
      </For>
      <Show when={which()} keyed>
        {(key) => <AskDialog view={viewOf(key)} />}
      </Show>
    </>
  );
}
