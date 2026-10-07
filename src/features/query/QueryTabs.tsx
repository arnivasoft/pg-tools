// Sağ panel: sabit "Nesne" sekmesi (Inspector) + sorgu + dashboard + designer + veri sekmeleri. Ana pakette kalır; çalışma
// alanı (CodeMirror, ızgara) ilk sorgu sekmesi etkinleşene kadar, dashboard/designer/veri gövdeleri ilk sekmeleri açılana kadar
// inmeyen tembel parçalardır.
import { Lock, X } from "lucide-solid";
import { For, Show, Suspense, createSignal, lazy, onCleanup, onMount, type JSX } from "solid-js";
import { t } from "../../app/strings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import type { DashController } from "../dashboard/dash-controller";
import type { DataController } from "../data/data-open";
import type { DesignerController } from "../designer/designer-controller";
import { isProductionLabel } from "../servers/server-form";
import type { QueryController } from "./query-controller";
import { OBJECT_TAB_ID, isDashTabId, isDataTabId, isDesignerTabId, isQueryTabId } from "./tabs-store";
import { ENV_CHOICES, canCommitAny, envLabel, envLockDefault, serverLineColor, type EnvChoice } from "./workspace-model";

const QueryTab = lazy(() => import("./QueryTab"));
// Dashboard sekme başlıkları ve panelleri: denetleyici yüklendiğinde (ilk dashboard açılırken) inen tembel parça.
const dashTabs = () => import("../dashboard/DashTabs");
const DashTabHeads = lazy(() => dashTabs().then((m) => ({ default: m.DashTabHeads })));
const DashPanels = lazy(() => dashTabs().then((m) => ({ default: m.DashPanels })));
// Designer sekme başlıkları ve panelleri (kapatma onayı dahil): designer denetleyicisi yüklendiğinde inen tembel parça.
const designerTabs = () => import("../designer/DesignerTabs");
const DesignerTabHeads = lazy(() => designerTabs().then((m) => ({ default: m.DesignerTabHeads })));
const DesignerPanels = lazy(() => designerTabs().then((m) => ({ default: m.DesignerPanels })));
// Veri sekme başlıkları ve panelleri (kapatma onayı dahil): veri denetleyicisi yüklendiğinde inen tembel parça.
const dataTabs = () => import("../data/DataTabs");
const DataTabHeads = lazy(() => dataTabs().then((m) => ({ default: m.DataTabHeads })));
const DataPanels = lazy(() => dataTabs().then((m) => ({ default: m.DataPanels })));

