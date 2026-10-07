// Designer sekmesinin gövdesi (tembel parça): üst çubuk (tür rozeti, sunucu/veritabanı, durum, Yeniden yükle), durum
// şeritleri, türe göre form yuvası ve sağda SQL önizleme + uygulama çubuğu. Formlar türe göre ayrı tembel parçalardır.
import { RefreshCw } from "lucide-solid";
import { Show, Suspense, createMemo, createSignal, lazy } from "solid-js";
import { Dynamic } from "solid-js/web";
import { tdes } from "../../app/strings-designer";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import { Button } from "../../ui/Button";
import { ConfirmDialog } from "../../ui/ConfirmDialog";
import ApplyBar from "./ApplyBar";
import type { DesignerController, DesignerModel } from "./designer-controller";
import { groupIssues, kindLabel } from "./designer-model";
import type { FormEntry } from "./FormParts";
import { FORM_PATHS } from "./forms-model";
import SqlPreview from "./SqlPreview";

// Tür → form (tembel bileşen + bilinen alan yolları; kök yol alt yolları da kapsar: `columns` ⊃ `columns[1].key`).
// Table formu kendi parçasıdır (TableForm-*.js). Geçici `Partial`: view/MV/database/role formları Plan 15'te gelir;
// o zamana dek formu olmayan tür tek satır şerit gösterir (Plan 15 kaldırır).
// View ve MV aynı parçayı paylaşır (`model.kind`'e göre); tek `lazy` (aynı bileşen, tek parça).
const viewForm = lazy(() => import("./ViewForm"));
const FORMS: Partial<Record<ObjectKind, FormEntry>> = {
  schema: { view: lazy(() => import("./SchemaForm")), paths: FORM_PATHS.schema },
  extension: { view: lazy(() => import("./ExtensionForm")), paths: FORM_PATHS.extension },
  sequence: { view: lazy(() => import("./SequenceForm")), paths: FORM_PATHS.sequence },
  index: { view: lazy(() => import("./IndexForm")), paths: FORM_PATHS.index },
  table: { view: lazy(() => import("./TableForm")), paths: FORM_PATHS.table },
  view: { view: viewForm, paths: FORM_PATHS.view },
  materialized_view: { view: viewForm, paths: FORM_PATHS.materialized_view },
};

const NO_PATHS: ReadonlySet<string> = new Set();

type StatusKey = keyof typeof tdes.view.status;

function StatusBadge(props: { model: DesignerModel }) {
  const key = (): StatusKey =>
    props.model.status === "ready" && props.model.mode === "create" && props.model.outcome?.outcome === "applied"
      ? "done"
      : props.model.status;
  const danger = () => key() === "error" || key() === "disconnected" || key() === "gone";
  const live = () => key() === "ready" || key() === "done";
  return (
    <span
      class="inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full border border-border px-2 text-[12px]"
      classList={{ "text-danger": danger(), "text-muted": !danger() && !live(), "text-fg": live() }}
    >
      <span
        aria-hidden="true"
        class="h-1.5 w-1.5 rounded-full"
        classList={{ "bg-accent": live(), "bg-danger": danger(), "bg-muted": !danger() && !live() }}
      />
      <span class="sr-only">{tdes.view.statusLabel}: </span>
      {tdes.view.status[key()]}
    </span>
  );
}

