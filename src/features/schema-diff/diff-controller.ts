// Şema karşılaştırma sekmelerinin durumu (tembel parça): sekme başına Solid store, seçici listeleri, karşılaştırma işi
// (ilerleme olayları, iptal), ayrıntı (gecikmeli istek + diffId önbelleği) ve betik diyaloğu eylemleri.
//
// Kurallar:
// - Üç bağımsız kuşak vardır: seçici listeleri (taraf başına), karşılaştırma ve ayrıntı (betik diyaloğu için de ayrı bir
//   sayaç). Eski kuşağın yanıtı, olayı ve hatası durum yazmaz; kapanan sekmeye hiçbir şey yazılmaz.
// - İptal sırası (spec §6.8): arka uç işi ilk `await`'inden önce kaydeder; kayıttan önce gelen `schema_diff_cancel`
//   etkisizdir. Bu çalıştırmanın ilk olayı kaydı kanıtlar: ondan önceki iptal bekletilir ve ilk olayda (bir kez)
//   gönderilir; olay gelmeden `start` dönerse bekleyen iptal düşer.
// - Bir diffId için `schemaDiffClose` en çok bir kez çağrılır; çalışırken kapanan sekmede `start` yerleşince çağrılır.
// - Gizlilik: bu modül `console` kullanmaz; bildirimler sabit metindir (yol, ad, SQL yok).
// - İçe aktarma: `app/strings`, `ui/*`, `query/*`, `explorer/*`, inspector ve designer modülleri, CodeMirror yalnız tür olarak.
import { createSignal } from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";
import { tsd } from "../../app/strings-diff";
import type { ToastKind } from "../../app/toast-model";
import type { DiffDatabase } from "../../ipc/bindings/DiffDatabase";
import type { DiffDetail } from "../../ipc/bindings/DiffDetail";
import type { DiffEvent } from "../../ipc/bindings/DiffEvent";
import type { DiffOptions } from "../../ipc/bindings/DiffOptions";
import type { DiffResult } from "../../ipc/bindings/DiffResult";
import type { DiffScript } from "../../ipc/bindings/DiffScript";
import type { DiffSide } from "../../ipc/bindings/DiffSide";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SideTag } from "../../ipc/bindings/SideTag";
import type { Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import type { QueryController } from "../query/query-controller";
import type { TabsStore } from "../query/tabs-store";
import type { DiffApi } from "./diff-commands";
import {
  compareBlocked, defaultFilters, emptySide, entryFromTarget, errorText, foldSearch, initialSelection, scriptBlocked,
  selectedKeys, setMany, suggestScriptName, tabTitle, toggleKey, visibleItems, type Category, type SideSel,
} from "./diff-model";

export const DETAIL_DELAY_MS = 120;

export type Picker = { save(o: { defaultPath: string; filters: { name: string; extensions: string[] }[] }): Promise<string | null> };

export interface DiffControllerDeps {
  api: DiffApi & Pick<Api, "ddlTargetOf" | "ddlLookup">;
  servers: () => ServerConfig[];
  /** `deps.tree().nodes.get(nodeId)?.label ?? null` */
  nodeLabel: (nodeId: string) => string | null;
  connect: (serverId: string) => Promise<boolean>;
  tabs: Pick<TabsStore, "openSchemaDiff" | "rename">;
  openScript: QueryController["openScript"];
  copy: (text: string) => Promise<void>;
  notify: (text: string, kind: ToastKind) => void;
  /** ScriptDialog parçasıyla tembel yüklenir. */
  pick: () => Promise<Picker>;
  newId(): string;
  now(): Date;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
}

export interface SideState extends SideSel {
  databases: DiffDatabase[] | null;
  databasesError: string | null;
  schemas: string[] | null;
  schemasError: string | null;
  loadingLists: boolean;
}

export type Phase = "idle" | "running" | "done";

export interface DiffModel {
  tabId: string;
  source: SideState;
  target: SideState;
  options: DiffOptions;
  phase: Phase;
  cancelRequested: boolean;
  diffId: string | null;
  progress: { source: { loaded: number; total: number } | null; target: { loaded: number; total: number } | null };
  result: DiffResult | null;
  stale: boolean;
  /** cancelled | gone (`role="status"`). */
  banner: string | null;
  /** `role="alert"`. */
  error: string | null;
  filters: Record<Category, boolean>;
  search: string;
  selected: ReadonlySet<string>;
  focused: string | null;
  detail: { key: string; data: DiffDetail | null; error: string | null } | null;
  /** `null` = diyalog kapalı. */
  script: { data: DiffScript | null; error: string | null; busy: boolean } | null;
}

/** QueryTabs'ın, App'in ve sekme panellerinin gördüğü denetleyici yüzü. */
export interface DiffTabCtl {
  model(tabId: string): DiffModel | null;
  ids(): string[];
  /** Tepkisel sunucu listesi (seçiciler ve hedef bandı; görünümler yalnızca denetleyiciyi alır). */
  servers(): ServerConfig[];
  open(nodeId: string): Promise<void>;
  close(tabId: string): void;
  setServer(tabId: string, side: SideTag, serverId: string | null): Promise<void>;
  setDatabase(tabId: string, side: SideTag, oid: number | null): Promise<void>;
  setSchema(tabId: string, side: SideTag, schema: string | null): void;
  swap(tabId: string): void;
  setOption<K extends keyof DiffOptions>(tabId: string, key: K, value: DiffOptions[K]): void;
  compare(tabId: string): Promise<void>;
  cancel(tabId: string): void;
  setFilter(tabId: string, c: Category, on: boolean): void;
  setSearch(tabId: string, q: string): void;
  toggle(tabId: string, key: string): void;
  selectAll(tabId: string, on: boolean): void;
  selectVisible(tabId: string, on: boolean): void;
  focus(tabId: string, key: string | null): void;
  moveFocus(tabId: string, delta: number | "home" | "end"): void;
  buildScript(tabId: string): Promise<void>;
  closeScript(tabId: string): void;
  openInQueryTool(tabId: string): Promise<void>;
  copyScript(tabId: string): Promise<void>;
  saveScript(tabId: string): Promise<void>;
}

type Named = { db: string; schema: string };

/** Bir `schemaDiffStart` çağrısı. */
interface Run {
  diffId: string;
  gen: number;
  /** İlk olay geldi: arka uç işi kaydetti, iptal artık etkilidir. */
  registered: boolean;
  pendingCancel: boolean;
  cancelSent: boolean;
  settled: boolean;
  /** Sekme çalışırken kapandı: yerleşince `schemaDiffClose`. */
  closeAfter: boolean;
  /** `schemaDiffClose` gönderildi (diffId başına en çok bir kez; durum çalıştırma kaydında, küresel küme yok). */
  closeSent: boolean;
}

interface Entry {
  state: DiffModel;
  set: SetStoreFunction<DiffModel>;
  closed: boolean;
  /** Taraf başına seçici liste kuşağı (denetleyici genelinde tekil kimlik; `swap` ile yer değiştirir). */
  listGen: Record<SideTag, number>;
  compareGen: number;
  detailGen: number;
  scriptGen: number;
  run: Run | null;
  /** Son başarılı karşılaştırmanın tarafları ve adları (betik/Query Tool/dosya adı için). */
  ran: { source: DiffSide; target: DiffSide; names: { src: Named; tgt: Named } } | null;
  /** Son karşılaştırma başladıktan sonra seçici ya da seçenek değişti. */
  changed: boolean;
  cache: Map<string, DiffDetail>;
  detailTimer: unknown;
}

const emptySideState = (): SideState => ({
  ...emptySide(), databases: null, databasesError: null, schemas: null, schemasError: null, loadingLists: false,
});

const p = (e: unknown): ErrorPayload => toPayload(e);

export function createDiffController(deps: DiffControllerDeps): DiffTabCtl {
  const { api } = deps;
  const entries = new Map<string, Entry>();
  const [order, setOrder] = createSignal<string[]>([]);
  let listSeq = 0;

  const get = (tabId: string): Entry | null => {
    const e = entries.get(tabId);
    return e && !e.closed ? e : null;
  };

  const closeRun = (run: Run) => {
    if (run.closeSent) return;
    run.closeSent = true;
    api.schemaDiffClose(run.diffId).catch(() => {});
  };
  /** Ekrandaki diffId'yi kapatır: `state.diffId` yalnızca güncel çalıştırmanın kimliği olabilir. */
  const closeCurrent = (e: Entry) => {
    if (e.run !== null && e.state.diffId === e.run.diffId) closeRun(e.run);
  };
  const sendCancel = (run: Run) => {
    if (run.cancelSent) return;
    run.cancelSent = true;
    api.schemaDiffCancel(run.diffId).catch(() => {});
  };

  const clearDetailTimer = (e: Entry) => {
    if (e.detailTimer !== null) deps.clearTimer(e.detailTimer);
    e.detailTimer = null;
  };

  /** Liste kuşağının şu an hangi tarafta olduğu (swap sonrası yer değiştirmiş olabilir); eskiyse `null`. */
  const sideOf = (e: Entry, gen: number): SideTag | null => {
    if (e.closed) return null;
    if (e.listGen.source === gen) return "source";
    if (e.listGen.target === gen) return "target";
    return null;
  };
  const newListGen = (e: Entry, side: SideTag) => (e.listGen[side] = ++listSeq);

  const markChanged = (e: Entry) => {
    e.changed = true;
    if (e.state.result !== null) e.set("stale", true);
  };

  const dbName = (s: SideState, oid: number): string => s.databases?.find((d) => d.oid === oid)?.name ?? String(oid);

  /** Kayıttan düşmüş sonuç (`diff_not_found`). */
  const gone = (e: Entry) => {
    clearDetailTimer(e);
    e.cache = new Map();
    e.detailGen++;
    e.scriptGen++;
    e.set({
      result: null, diffId: null, phase: "idle", banner: tsd.gone, script: null, detail: null, focused: null,
      selected: new Set<string>(), stale: false,
    });
  };

  // ---- seçici listeleri

  const writeLists = (e: Entry, gens: number[], patch: Partial<SideState>) => {
    for (const g of gens) {
      const side = sideOf(e, g);
      if (side) e.set(side, patch);
    }
  };

  /** Bağlan → veritabanı listesi → (veritabanı seçiliyse) şema listesi; verilen kuşaklara yazar. */
  const loadLists = async (e: Entry, gens: number[], serverId: string, databaseOid: number | null) => {
    // Her adım öncesi: sekme kapandıysa ya da tarafların seçimi ilerlediyse bağlanma (parola istemi) ve liste çağrısı
    // yapılmaz.
    const live = () => gens.some((g) => sideOf(e, g) !== null);
    if (!live()) return;
    writeLists(e, gens, { loadingLists: true });
    try {
      let ok: boolean;
      try {
        ok = await deps.connect(serverId);
      } catch (err) {
        writeLists(e, gens, { databasesError: errorText(p(err)), loadingLists: false });
        return;
      }
      if (!ok) {
        writeLists(e, gens, { databasesError: tsd.errors.connectFailed, loadingLists: false });
        return;
      }
      if (!live()) return;
      try {
        writeLists(e, gens, { databases: await api.schemaDiffDatabases(serverId), databasesError: null });
      } catch (err) {
        writeLists(e, gens, { databasesError: errorText(p(err)), loadingLists: false });
        return;
      }
      if (databaseOid !== null && live()) await loadSchemas(e, gens, serverId, databaseOid);
    } finally {
      writeLists(e, gens, { loadingLists: false });
    }
  };

  const loadSchemas = async (e: Entry, gens: number[], serverId: string, databaseOid: number) => {
    try {
      const items = await api.ddlLookup(serverId, databaseOid, { kind: "schemas" });
      writeLists(e, gens, { schemas: items.map((i) => i.value), schemasError: null });
    } catch (err) {
      writeLists(e, gens, { schemasError: errorText(p(err)) });
    }
  };

  // ---- ayrıntı

  const requestDetail = (e: Entry, key: string) => {
    clearDetailTimer(e);
    const gen = ++e.detailGen;
    const cached = e.cache.get(key);
    if (cached !== undefined) {
      e.set("detail", { key, data: cached, error: null });
      return;
    }
    e.set("detail", { key, data: null, error: null });
    const diffId = e.state.diffId;
    if (diffId === null) return;
    const cache = e.cache;
    const current = () => !e.closed && e.detailGen === gen && e.state.focused === key;
    e.detailTimer = deps.setTimer(() => {
      e.detailTimer = null;
      if (!current()) return;
      api.schemaDiffDetail(diffId, key).then(
        (d) => {
          // Önbellek diffId başınadır: yeni karşılaştırma ya da düşmüş sonuç önbelleği değiştirir.
          if (e.cache === cache) cache.set(key, d);
          if (current()) e.set("detail", { key, data: d, error: null });
        },
        (err: unknown) => {
          if (!current()) return;
          const pl = p(err);
          if (pl.code === "diff_not_found") gone(e);
          else e.set("detail", { key, data: null, error: errorText(pl) });
        },
      );
    }, DETAIL_DELAY_MS);
  };

  const focusKey = (e: Entry, key: string | null) => {
    e.set("focused", key);
    if (key === null) {
      clearDetailTimer(e);
      e.detailGen++;
      e.set("detail", null);
      return;
    }
    requestDetail(e, key);
  };

  const visible = (e: Entry) => {
    const r = e.state.result;
    return r === null ? [] : visibleItems(r.items, e.state.filters, foldSearch(e.state.search));
  };

  // ---- açılış ve seçiciler

  const open = async (nodeId: string): Promise<void> => {
    const tabId = deps.tabs.openSchemaDiff({ title: tsd.title.initial });
    const [state, set] = createStore<DiffModel>({
      tabId, source: emptySideState(), target: emptySideState(),
      options: { ownership: false, tablespace: false, comments: true },
      phase: "idle", cancelRequested: false, diffId: null, progress: { source: null, target: null },
      result: null, stale: false, banner: null, error: null,
      filters: defaultFilters(), search: "", selected: new Set<string>(), focused: null, detail: null, script: null,
    });
    const e: Entry = {
      state, set, closed: false, listGen: { source: 0, target: 0 }, compareGen: 0, detailGen: 0, scriptGen: 0,
      run: null, ran: null, changed: false, cache: new Map(), detailTimer: null,
    };
    entries.set(tabId, e);
    setOrder((o) => [...o, tabId]);

    const gens = [newListGen(e, "source"), newListGen(e, "target")];
    let side: SideSel | null;
    try {
      side = entryFromTarget(await api.ddlTargetOf(nodeId), deps.nodeLabel(nodeId) ?? "");
    } catch (err) {
      if (!e.closed) e.set("error", errorText(p(err)));
      return;
    }
    if (e.closed || side === null || side.serverId === null) return;
    writeLists(e, gens.slice(0, 1), { ...side });
    writeLists(e, gens.slice(1), { serverId: side.serverId, databaseOid: side.databaseOid, schema: null });
    await loadLists(e, gens, side.serverId, side.databaseOid);
  };

  const setServer = async (tabId: string, side: SideTag, serverId: string | null): Promise<void> => {
    const e = get(tabId);
    if (!e) return;
    const gen = newListGen(e, side);
    e.set(side, {
      serverId, databaseOid: null, schema: null, databases: null, databasesError: null, schemas: null, schemasError: null,
      loadingLists: false,
    });
    markChanged(e);
    if (serverId !== null) await loadLists(e, [gen], serverId, null);
  };

  const setDatabase = async (tabId: string, side: SideTag, oid: number | null): Promise<void> => {
    const e = get(tabId);
    if (!e) return;
    const gen = newListGen(e, side);
    const serverId = e.state[side].serverId;
    e.set(side, { databaseOid: oid, schema: null, schemas: null, schemasError: null, loadingLists: false });
    markChanged(e);
    if (oid === null || serverId === null) return;
    writeLists(e, [gen], { loadingLists: true });
    await loadSchemas(e, [gen], serverId, oid);
    writeLists(e, [gen], { loadingLists: false });
  };

  const setSchema = (tabId: string, side: SideTag, schema: string | null) => {
    const e = get(tabId);
    if (!e) return;
    e.set(side, "schema", schema);
    markChanged(e);
  };

  const swap = (tabId: string) => {
    const e = get(tabId);
    if (!e) return;
    // Sığ kopyalar (store setter değeri kendisi unwrap eder; `unwrap` içe aktarılmaz: ana pakete yeni dışa aktarma).
    const s = { ...e.state.source };
    const t = { ...e.state.target };
    e.set("source", t);
    e.set("target", s);
    e.listGen = { source: e.listGen.target, target: e.listGen.source };
    markChanged(e);
  };

  const setOption = <K extends keyof DiffOptions>(tabId: string, key: K, value: DiffOptions[K]) => {
    const e = get(tabId);
    if (!e) return;
    e.set("options", key, value);
    markChanged(e);
  };

  // ---- karşılaştırma

  const compare = async (tabId: string): Promise<void> => {
    const e = get(tabId);
    if (!e) return;
    const st = e.state;
    if (compareBlocked(st.source, st.target, st.phase === "running") !== null) return;
    const source: DiffSide = { serverId: st.source.serverId!, databaseOid: st.source.databaseOid!, schema: st.source.schema! };
    const target: DiffSide = { serverId: st.target.serverId!, databaseOid: st.target.databaseOid!, schema: st.target.schema! };
    const options: DiffOptions = { ...st.options };
    const names = {
      src: { db: dbName(st.source, source.databaseOid), schema: source.schema },
      tgt: { db: dbName(st.target, target.databaseOid), schema: target.schema },
    };
    closeCurrent(e);
    const gen = ++e.compareGen;
    const run: Run = {
      diffId: deps.newId(), gen, registered: false, pendingCancel: false, cancelSent: false, settled: false, closeAfter: false,
      closeSent: false,
    };
    e.run = run;
    e.ran = null;
    e.changed = false;
    e.cache = new Map();
    e.detailGen++;
    e.scriptGen++;
    clearDetailTimer(e);
    e.set({
      phase: "running", cancelRequested: false, diffId: run.diffId, progress: { source: null, target: null }, result: null,
      stale: false, banner: null, error: null, selected: new Set<string>(), focused: null, detail: null, script: null,
    });
    const current = () => !e.closed && e.compareGen === gen;
    const onEvent = (ev: DiffEvent) => {
      if (!run.registered && !run.settled) {
        run.registered = true;
        if (run.pendingCancel) {
          run.pendingCancel = false;
          sendCancel(run);
        }
      }
      if (current()) e.set("progress", ev.side, { loaded: ev.loaded, total: ev.total });
    };
    const settle = () => {
      run.settled = true;
      run.pendingCancel = false;
      if (run.closeAfter) closeRun(run);
    };
    let r: DiffResult;
    try {
      r = await api.schemaDiffStart(run.diffId, source, target, options, onEvent);
    } catch (err) {
      settle();
      if (!current()) return;
      const pl = p(err);
      if (pl.code === "cancelled") e.set({ phase: "idle", cancelRequested: false, banner: tsd.cancelled, diffId: null });
      else e.set({ phase: "idle", cancelRequested: false, error: errorText(pl) });
      return;
    }
    settle();
    if (!current()) return;
    e.ran = { source, target, names };
    e.set({ phase: "done", cancelRequested: false, result: r, selected: initialSelection(r.items), stale: e.changed });
    deps.tabs.rename(tabId, tabTitle(names.src, names.tgt));
    focusKey(e, visible(e)[0]?.key ?? null);
  };

  const cancel = (tabId: string) => {
    const e = get(tabId);
    const run = e?.run;
    if (!e || !run || run.settled || e.state.phase !== "running") return;
    e.set("cancelRequested", true);
    if (run.registered) sendCancel(run);
    else run.pendingCancel = true;
  };

  const close = (tabId: string) => {
    const e = entries.get(tabId);
    if (!e) return;
    e.closed = true;
    clearDetailTimer(e);
    entries.delete(tabId);
    setOrder((o) => o.filter((x) => x !== tabId));
    const run = e.run;
    if (run && !run.settled) {
      if (run.registered) sendCancel(run);
      else run.pendingCancel = true;
      run.closeAfter = true;
    } else {
      closeCurrent(e);
    }
  };

  // ---- liste ve seçim

  const withResult = (tabId: string, fn: (e: Entry, r: DiffResult) => void) => {
    const e = get(tabId);
    const r = e?.state.result;
    if (e && r) fn(e, r);
  };

  // ---- betik

  const scriptState = (tabId: string) => {
    const e = get(tabId);
    const sql = e?.state.script?.data?.sql;
    return e && sql !== undefined && e.ran !== null ? { e, sql, ran: e.ran } : null;
  };

  const scriptError = (e: Entry, gen: number, err: unknown) => {
    if (e.closed || e.scriptGen !== gen) return;
    const pl = p(err);
    if (pl.code === "diff_not_found") gone(e);
    else if (e.state.script !== null) e.set("script", { error: errorText(pl), busy: false });
  };

  const buildScript = async (tabId: string): Promise<void> => {
    const e = get(tabId);
    if (!e) return;
    const st = e.state;
    const keys = st.result === null ? [] : selectedKeys(st.selected, st.result.items);
    const blocked = scriptBlocked({
      running: st.phase === "running", hasResult: st.result !== null, stale: st.stale, selected: keys.length,
    });
    if (blocked !== null || st.diffId === null) return;
    const gen = ++e.scriptGen;
    e.set("script", { data: null, error: null, busy: true });
    try {
      const data = await api.schemaDiffScript(st.diffId, keys);
      if (e.closed || e.scriptGen !== gen) return;
      e.set("script", { data, error: null, busy: false });
    } catch (err) {
      scriptError(e, gen, err);
    }
  };

  const closeScript = (tabId: string) => {
    const e = get(tabId);
    if (!e) return;
    e.scriptGen++;
    e.set("script", null);
  };

  const openInQueryTool = async (tabId: string): Promise<void> => {
    const s = scriptState(tabId);
    if (!s) return;
    const { target, names } = s.ran;
    const gen = s.e.scriptGen;
    try {
      await deps.openScript({ serverId: target.serverId, databaseOid: target.databaseOid, database: names.tgt.db, text: s.sql });
    } catch (err) {
      scriptError(s.e, gen, err);
      return;
    }
    if (!s.e.closed && s.e.scriptGen === gen) closeScript(tabId);
  };

  const copyScript = async (tabId: string): Promise<void> => {
    const s = scriptState(tabId);
    if (!s) return;
    const gen = s.e.scriptGen;
    try {
      await deps.copy(s.sql);
      deps.notify(tsd.script.copied, "success");
    } catch (err) {
      scriptError(s.e, gen, err);
    }
  };

  const saveScript = async (tabId: string): Promise<void> => {
    const s = scriptState(tabId);
    const diffId = s?.e.state.diffId;
    if (!s || !diffId) return;
    const gen = s.e.scriptGen;
    const { names } = s.ran;
    try {
      const picker = await deps.pick();
      const path = await picker.save({
        defaultPath: suggestScriptName(names.src, names.tgt, deps.now()),
        filters: [{ name: tsd.script.filterName, extensions: ["sql"] }],
      });
      if (typeof path !== "string") return;
      // Seçici açıkken sekme kapandıysa, diyalog kapandıysa ya da betik yenilendiyse hiçbir şey yapılmaz.
      if (s.e.closed || s.e.scriptGen !== gen || s.e.state.script?.data?.sql !== s.sql) return;
      await api.schemaDiffSave(diffId, path);
      deps.notify(tsd.script.saved, "success");
    } catch (err) {
      scriptError(s.e, gen, err);
    }
  };

  return {
    model: (tabId) => {
      order();
      return get(tabId)?.state ?? null;
    },
    ids: () => order(),
    servers: () => deps.servers(),
    open,
    close,
    setServer,
    setDatabase,
    setSchema,
    swap,
    setOption,
    compare,
    cancel,
    setFilter: (tabId, c, on) => get(tabId)?.set("filters", c, on),
    setSearch: (tabId, q) => get(tabId)?.set("search", q),
    toggle: (tabId, key) =>
      withResult(tabId, (e, r) => {
        const it = r.items.find((i) => i.key === key);
        if (it) e.set("selected", toggleKey(e.state.selected, it));
      }),
    selectAll: (tabId, on) => withResult(tabId, (e, r) => e.set("selected", setMany(e.state.selected, r.items, on))),
    selectVisible: (tabId, on) => withResult(tabId, (e) => e.set("selected", setMany(e.state.selected, visible(e), on))),
    focus: (tabId, key) => {
      const e = get(tabId);
      if (e) focusKey(e, key);
    },
    moveFocus: (tabId, delta) =>
      withResult(tabId, (e) => {
        const vis = visible(e);
        if (vis.length === 0) return;
        const idx = vis.findIndex((i) => i.key === e.state.focused);
        let next: number;
        if (delta === "home") next = 0;
        else if (delta === "end") next = vis.length - 1;
        else if (idx < 0) next = delta > 0 ? 0 : vis.length - 1;
        else next = Math.max(0, Math.min(vis.length - 1, idx + delta));
        if (vis[next].key !== e.state.focused) focusKey(e, vis[next].key);
      }),
    buildScript,
    closeScript,
    openInQueryTool,
    copyScript,
    saveScript,
  };
}