export function QueryTabs(props: {
  ctl: QueryController;
  inspector: JSX.Element;
  /** Tembel dashboard denetleyicisi (ilk dashboard açılırken yüklenir; o zamana kadar `null`). */
  dash?: () => DashController | null;
  /** Kayıp dashboard'u yeniden aç (önce sunucuyu bağlar). */
  onReopenDash?: (tabId: string) => void;
  /** Tembel designer denetleyicisi (ilk "Oluştur/Düzenle"de yüklenir; o zamana kadar `null`). */
  designer?: () => DesignerController | null;
  /** Designer betiğini yeni sorgu sekmesinde aç (çalıştırmaz). */
  onOpenScript?: (input: { serverId: string; databaseOid: number | null; text: string }) => void;
  /** Tembel veri denetleyicisi (ilk "Verileri Görüntüle/Düzenle"de yüklenir; o zamana kadar `null`). */
  data?: () => DataController | null;
}) {
  const ctl = props.ctl;
  const active = () => ctl.tabs.state.activeId;
  const isObject = () => active() === OBJECT_TAB_ID;
  const isQuery = () => isQueryTabId(ctl.tabs.state, active());
  const isDash = (id: string) => isDashTabId(ctl.tabs.state, id);
  const isDesigner = (id: string) => isDesignerTabId(ctl.tabs.state, id);

  /** Dashboard sekmesi onaysız kapanır: önce oturum (`dash_close`), sonra sekme. */
  const closeDash = (id: string) => {
    props.dash?.()?.close(id);
    ctl.tabs.close(id);
  };
  /** Designer sekmesi: önce denetleyici girdisi (önizleme/uçuştaki uygulama iptal), sonra sekme. */
  const closeDesigner = (id: string) => {
    props.designer?.()?.close(id);
    ctl.tabs.close(id);
  };
  /** Veri sekmesi: önce denetleyici girdisi (oturum kapanır, uçuştaki uygulama iptal), sonra sekme. */
  const closeData = (id: string) => {
    props.data?.()?.close(id);
    ctl.tabs.close(id);
  };
  // Kirli designer/veri sekmesi kapatılmadan önce onay ister (odak Vazgeç'te; metin tembel parçada); temiz sekme doğrudan
  // kapanır. Tür başına ayrı sinyal (ortak `{ kind, id }` sinyalinden ana pakette birkaç bayt daha küçük; Plan 17 Task 1).
  const [designerClosing, setDesignerClosing] = createSignal<string | null>(null);
  const answerDesignerClose = (discard: boolean) => {
    const id = designerClosing();
    setDesignerClosing(null);
    if (discard && id !== null && isDesigner(id)) closeDesigner(id);
  };
  const [dataClosing, setDataClosing] = createSignal<string | null>(null);
  const answerDataClose = (discard: boolean) => {
    const id = dataClosing();
    setDataClosing(null);
    if (discard && id !== null) closeData(id);
  };
  const requestClose = (id: string) => {
    if (isDash(id)) closeDash(id);
    else if (isDesigner(id)) {
      if (props.designer?.()?.isDirty(id)) setDesignerClosing(id);
      else closeDesigner(id);
    } else if (isDataTabId(id)) {
      if (props.data?.()?.isDirty(id)) setDataClosing(id);
      else closeData(id);
    } else void ctl.requestClose(id);
  };

  // WebView yeniden yüklemesi (F5, Ctrl+R, Ctrl+Shift+R) oturumları kaybettirir: her zaman engellenir.
  // Sorgu sekmesi etkinken (ağaç dışında) F5 tüm betiği çalıştırır, dashboard etkinken onu yeniler, designer etkinken
  // önizlemeyi hemen yeniler (sorgu çalışmaz), veri sekmesinde satırları yeniden yükler; ağaçta F5 düğümü yeniler.
  const onKeyCapture = (e: KeyboardEvent) => {
    const k = e.key;
    const reload = k === "F5" || ((e.ctrlKey || e.metaKey) && !e.altKey && k.toLowerCase() === "r");
    if (!reload) return;
    e.preventDefault();
    const a = active();
    if (k !== "F5" || e.ctrlKey || e.shiftKey || e.altKey || a === OBJECT_TAB_ID) return;
    const target = e.target instanceof Element ? e.target : null;
    // Yalnızca sağ paneldeyken (ya da odak hiçbir yerde değilken); ağaç, filtre kutusu, araç çubuğu: F5 çalıştırmaz.
    const here = target === null || target === document.body || target.closest("[data-query-panel]") !== null;
    if (!here || document.querySelector('[role="dialog"]')) return;
    e.stopPropagation();
    if (isDash(a)) props.dash?.()?.refreshNow(a);
    else if (isDesigner(a)) props.designer?.()?.previewNow(a);
    else if (isDataTabId(a)) props.data?.()?.reloadNow(a);
    else if (isQuery()) ctl.runActive("all");
  };

  onMount(() => {
    window.addEventListener("keydown", onKeyCapture, true);
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // Pencere API'si büyük bir modül: ana paketi büyütmemek için açılıştan sonra yüklenir.
    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) =>
        getCurrentWindow().onCloseRequested(async (event) => {
          // Açık transaction'lar için tek diyalog; sonra taslaklar yazılır ve pencere kapanır.
          let ok = true;
          try {
            ok = await ctl.beforeExit();
          } catch (err) {
            console.error("kapanış hazırlığı başarısız", err);
          }
          if (!ok) event.preventDefault();
        }),
      )
      .then((u) => (disposed ? u() : (unlisten = u)))
      .catch(() => {}); // Tauri dışı ortam (ör. düz tarayıcı önizlemesi): kapanış kancası yok.
    onCleanup(() => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("keydown", onKeyCapture, true);
    });
  });

  const onTabKey = (e: KeyboardEvent, id: string) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      ctl.activate(id);
    } else if (e.key === "Delete" && id !== OBJECT_TAB_ID) {
      e.preventDefault();
      requestClose(id);
    }
  };

  const tabClass = (selected: boolean) =>
    `group relative flex h-8 max-w-56 shrink-0 cursor-default items-center gap-1.5 border-r border-border px-2.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent ${
      selected ? "bg-bg text-fg" : "text-muted hover:bg-hover"
    }`;

  /** Sorgu ve dashboard sekme başlığı: sunucu işaretleri (renk çizgisi, üretim rozeti, kilit) + içerik + kapatma. */
  const TabHead = (p: { id: string; title: string; serverId: string | null; children: JSX.Element }) => {
    const config = () => ctl.serverConfig(p.serverId);
    const production = () => isProductionLabel(config()?.label);
    const line = () => serverLineColor(config());
    return (
      <div
        role="tab"
        tabIndex={0}
        aria-selected={active() === p.id}
        title={p.title}
        class={tabClass(active() === p.id)}
        onClick={() => ctl.activate(p.id)}
        onKeyDown={(e) => onTabKey(e, p.id)}
        onMouseDown={(e) => {
          if (e.button === 1) e.preventDefault(); // orta tık otomatik kaydırma başlatmasın
        }}
        onAuxClick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            requestClose(p.id);
          }
        }}
      >
        <Show when={line()}>
          {(color) => (
            <span aria-hidden="true" class="absolute inset-x-0 top-0" style={{ height: production() ? "3px" : "2px", background: color() }} />
          )}
        </Show>
        <Show when={production()}>
          <span class="shrink-0 rounded px-1 text-[10px] font-semibold text-white" style={{ background: line() ?? "var(--danger)" }}>
            {config()?.label}
          </span>
        </Show>
        <Show when={config()?.readOnlyLock}>
          <Lock size={11} class="shrink-0" role="img" aria-label={t.query.ctx.locked} />
        </Show>
        {p.children}
        <button
          type="button"
          tabIndex={-1}
          class="ml-0.5 shrink-0 rounded p-0.5 text-muted opacity-60 hover:bg-hover hover:text-fg hover:opacity-100"
          title={t.query.tabs.closeNamed(p.title)}
          aria-label={t.query.tabs.closeNamed(p.title)}
          onClick={(e) => {
            e.stopPropagation();
            requestClose(p.id);
          }}
        >
          <X size={12} />
        </button>
      </div>
    );
  };

  return (
    <div data-query-panel class="flex h-full min-h-0 flex-col">
      <div role="tablist" aria-label={t.query.tabs.ariaLabel} class="flex h-8 shrink-0 overflow-x-auto border-b border-border bg-panel">
        <div
          role="tab"
          tabIndex={0}
          aria-selected={isObject()}
          class={tabClass(isObject())}
          onClick={() => ctl.activate(OBJECT_TAB_ID)}
          onKeyDown={(e) => onTabKey(e, OBJECT_TAB_ID)}
        >
          {t.query.objectTab}
        </div>
        <For each={ctl.tabs.state.tabs}>
          {(tab) => {
            const m = () => ctl.model(tab.id);
            const busy = () => {
              const s = m()?.run.status;
              return s === "running" || s === "cancelling";
            };
            return (
              <TabHead id={tab.id} title={tab.title} serverId={m()?.serverId ?? null}>
                <span class="truncate" classList={{ "line-through opacity-70": !!m()?.serverGone }}>
                  {tab.title}
                </span>
                <Show when={m()?.run.tx === "inTransaction" || m()?.run.tx === "failed"}>
                  <span
                    class="h-1.5 w-1.5 shrink-0 rounded-full"
                    classList={{ "bg-accent": m()?.run.tx === "inTransaction", "bg-danger": m()?.run.tx === "failed" }}
                    title={m()?.run.tx === "failed" ? t.query.ctx.failedTx : t.query.ctx.openTx("")}
                  >
                    <span class="sr-only">{m()?.run.tx === "failed" ? t.query.tabs.txFailed : t.query.tabs.txOpen}</span>
                  </span>
                </Show>
                <Show when={busy()}>
                  <span class="shrink-0 text-muted" role="img" aria-label={t.query.ctx.running}>…</span>
                </Show>
              </TabHead>
            );
          }}
        </For>
        <Show when={props.dash?.()}>
          {(dash) => (
            <Suspense>
              <DashTabHeads dash={dash()} tabs={ctl.tabs.state.dashboards} Head={TabHead} />
            </Suspense>
          )}
        </Show>
        <Show when={props.designer?.()}>
          {(designer) => (
            <Suspense>
              <DesignerTabHeads designer={designer()} tabs={ctl.tabs.state.designers} Head={TabHead} />
            </Suspense>
          )}
        </Show>
        <Show when={props.data?.()}>
          {(data) => (
            <Suspense>
              <DataTabHeads data={data()} tabs={ctl.tabs.state.data} Head={TabHead} />
            </Suspense>
          )}
        </Show>
      </div>
      {/* Gizli panel `invisible` ile saklanır (display:none kaydırma konumunu sıfırlar). */}
      <div class="relative min-h-0 flex-1">
        <div class="absolute inset-0" classList={{ "invisible pointer-events-none": !isObject() }} aria-hidden={!isObject()}>
          {props.inspector}
        </div>
        <Show when={ctl.everQuery()}>
          <div class="absolute inset-0 bg-bg" classList={{ "invisible pointer-events-none": !isQuery() }} aria-hidden={!isQuery()}>
            <Suspense fallback={<p class="p-3 text-muted">{t.query.tabs.loading}</p>}>
              <QueryTab ctl={ctl} />
            </Suspense>
          </div>
        </Show>
        <Show when={props.dash?.()}>
          {(dash) => (
            <Suspense>
              <DashPanels dash={dash()} tabs={ctl.tabs.state.dashboards} activeId={active()} onReopen={(id) => props.onReopenDash?.(id)} />
            </Suspense>
          )}
        </Show>
        <Show when={props.designer?.()}>
          {(designer) => (
            <Suspense>
              <DesignerPanels
                designer={designer()}
                tabs={ctl.tabs.state.designers}
                activeId={active()}
                onOpenScript={(input) => props.onOpenScript?.(input)}
                onClose={requestClose}
                closing={designerClosing()}
                onCloseAnswer={answerDesignerClose}
              />
            </Suspense>
          )}
        </Show>
        <Show when={props.data?.()}>
          {(data) => (
            <Suspense>
              <DataPanels
                data={data()}
                tabs={ctl.tabs.state.data}
                activeId={active()}
                onClose={requestClose}
                closing={dataClosing()}
                onCloseAnswer={answerDataClose}
              />
            </Suspense>
          )}
        </Show>
      </div>
      <CloseTxDialog ctl={ctl} />
      <BulkTxDialog ctl={ctl} />
      <EnvDialog ctl={ctl} />
    </div>
  );
}

