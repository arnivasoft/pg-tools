// Nesne arama modeli (tembel parça; Plan 32). Gecikmeli arama, nesil sayacı, tür süzgeci ve oturum içi hafıza.
// Kurallar:
// - Boş (kırpılmış) sorgu aramaz; 200 karakter üstü ön yüzde de reddedilir.
// - Nesil sayacı: her aramada artar; eski yanıt/hata yok sayılır.
// - `not_connected` bir kez `connect` + yeniden arama dener; ikincisi hata metni olur.
// - Hafıza yalnız bellekte (modül Map'i); `localStorage`'a ve `console.*`'a sorgu/ad yazılmaz.
import { createSignal, type Accessor } from "solid-js";
import { ts } from "../../app/strings-search";
import type { SearchHit } from "../../ipc/bindings/SearchHit";
import type { SearchKind } from "../../ipc/bindings/SearchKind";
import type { SearchResult } from "../../ipc/bindings/SearchResult";
import { errorText, toPayload } from "../../ipc/errors";

export type SearchDeps = {
  search: (serverId: string, databaseOid: number, query: string, kinds: SearchKind[]) => Promise<SearchResult>;
  connect: (serverId: string) => Promise<boolean>;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (h: unknown) => void;
};

export type SearchState = {
  query: string;
  kinds: SearchKind[]; // boş = tümü
  status: "idle" | "searching" | "done" | "error";
  hits: SearchHit[];
  truncated: boolean;
  error: string | null;
  active: number; // seçili sonuç dizini, -1 = yok
};

export const SEARCH_DELAY_MS = 250;
export const MAX_QUERY_CHARS = 200;

/** Spec §2.1 tablo sırası. */
export const ALL_KINDS: SearchKind[] = [
  "schema", "table", "partition", "view", "matview", "function", "procedure", "triggerFunction", "sequence",
  "type", "domain", "column", "index", "constraint", "trigger", "policy", "extension",
];

const memory = new Map<string, { query: string; kinds: SearchKind[] }>();

export function createSearch(deps: SearchDeps, target: { serverId: string; databaseOid: number }) {
  const key = `${target.serverId}:${target.databaseOid}`;
  const saved = memory.get(key);
  const [state, setState] = createSignal<SearchState>({
    query: saved?.query ?? "",
    kinds: saved ? [...saved.kinds] : [],
    status: "idle",
    hits: [],
    truncated: false,
    error: null,
    active: -1,
  });
  let gen = 0;
  let timer: unknown = null;
  let disposed = false;

  const patch = (p: Partial<SearchState>) => setState((s) => ({ ...s, ...p }));
  const remember = () => {
    const s = state();
    memory.set(key, { query: s.query, kinds: [...s.kinds] });
  };
  const stopTimer = () => {
    if (timer !== null) deps.clearTimer(timer);
    timer = null;
  };

  const attempt = async (g: number, q: string, kinds: SearchKind[], retried: boolean): Promise<void> => {
    let res: SearchResult;
    try {
      res = await deps.search(target.serverId, target.databaseOid, q, kinds);
    } catch (e) {
      if (g !== gen) return;
      const p = toPayload(e);
      if (p.code === "not_connected" && !retried) {
        let ok = false;
        try {
          ok = await deps.connect(target.serverId);
        } catch {
          ok = false;
        }
        if (g !== gen) return;
        if (ok) return attempt(g, q, kinds, true);
      }
      patch({ status: "error", error: errorText(p), hits: [], truncated: false, active: -1 });
      return;
    }
    if (g !== gen) return;
    patch({
      status: "done", error: null, hits: res.hits, truncated: res.truncated, active: res.hits.length > 0 ? 0 : -1,
    });
  };

  const run = () => {
    stopTimer();
    if (disposed) return;
    const g = ++gen;
    const s = state();
    const q = s.query.trim();
    if (q === "") {
      patch({ status: "idle", hits: [], truncated: false, error: null, active: -1 });
      return;
    }
    if ([...q].length > MAX_QUERY_CHARS) {
      patch({ status: "error", hits: [], truncated: false, error: ts.tooLong, active: -1 });
      return;
    }
    patch({ status: "searching", error: null });
    void attempt(g, q, [...s.kinds], false);
  };

  const setQuery = (q: string) => {
    if (disposed) return;
    patch({ query: q });
    remember();
    stopTimer();
    // Yazım sürerken eski yanıt yeni sorguyu ezmesin: kuşak hemen artar.
    gen++;
    timer = deps.setTimer(() => {
      timer = null;
      run();
    }, SEARCH_DELAY_MS);
  };

  const searchNow = () => run();

  const setKinds = (next: SearchKind[]) => {
    patch({ kinds: next });
    remember();
    run();
  };
  const toggleKind = (k: SearchKind) => {
    if (disposed) return;
    const cur = state().kinds;
    setKinds(cur.includes(k) ? cur.filter((x) => x !== k) : ALL_KINDS.filter((x) => x === k || cur.includes(x)));
  };
  const setAllKinds = () => {
    if (disposed) return;
    setKinds([]);
  };

  const move = (delta: 1 | -1) => {
    const s = state();
    if (s.hits.length === 0) return;
    const from = s.active < 0 ? (delta === 1 ? -1 : s.hits.length) : s.active;
    patch({ active: Math.min(s.hits.length - 1, Math.max(0, from + delta)) });
  };
  const setActive = (i: number) => {
    if (i >= 0 && i < state().hits.length) patch({ active: i });
  };
  /** Yazım ya da arama sürerken null: Enter eski sonuca gitmez, hemen arar. */
  const activeHit = (): SearchHit | null => {
    const s = state();
    if (timer !== null || s.status !== "done") return null;
    return s.active >= 0 && s.active < s.hits.length ? s.hits[s.active] : null;
  };
  const dispose = () => {
    disposed = true;
    gen++;
    stopTimer();
  };

  if (state().query.trim() !== "") run();

  return { state: state as Accessor<SearchState>, setQuery, searchNow, toggleKind, setAllKinds, move, setActive, activeHit, dispose };
}