export default function DesignerView(props: {
  designer: DesignerController;
  tabId: string;
  /** "<sunucu>/<veritabanı>" (sekme kaydından; yoksa sunucu adı). */
  where: string | null;
  onOpenScript: (input: { serverId: string; databaseOid: number | null; text: string }) => void;
  onClose: (tabId: string) => void;
}) {
  const model = () => props.designer.model(props.tabId);
  return (
    <Show when={model()} fallback={<p class="p-3 text-muted">{tdes.loading}</p>}>
      {(m) => {
        const v = tdes.view;
        const [askReload, setAskReload] = createSignal(false);
        const entry = () => FORMS[m().kind];
        // Genel hatalar: `old.`, üst-düzey `columns` ve formun bilmediği yollar.
        const issues = createMemo(() => groupIssues(m().view?.errors ?? [], entry()?.paths ?? NO_PATHS));
        const done = () => m().mode === "create" && m().outcome?.outcome === "applied";
        const readOnly = () => m().locked || m().status !== "ready" || m().applying || done();
        const canReload = () => !m().applying && m().status !== "gone" && m().status !== "loading";
        const reload = () => {
          if (!canReload()) return;
          if (m().dirty) setAskReload(true);
          else void props.designer.reload(props.tabId);
        };
        return (
          <div class="flex h-full min-h-0 flex-col">
            <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border bg-panel px-2">
              <span class="inline-flex h-5 shrink-0 items-center rounded border border-border px-1.5 text-[11px] font-medium">
                {kindLabel(m().kind)}
              </span>
              <span class="shrink-0 text-[12px] text-muted">{v.mode[m().mode]}</span>
              <span class="min-w-0 truncate text-[13px]" title={props.where ?? m().serverName}>
                {props.where ?? m().serverName}
              </span>
              <StatusBadge model={m()} />
              <Show when={m().locked}>
                <span class="shrink-0 text-[12px] text-muted">{tdes.bar.lockedTitle}</span>
              </Show>
              <div class="ml-auto flex items-center gap-1">
                <Button variant="ghost" disabled={!canReload()} title={v.reloadTitle} onClick={reload}>
                  <RefreshCw size={14} /> {m().status === "error" ? v.retry : v.reload}
                </Button>
              </div>
            </div>
            <Show when={m().error && m().status !== "disconnected"}>
              <div role="alert" class="m-2 shrink-0 rounded border border-danger px-3 py-2 text-[13px]">
                <p class="font-medium text-danger">{m().status === "gone" ? v.status.gone : v.loadError}</p>
                <p class="text-muted">{m().error}</p>
              </div>
            </Show>
            <Show when={m().status === "disconnected"}>
              <div role="alert" class="m-2 shrink-0 rounded border border-danger px-3 py-2 text-[13px]">
                <p class="font-medium text-danger">{v.status.disconnected}</p>
                <p class="text-muted">{m().error ?? v.disconnected}</p>
              </div>
            </Show>
            <Show when={m().needsReload && m().status === "ready"}>
              <div role="alert" class="m-2 flex shrink-0 items-center gap-3 rounded border border-danger px-3 py-2 text-[13px]">
                <p class="min-w-0 flex-1">{m().gateError ?? v.needsReload}</p>
                <Button disabled={!canReload()} onClick={reload}>
                  <RefreshCw size={14} /> {v.reload}
                </Button>
              </div>
            </Show>
            <Show when={done()}>
              <div role="status" class="m-2 shrink-0 rounded border border-border px-3 py-2 text-[13px] text-muted">
                {v.done}
              </div>
            </Show>
            <div class="flex min-h-0 flex-1">
              <section aria-label={kindLabel(m().kind)} class="min-w-0 flex-1 overflow-auto p-3">
                <Show when={entry()} fallback={<p role="status" class="text-muted">{v.formMissing}</p>}>
                  {(e) => (
                    <Show when={m().draft !== null}>
                      <Suspense fallback={<p class="text-muted">{tdes.loading}</p>}>
                        <Dynamic
                          component={e().view}
                          designer={props.designer}
                          tabId={props.tabId}
                          model={m()}
                          issues={issues()}
                          readOnly={readOnly()}
                        />
                      </Suspense>
                    </Show>
                  )}
                </Show>
                <Show when={m().draft === null && m().status === "loading"}>
                  <p class="text-muted">{tdes.loading}</p>
                </Show>
              </section>
              <div
                class="flex min-w-80 shrink-0 flex-col border-l border-border"
                classList={{ "w-[46%]": m().kind !== "table", "w-[38%]": m().kind === "table" }}
              >
                <SqlPreview model={m()} general={issues().general} />
                <ApplyBar
                  designer={props.designer}
                  model={m()}
                  onOpenScript={(text) => props.onOpenScript({ serverId: m().serverId, databaseOid: m().databaseOid, text })}
                  onClose={() => props.onClose(props.tabId)}
                />
              </div>
            </div>
            <ConfirmDialog
              open={askReload()}
              title={tdes.discard.title}
              message={tdes.discard.reloadMessage}
              confirmLabel={tdes.discard.reload}
              danger
              onCancel={() => setAskReload(false)}
              onConfirm={() => {
                setAskReload(false);
                void props.designer.reload(props.tabId, { force: true });
              }}
            />
          </div>
        );
      }}
    </Show>
  );
}