function CloseTxDialog(props: { ctl: QueryController }) {
  const g = props.ctl.closeGate;
  const c = t.query.close;
  let cancelEl: HTMLButtonElement | undefined;
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && g.answer("cancel")}
      title={c.title}
      width="420px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => g.answer("cancel")}>
            {t.server.cancel}
          </Button>
          <Show when={g.request()?.choices.includes("rollback")}>
            <Button class="!bg-danger !text-white" variant="primary" onClick={() => g.answer("rollback")}>
              {c.rollback}
            </Button>
          </Show>
          <Show when={g.request()?.choices.includes("commit")}>
            <Button variant="primary" onClick={() => g.answer("commit")}>
              {c.commit}
            </Button>
          </Show>
        </>
      }
    >
      <p>{g.request()?.failed ? c.failedMessage(g.request()?.title ?? "") : c.message(g.request()?.title ?? "")}</p>
      <Show when={g.request()?.busy}>
        <p class="mt-2 text-xs text-muted">{c.busyNote}</p>
      </Show>
    </Dialog>
  );
}

function BulkTxDialog(props: { ctl: QueryController }) {
  const g = props.ctl.bulkGate;
  const b = t.query.bulk;
  let cancelEl: HTMLButtonElement | undefined;
  const list = () => g.request()?.list ?? [];
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && g.answer("cancel")}
      title={b.title}
      width="460px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => g.answer("cancel")}>
            {t.server.cancel}
          </Button>
          <Button class="!bg-danger !text-white" variant="primary" onClick={() => g.answer("rollback")}>
            {b.rollbackAll}
          </Button>
          <Show when={canCommitAny(list())}>
            <Button variant="primary" onClick={() => g.answer("commit")}>
              {b.commitAll}
            </Button>
          </Show>
        </>
      }
    >
      <p class="mb-2">{g.request()?.message}</p>
      <ul class="mb-2 flex max-h-48 flex-col gap-1 overflow-auto">
        <For each={list()}>
          {(p) => (
            <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
              <span class="min-w-0 flex-1 truncate">{p.title}</span>
              <span classList={{ "text-danger": p.tx === "failed", "text-muted": p.tx !== "failed" }}>
                {b.tx[p.tx]}
                {p.busy ? ` · ${b.busy}` : ""}
              </span>
            </li>
          )}
        </For>
      </ul>
      <Show when={list().some((p) => p.tx === "failed" || p.busy)}>
        <p class="text-xs text-muted">{b.failedNote}</p>
      </Show>
    </Dialog>
  );
}

