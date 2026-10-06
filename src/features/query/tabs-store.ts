// Sorgu sekmeleri: saf işlevler + ince bir Solid store sarmalayıcısı.
import { createStore } from "solid-js/store";

/** Sabit, kapatılamaz "Nesne" sekmesinin kimliği (sorgu sekmelerinin önünde durur, listede yer almaz). */
export const OBJECT_TAB_ID = "object";

export interface QueryTab {
  id: string;
  serverId: string;
  database: string;
  title: string;
}

export interface TabsState {
  /** Sorgu sekmeleri, görünüm sırasıyla ("Nesne" sekmesi hariç). */
  tabs: QueryTab[];
  /** Etkin sekme: `OBJECT_TAB_ID` ya da bir sorgu sekmesinin kimliği. */
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

export function initialTabs(): TabsState {
  return { tabs: [], activeId: OBJECT_TAB_ID, nextId: 1, seq: {} };
}

/** Yeni sekme ekler ve etkinleştirir. */
export function openTab(state: TabsState, input: OpenTabInput): { state: TabsState; id: string } {
  const key = `${input.serverName}/${input.database}`;
  const n = state.seq[key] ?? 1;
  let nextId = state.nextId;
  let id = input.id;
  if (!id || id === OBJECT_TAB_ID || state.tabs.some((t) => t.id === id)) {
    do id = `q${nextId++}`;
    while (state.tabs.some((t) => t.id === id));
  }
  const title = input.title?.trim() || `${key} #${n}`;
  const tab: QueryTab = { id, serverId: input.serverId, database: input.database, title };
  return {
    id,
    state: {
      tabs: [...state.tabs, tab],
      activeId: id,
      nextId,
      seq: { ...state.seq, [key]: n + 1 },
    },
  };
}

/** Sekmeyi kapatır ("Nesne" kapatılamaz). Etkin sekme kapanırsa sağdaki, yoksa soldaki, yoksa "Nesne" etkinleşir. */
export function closeTab(state: TabsState, id: string): TabsState {
  const i = state.tabs.findIndex((t) => t.id === id);
  if (i < 0) return state;
  const tabs = state.tabs.filter((t) => t.id !== id);
  let activeId = state.activeId;
  if (activeId === id) activeId = (tabs[i] ?? tabs[i - 1])?.id ?? OBJECT_TAB_ID;
  return { ...state, tabs, activeId };
}

/** Boş/yalnız boşluk başlık yok sayılır. */
export function renameTab(state: TabsState, id: string, title: string): TabsState {
  const tt = title.trim();
  if (!tt) return state;
  let changed = false;
  const tabs = state.tabs.map((t) => {
    if (t.id !== id || t.title === tt) return t;
    changed = true;
    return { ...t, title: tt };
  });
  return changed ? { ...state, tabs } : state;
}

export function activateTab(state: TabsState, id: string): TabsState {
  if (id === state.activeId) return state;
  if (id !== OBJECT_TAB_ID && !state.tabs.some((t) => t.id === id)) return state;
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
    close: (id: string) => apply((s) => closeTab(s, id)),
    rename: (id: string, title: string) => apply((s) => renameTab(s, id, title)),
    activate: (id: string) => apply((s) => activateTab(s, id)),
  };
}

export type TabsStore = ReturnType<typeof createTabsStore>;
