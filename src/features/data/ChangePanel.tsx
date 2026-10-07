// Değişiklik paneli (tembel DataTabs parçası): önizleme ifadeleri (kısaltılmış, görünür kaçışlı; "Kopyala" tam metni
// kopyalar), uyarılar, alan hataları, onay kutusu, Kaydet/İptal, sonuç şeridi ve başarılı uygulama notu (`aria-live`).
// `designer/*` (RiskBadge, SqlPreview) içe aktarılmaz (Plan 17 Karar 2): küçük yerel risk rozeti. SQL günlüğe/bildirime
// girmez; kopyalama geri bildirimi yalnızca yerel durum satırındadır.
import { For, Show, createSignal, createUniqueId } from "solid-js";
import { copyText } from "../../app/clipboard";
import { tdata } from "../../app/strings-data";
import type { Risk } from "../../ipc/bindings/Risk";
import { statementDisplay } from "./data-apply-model";
import type { DataModel } from "./data-controller";
import { changeCount } from "./data-edit-model";
import { BTN, BTN_PRIMARY } from "./DataToolbar";

const riskClass: Record<Risk, string> = {
  safe: "border-border text-muted",
  destructive: "border-[color:var(--srv-amber)] text-warn-text",
  data_loss: "border-danger text-danger",
};

/** Risk rozeti (metin taşır; renk tek başına bilgi değildir). */
function Risky(props: { risk: Risk }) {
  return (
    <span class={`inline-flex h-[18px] shrink-0 items-center rounded-full border px-1.5 text-[11px] ${riskClass[props.risk]}`}>
      {`${tdata.panel.risk}: ${tdata.risk[props.risk]}`}
    </span>
  );
}