function EnvDialog(props: { ctl: QueryController }) {
  const g = props.ctl.envGate;
  const e = t.query.env;
  const [choice, setChoice] = createSignal<EnvChoice | null>(null);
  const [lock, setLock] = createSignal(false);
  const config = () => g.request()?.config ?? null;
  const pick = (c: EnvChoice) => {
    setChoice(c);
    const cfg = config();
    if (cfg) setLock(envLockDefault(cfg, c));
  };
  const close = (answer: Parameters<typeof g.answer>[0]) => {
    g.answer(answer);
    setChoice(null);
    setLock(false);
  };
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && close(null)}
      title={e.title}
      width="400px"
      footer={
        <>
          <Button class="mr-auto" onClick={() => close("skip")}>
            {e.skip}
          </Button>
          <Button
            variant="primary"
            disabled={choice() === null}
            onClick={() => {
              const c = choice();
              if (c) close({ choice: c, lock: lock() });
            }}
          >
            {e.save}
          </Button>
        </>
      }
    >
      <p class="mb-3">{e.message(config()?.name ?? "")}</p>
      <div role="radiogroup" aria-label={e.title} class="flex flex-col gap-1.5">
        <For each={ENV_CHOICES}>
          {(c) => (
            <label class="flex items-center gap-2 text-[13px]">
              <input type="radio" name="pgt-env" checked={choice() === c} onChange={() => pick(c)} />
              {envLabel(c)}
            </label>
          )}
        </For>
      </div>
      <Show when={choice() === "production"}>
        <label class="mt-3 flex items-center gap-2 text-[13px]">
          <input type="checkbox" checked={lock()} onChange={(ev) => setLock(ev.currentTarget.checked)} />
          {e.lock}
        </label>
      </Show>
    </Dialog>
  );
}
