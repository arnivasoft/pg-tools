// Sağ panel sekmeleri (sorgu + dashboard + designer): saf işlevler + ince bir Solid store sarmalayıcısı.
import { createStore } from "solid-js/store";

/** Sabit, kapatılamaz "Nesne" sekmesinin kimliği (sorgu sekmelerinin önünde durur, listede yer almaz). */
export const OBJECT_TAB_ID = "object";

export interface QueryTab {
  id: string;
  serverId: string;
  database: string;
  title: string;
}

/** Dashboard sekmesi (durumu tembel dashboard denetleyicisinde; burada yalnızca sekme kaydı). */
export interface DashTab {
  id: string;
  serverId: string;
  /** `null`: sunucu düzeyi (bakım veritabanı). */
  databaseOid: number | null;
  title: string;
}

/** Nesne diyaloğu (designer) sekmesi (durumu tembel designer denetleyicisinde; burada yalnızca sekme kaydı). */
export interface DesignerTab {
  id: string;
  serverId: string;
  databaseOid: number | null;
  title: string;
  /**
   * Tekilleştirme anahtarı (`designerTabKey`): alter `alter:<kind>:<sunucu>:<db>:<oid>` (aynı nesne tek sekme); create
   * `create:<kind>:<sunucu>:<db>:<şema>:<tablo>:<n>` (aynı bağlamda birden çok "yeni" sekmesi serbest; `n` artar).
   */
  key: string;
  /** Üst çubukta görünen "<sunucu>/<veritabanı>" (verilmezse denetleyicideki sunucu adı). */
  where: string | null;
}

export interface TabsState {
  /** Sorgu sekmeleri, görünüm sırasıyla ("Nesne" sekmesi hariç). */
  tabs: QueryTab[];
  /** Dashboard sekmeleri; sorgu sekmelerinden sonra çizilir. */
  dashboards: DashTab[];
  /** Designer sekmeleri; dashboard'lardan sonra çizilir. */
  designers: DesignerTab[];
  /** Etkin sekme: `OBJECT_TAB_ID`, bir sorgu, dashboard ya da designer sekmesinin kimliği. */
  activeId: string;
  /** Sekme kimliği sayacı. */
  nextId: number;
  /** "<sunucu>/<db>" başına sonraki varsayılan başlık numarası (#n). */
  seq: Record<string, number>;
}

export interface OpenTabInput {
  serverId: string;
  serverName: string;
  database: string;
  /** Verilmezse `"<sunucu>/<db> #n"`. */
  title?: string;
  /** Kayıtlı taslaktan geri yüklemede kimliği korumak için. */
  id?: string;
}

export interface OpenDashInput {
  serverId: string;
  databaseOid: number | null;
  /** Ör. "Dashboard · <sunucu>/<veritabanı>" (metin dashboard parçasında üretilir; bu modül metin içe aktarmaz). */
  title: string;
}

/**
 * Designer sekmesi. Başlık ve `key` designer parçasında üretilir (`designerTabKey`; bu modül metin ve anahtar kuralı
 * taşımaz — ana paket büyümesin).
 */
export interface OpenDesignerInput {
  serverId: string;
  databaseOid: number | null;
  title: string;
  key: string;
  where?: string | null;
}

export function initialTabs(): TabsState {
  return { tabs: [], dashboards: [], designers: [], activeId: OBJECT_TAB_ID, nextId: 1, seq: {} };
}

const taken = (state: TabsState, id: string) =>
  id === OBJECT_TAB_ID ||
  state.tabs.some((x) => x.id === id) ||
  state.dashboards.some((x) => x.id === id) ||
  state.designers.some((x) => x.id === id);

/** Kimlik bir sorgu sekmesine mi ait (dashboard ya da "Nesne" değil)? */
export function isQueryTabId(state: TabsState, id: string): boolean {
  return state.tabs.some((x) => x.id === id);
}

export function isDashTabId(state: TabsState, id: string): boolean {
  return state.dashboards.some((x) => x.id === id);
}

export function isDesignerTabId(state: TabsState, id: string): boolean {
  return state.designers.some((x) => x.id === id);
}

/** Aynı sunucu + veritabanı için dashboard zaten açıksa onu etkinleştirir; değilse yenisini ekler ve etkinleştirir. */
export function openDashboard(state: TabsState, input: OpenDashInput): { state: TabsState; id: string; created: boolean } {
  const same = state.dashboards.find((d) => d.serverId === input.serverId && d.databaseOid === input.databaseOid);
  if (same) return { state: activateTab(state, same.id), id: same.id, created: false };
  let nextId = state.nextId;
  let id: string;
  do id = `d${nextId++}`;
  while (taken(state, id));
  const tab: DashTab = { id, serverId: input.serverId, databaseOid: input.databaseOid, title: input.title };
  return { id, created: true, state: { ...state, dashboards: [...state.dashboards, tab], activeId: id, nextId } };
}

/**
 * Aynı `key` zaten açıksa o sekme etkinleşir (`created = false`; alter: aynı nesne tek sekme); değilse yenisi eklenir ve
 * etkinleşir (create anahtarları `designerTabKey` ile her açılışta benzersizdir).
 */
