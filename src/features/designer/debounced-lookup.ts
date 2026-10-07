// Anahtarlı, debounce'lu lookup (TableForm parçası; Kısıtlar bölümünün FK hedef tablo/kolon önerileri): 200 ms debounce,
// belirteçle "son istek kazanır", aynı anahtar yeniden sorgulanmaz, hata → boş liste (serbest yazım sürer; anahtar
// unutulur, yeniden denenebilir). Anahtar değişince eski liste hemen boşalır (başka tablonun kolonları görünmesin).
//
// Paket notu: type-suggestions.ts ile aynı nedenle `createPreviewer` (preview.ts) içe aktarılmaz (yalnızca tip).
import { createSignal, type Accessor } from "solid-js";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { PreviewDeps } from "./preview";

export const LOOKUP_DELAY_MS = 200;

const browserTimers: PreviewDeps = {
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export interface DebouncedLookup<K> {
  items: Accessor<LookupItem[]>;
  busy: Accessor<boolean>;
  /** `null` → liste boşalır, bekleyen istek iptal edilir. */
  query: (key: K | null) => void;
  dispose: () => void;
}

/** `keyOf`: anahtar eşitliği için metin (aynı metin → yeniden sorgulanmaz). */
export function createDebouncedLookup<K>(
  fetch: (key: K) => Promise<LookupItem[]>,
  keyOf: (key: K) => string,
  timers: PreviewDeps = browserTimers,
): DebouncedLookup<K> {
  const [items, setItems] = createSignal<LookupItem[]>([]);
  const [busy, setBusy] = createSignal(false);
  let token = 0;
  let timer: unknown = null;
  let disposed = false;
  let last: string | null = null;
  const cancel = () => {
    token++;
    if (timer !== null) {
      timers.clearTimer(timer);
      timer = null;
    }
  };
  const done = (mine: number, r: LookupItem[], failed = false) => {
    if (disposed || mine !== token) return;
    if (failed) last = null;
    setItems(r);
    setBusy(false);
  };
  return {
    items,
    busy,
    query: (key) => {
      if (disposed) return;
      const k = key === null ? null : keyOf(key);
      if (k !== null && k === last) return;
      cancel();
      last = k;
      setItems([]);
      if (key === null) {
        setBusy(false);
        return;
      }
      const mine = token;
      setBusy(true);
      timer = timers.setTimer(() => {
        timer = null;
        let p: Promise<LookupItem[]>;
        try {
          p = fetch(key);
        } catch (e) {
          p = Promise.reject(e);
        }
        p.then((r) => done(mine, r), () => done(mine, [], true));
      }, LOOKUP_DELAY_MS);
    },
    dispose: () => {
      disposed = true;
      last = null;
      cancel();
      setBusy(false);
    },
  };
}
