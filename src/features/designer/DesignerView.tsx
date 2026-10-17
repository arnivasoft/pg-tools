// Designer sekmesinin gövdesi (tembel parça): üst çubuk (tür rozeti, sunucu/veritabanı, durum, Yeniden yükle), durum
// şeritleri, türe göre form yuvası ve sağda SQL önizleme + uygulama çubuğu. Formlar türe göre ayrı tembel parçalardır.
import { RefreshCw } from "lucide-solid";
import { For, Show, Suspense, createMemo, createSignal, lazy, type Component } from "solid-js";
import { Dynamic } from "solid-js/web";
import { tdes } from "../../app/strings-designer";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import { Button } from "../../ui/Button";
import { ConfirmDialog } from "../../ui/ConfirmDialogView";
import ApplyBar from "./ApplyBar";
import type { DesignerController, DesignerModel, ObjectDesignerModel } from "./designer-controller";
import { groupIssues, isAclDef, kindLabel, type DesignerVariant } from "./designer-model";
import type { AclFormProps, FormEntry } from "./FormParts";
import { FORM_PATHS } from "./forms-model";
import { aclGridPaths } from "./privileges-paths";
import SqlPreview from "./SqlPreview";

// Tür → form (tembel bileşen + bilinen alan yolları; kök yol alt yolları da kapsar: `columns` ⊃ `columns[1].key`).
// Table formu kendi parçasıdır (TableForm-*.js). View ve MV aynı parçayı paylaşır (`model.kind`'e göre); tek `lazy`
// (aynı bileşen, tek parça). Her `ObjectKind` için bir form vardır (tam `Record`).
const viewForm = lazy(() => import("./ViewForm"));
// Function ve Procedure aynı parçayı paylaşır (RoutineForm-*.js; `model.kind`'e göre).
const routineForm = lazy(() => import("./RoutineForm"));
const FORMS: Record<ObjectKind, FormEntry> = {
  schema: { view: lazy(() => import("./SchemaForm")), paths: FORM_PATHS.schema },
  extension: { view: lazy(() => import("./ExtensionForm")), paths: FORM_PATHS.extension },
  sequence: { view: lazy(() => import("./SequenceForm")), paths: FORM_PATHS.sequence },
  index: { view: lazy(() => import("./IndexForm")), paths: FORM_PATHS.index },
  table: { view: lazy(() => import("./TableForm")), paths: FORM_PATHS.table },
  view: { view: viewForm, paths: FORM_PATHS.view },
  materialized_view: { view: viewForm, paths: FORM_PATHS.materialized_view },
  database: { view: lazy(() => import("./DatabaseForm")), paths: FORM_PATHS.database },
  role: { view: lazy(() => import("./RoleForm")), paths: FORM_PATHS.role },
  function: { view: routineForm, paths: FORM_PATHS.function },
  procedure: { view: routineForm, paths: FORM_PATHS.procedure },
  trigger: { view: lazy(() => import("./TriggerForm")), paths: FORM_PATHS.trigger },
  type: { view: lazy(() => import("./TypeForm")), paths: FORM_PATHS.type },
  domain: { view: lazy(() => import("./DomainForm")), paths: FORM_PATHS.domain },
};

// Plan 22: yetki sekmelerinin formları (`variant !== "object"`; ayrı tembel parçalar, kendi prop tipleriyle). Hata yolları
// taslaktan hesaplanır (`aclIssuePaths`). Task 5 varsayılan yetkiler formunu (DefaultPrivilegesForm) ekler; o zamana dek
// yer tutucu.
const preparing = lazy(async () => ({ default: () => <p class="text-muted">{tdes.view.preparing}</p> }));
const NO_PATHS: ReadonlySet<string> = new Set();
const ACL_FORMS: Record<Exclude<DesignerVariant, "object">, Component<AclFormProps>> = {
  acl: lazy(() => import("./PrivilegesForm")),
  default_acl: preparing,
};
/** Yetki sekmesinde ızgaranın sahiplendiği yollar; geri kalan her hata üst şeride (catch-all). */
function aclIssuePaths(m: DesignerModel): ReadonlySet<string> {
  const d = m.draft;
  return d && isAclDef(d) ? aclGridPaths("grants", d.grants, d.available) : NO_PATHS;
}

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
        const variant = () => m().variant;
        const objectForm = (): FormEntry | null => (variant() === "object" ? FORMS[m().kind] : null);
        const paths = () => objectForm()?.paths ?? aclIssuePaths(m());
        // Başlık rozeti: nesne sekmesinde tür, yetki sekmelerinde "Yetkiler" / "Varsayılan yetkiler".
        const badge = () => {
          const x = variant();
          return x === "object" ? kindLabel(m().kind) : v.variant[x];
        };
        // Genel hatalar: `old.`, üst-düzey `columns` ve formun bilmediği yollar.
        const issues = createMemo(() => groupIssues(m().view?.errors ?? [], paths()));
        // Tablo formu geniş (önizleme dar); yetki sekmeleri tablo türünde de normal genişlikte.
        const wideForm = () => variant() === "object" && m().kind === "table";
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
                {badge()}
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
            <Show when={m().postApplyWarnings.length > 0}>
              {/* Uygulama sonrası arka uç uyarıları (ör. PG 16+ etkisiz REVOKE); olduğu gibi gösterilir. */}
              <div role="status" class="m-2 shrink-0 rounded border border-border px-3 py-2 text-[13px]">
                <p class="font-medium">{tdes.apply.warnings}</p>
                <ul class="list-disc pl-5 text-muted">
                  <For each={m().postApplyWarnings}>{(w) => <li>{w}</li>}</For>
                </ul>
              </div>
            </Show>
            <Show when={done()}>
              <div role="status" class="m-2 shrink-0 rounded border border-border px-3 py-2 text-[13px] text-muted">
                {v.done}
              </div>
            </Show>
            <div class="flex min-h-0 flex-1">
              <section aria-label={badge()} class="min-w-0 flex-1 overflow-auto p-3">
                <Show when={m().draft !== null}>
                  <Suspense fallback={<p class="text-muted">{tdes.loading}</p>}>
                    <Show
                      when={objectForm()}
                      fallback={
                        <Dynamic
                          component={ACL_FORMS[variant() as Exclude<DesignerVariant, "object">]}
                          designer={props.designer}
                          tabId={props.tabId}
                          model={m()}
                          issues={issues()}
                          readOnly={readOnly()}
                        />
                      }
                    >
                      {(f) => (
                        <Dynamic
                          component={f().view}
                          designer={props.designer}
                          tabId={props.tabId}
                          // Nesne formu yalnız nesne sekmesinde seçilir (`objectForm`).
                          model={m() as ObjectDesignerModel}
                          issues={issues()}
                          readOnly={readOnly()}
                        />
                      )}
                    </Show>
                  </Suspense>
                </Show>
                <Show when={m().draft === null && m().status === "loading"}>
                  <p class="text-muted">{tdes.loading}</p>
                </Show>
              </section>
              <div
                class="flex min-w-80 shrink-0 flex-col border-l border-border"
                classList={{ "w-[46%]": !wideForm(), "w-[38%]": wideForm() }}
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