export function ChangePanel(props: {
  m: DataModel;
  saveReason: string | null;
  onConfirm: (checked: boolean) => void;
  onSave: () => void;
  onCancelApply: () => void;
  onReload: () => void;
}) {
  const uid = createUniqueId();
  const [open, setOpen] = createSignal(true);
  const [copyNote, setCopyNote] = createSignal<string | null>(null);
  const count = () => {
    const l = props.m.loaded;
    return l ? changeCount(l, props.m.edit) : 0;
  };
  const view = () => props.m.view;
  const issues = () => props.m.issues;
  const issueLines = () => {
    const x = issues();
    if (!x) return [];
    const out: string[] = [...x.general];
    for (const [i, msgs] of x.row) for (const msg of msgs) out.push(tdata.panel.changeIssue(i, msg));
    for (const [k, msgs] of x.cell) {
      const sep = k.indexOf(":");
      for (const msg of msgs) out.push(tdata.panel.cellIssue(Number(k.slice(0, sep)), k.slice(sep + 1), msg));
    }
    return out;
  };
  const gate = () => props.m.gate;
  const needsConfirm = () => gate().kind === "needsConfirm";
  const confirmText = () => {
    const g = gate();
    return g.kind === "needsConfirm" && g.production ? tdata.confirm.production : tdata.confirm.destroy;
  };
  const copy = (sql: string) =>
    void copyText(sql).then(
      () => setCopyNote(tdata.panel.copied),
      () => setCopyNote(tdata.panel.copyFailed),
    );
  const outcome = () => props.m.outcome;

  return (
    <section aria-labelledby={`${uid}-t`} class="flex max-h-[45%] shrink-0 flex-col border-t border-border bg-panel text-[13px]">
      <div class="flex h-9 shrink-0 items-center gap-2 px-2">
        <button
          type="button"
          class="rounded px-1 text-muted hover:bg-hover"
          aria-expanded={open()}
          aria-controls={`${uid}-b`}
          title={open() ? tdata.panel.hide : tdata.panel.show}
          onClick={() => setOpen(!open())}
        >
          <span aria-hidden="true">{open() ? "▾" : "▸"}</span>
          <span class="sr-only">{open() ? tdata.panel.hide : tdata.panel.show}</span>
        </button>
        <h2 id={`${uid}-t`} class="font-semibold">
          {tdata.panel.title}
        </h2>
        <span class="text-muted">{tdata.status.changes(count())}</span>
        <Show when={props.m.pending}>
          <span class="text-muted">{tdata.status.previewing}</span>
        </Show>
        <div class="ml-auto flex items-center gap-1.5">
          <Show
            when={props.m.applying}
            fallback={
              <button
                type="button"
                class={BTN_PRIMARY}
                title={props.saveReason ?? tdata.toolbar.save}
                disabled={props.saveReason !== null}
                onClick={() => props.onSave()}
              >
                {tdata.toolbar.save}
              </button>
            }
          >
            <span class="text-muted">{tdata.toolbar.saving}</span>
            <button type="button" class={BTN} title={tdata.toolbar.cancelTitle} onClick={() => props.onCancelApply()}>
              {tdata.toolbar.cancel}
            </button>
          </Show>
        </div>
      </div>
      {/* Canlı bölgeler her zaman DOM'da (içerik değişince duyurulur). */}
      <div role="status" aria-live="polite" class="px-2 empty:hidden">
        <Show when={props.m.postApplyNote}>{(note) => <p class="pb-1.5 text-fg">{note()}</p>}</Show>
      </div>
      <div role="alert" class="px-2 empty:hidden">
        <Show when={outcome()}>
          {(o) => (
            <div class="mb-1.5 flex flex-wrap items-center gap-2 rounded border border-danger px-2 py-1.5 text-danger">
              <span class="min-w-0 flex-1">
                {o().text}
                <Show when={o().hint}>{(h) => <span class="ml-1 text-fg">{h()}</span>}</Show>
              </span>
              <Show when={o().kind === "conflict" || o().kind === "unknown"}>
                <button type="button" class={BTN} onClick={() => props.onReload()}>
                  {tdata.dirty.reload}
                </button>
              </Show>
            </div>
          )}
        </Show>
      </div>
      <Show when={open()}>
        <div id={`${uid}-b`} class="min-h-0 overflow-auto px-2 pb-2">
          <Show when={props.m.previewError}>
            {(err) => (
              <p role="alert" class="mb-1.5 text-danger">
                {`${tdata.panel.error}: ${err()}`}
              </p>
            )}
          </Show>
          <Show when={issueLines().length > 0}>
            <div class="mb-1.5">
              <h3 class="font-semibold text-danger">{tdata.panel.general}</h3>
              <ul class="list-disc pl-5 text-danger">
                <For each={issueLines()}>{(line) => <li>{line}</li>}</For>
              </ul>
            </div>
          </Show>
          <Show when={(view()?.warnings.length ?? 0) > 0}>
            <div class="mb-1.5">
              <h3 class="font-semibold text-warn-text">{tdata.panel.warnings}</h3>
              <ul class="list-disc pl-5 text-warn-text">
                <For each={view()?.warnings ?? []}>{(w) => <li>{w}</li>}</For>
              </ul>
            </div>
          </Show>
          <Show
            when={view() && view()!.statements.length > 0}
            fallback={
              <Show when={count() === 0}>
                <p class="text-muted">{tdata.panel.empty}</p>
              </Show>
            }
          >
            <ol class="flex flex-col gap-1.5" classList={{ "opacity-60": !props.m.viewIsCurrent }}>
              <For each={view()?.statements ?? []}>
                {(st, i) => {
                  const shown = () => statementDisplay(st.sql);
                  return (
                    <li class="rounded border border-border bg-bg">
                      <div class="flex items-center gap-2 border-b border-border px-2 py-1">
                        <span class="text-[12px] text-muted">{tdata.panel.statement(i())}</span>
                        <Risky risk={st.risk} />
                        <button
                          type="button"
                          class="ml-auto inline-flex h-6 items-center rounded border border-border px-2 text-[12px] hover:bg-hover"
                          title={tdata.panel.copyTitle}
                          onClick={() => copy(st.sql)}
                        >
                          {tdata.panel.copy}
                        </button>
                      </div>
                      <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-all px-2 py-1 font-mono text-[12px]">
                        {shown().text}
                        <Show when={shown().omitted > 0}>
                          <span class="text-muted">{tdata.preview.truncated(shown().omitted)}</span>
                        </Show>
                      </pre>
                    </li>
                  );
                }}
              </For>
            </ol>
          </Show>
          <p role="status" class="text-[12px] text-muted empty:hidden">
            {copyNote() ?? ""}
          </p>
        </div>
      </Show>
      <Show when={needsConfirm()}>
        <label class="flex items-center gap-2 border-t border-border px-2 py-1.5">
          <input
            type="checkbox"
            checked={props.m.confirm.checked}
            disabled={props.m.applying}
            onChange={(ev) => props.onConfirm(ev.currentTarget.checked)}
          />
          {confirmText()}
        </label>
      </Show>
    </section>
  );
}