export function openDesigner(state: TabsState, input: OpenDesignerInput): { state: TabsState; id: string; created: boolean } {
  const { key } = input;
  const same = state.designers.find((d) => d.key === key);
  if (same) return { state: activateTab(state, same.id), id: same.id, created: false };
  let nextId = state.nextId;
  let id: string;
  do id = `e${nextId++}`;
  while (taken(state, id));
  const tab: DesignerTab = {
    id, serverId: input.serverId, databaseOid: input.databaseOid, title: input.title, key, where: input.where ?? null,
  };
  return { id, created: true, state: { ...state, designers: [...state.designers, tab], activeId: id, nextId } };
}

/** Yeni sekme ekler ve etkinleştirir. */
export function openTab(state: TabsState, input: OpenTabInput): { state: TabsState; id: string } {
  const key = `${input.serverName}/${input.database}`;
  const n = state.seq[key] ?? 1;
  let nextId = state.nextId;
  let id = input.id;
  if (!id || taken(state, id)) {
    do id = `q${nextId++}`;
    while (taken(state, id));
  }
  const title = input.title?.trim() || `${key} #${n}`;
  const tab: QueryTab = { id, serverId: input.serverId, database: input.database, title };
  return {
    id,
    state: {
      ...state,
      tabs: [...state.tabs, tab],
      activeId: id,
      nextId,
      seq: { ...state.seq, [key]: n + 1 },
    },
  };
}

/**
 * Sekmeyi kapatır ("Nesne" kapatılamaz). Etkin sekme kapanırsa görünüm sırasında (sorgu sekmeleri → dashboard'lar →
 * designer'lar) sağdaki, yoksa soldaki, yoksa "Nesne" etkinleşir.
 */
export function closeTab(state: TabsState, id: string): TabsState {
  const order = [...state.tabs.map((x) => x.id), ...state.dashboards.map((x) => x.id), ...state.designers.map((x) => x.id)];
  const i = order.indexOf(id);
  if (i < 0) return state;
  const rest = order.filter((x) => x !== id);
  let activeId = state.activeId;
  if (activeId === id) activeId = rest[i] ?? rest[i - 1] ?? OBJECT_TAB_ID;
  return {
    ...state,
    tabs: state.tabs.filter((x) => x.id !== id),
    dashboards: state.dashboards.filter((x) => x.id !== id),
    designers: state.designers.filter((x) => x.id !== id),
    activeId,
  };
}

/** Sorgu, dashboard ya da designer sekmesi; boş/yalnız boşluk başlık yok sayılır. */
export function renameTab(state: TabsState, id: string, title: string): TabsState {
  const tt = title.trim();
  if (!tt) return state;
  let changed = false;
  const retitle = <T extends { id: string; title: string }>(list: T[]): T[] =>
    list.map((x) => {
      if (x.id !== id || x.title === tt) return x;
      changed = true;
      return { ...x, title: tt };
    });
  const tabs = retitle(state.tabs);
  const dashboards = retitle(state.dashboards);
  const designers = retitle(state.designers);
  return changed ? { ...state, tabs, dashboards, designers } : state;
}

export function activateTab(state: TabsState, id: string): TabsState {
  if (id === state.activeId) return state;
  if (!taken(state, id)) return state;
  return { ...state, activeId: id };
}

/** Sekmeyi `to` konumuna taşır (sorgu sekmeleri içinde; sınırlara kırpılır). */
export function moveTab(state: TabsState, id: string, to: number): TabsState {
  const from = state.tabs.findIndex((t) => t.id === id);
  if (from < 0) return state;
  const target = Math.max(0, Math.min(state.tabs.length - 1, to));
  if (target === from) return state;
  const tabs = state.tabs.slice();
  const [tab] = tabs.splice(from, 1);
  tabs.splice(target, 0, tab);
  return { ...state, tabs };
}

/** Solid store sarmalayıcısı; tüm mantık yukarıdaki saf işlevlerde. */
export function createTabsStore(initial: TabsState = initialTabs()) {
  const [state, setState] = createStore<TabsState>(initial);
  const apply = (f: (s: TabsState) => TabsState) => setState((s) => f(s));
  return {
    state,
    open(input: OpenTabInput): string {
      const r = openTab(state, input);
      setState(r.state);
      return r.id;
    },
    /** Dashboard sekmesi ekler ya da (aynı sunucu + veritabanı) var olanı etkinleştirir. */
    openDashboard(input: OpenDashInput): { id: string; created: boolean } {
      const r = openDashboard(state, input);
      setState(r.state);
      return { id: r.id, created: r.created };
    },
    /** Designer sekmesi ekler ya da (alter, aynı `key`) var olanı etkinleştirir. */
    openDesigner(input: OpenDesignerInput): { id: string; created: boolean } {
      const r = openDesigner(state, input);
      setState(r.state);
      return { id: r.id, created: r.created };
    },
    close: (id: string) => apply((s) => closeTab(s, id)),
    rename: (id: string, title: string) => apply((s) => renameTab(s, id, title)),
    activate: (id: string) => apply((s) => activateTab(s, id)),
  };
}

export type TabsStore = ReturnType<typeof createTabsStore>;
