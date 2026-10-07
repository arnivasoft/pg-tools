// Kolon tipi önerileri (TableForm parçası; TypeInput kullanır): `types` lookup'ı 200 ms debounce ve "son istek kazanır"
// kuralıyla. Hata → boş liste (serbest yazım sürer); `busy` istek beklerken/uçuştayken true.
//
// Paket notu: `createPreviewer` (preview.ts) BİLEREK içe aktarılmaz. preview.ts denetleyici parçasındadır (designer-open);
// bu tembel parçadan da içe aktarılınca paketleyici onu ayrı bir parçaya böler ve ana paketin dinamik içe aktarma
// bağımlılık listesi (`__vite__mapDeps`) yeni dosya adıyla büyür (ölçüldü: +32 B, tavanı aşar). Aynı kural (debounce +
// belirteçle son istek kazanır + dispose) burada küçük bir kopya olarak uygulanır; zamanlayıcı yine enjekte edilir.
import { createSignal, type Accessor } from "solid-js";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { PreviewDeps } from "./preview";
import { typeQueryOf } from "./table-model";

export const TYPE_QUERY_DELAY_MS = 200;

const browserTimers: PreviewDeps = {
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export interface TypeSuggestions {
  items: Accessor<LookupItem[]>;
  busy: Accessor<boolean>;
  /** Yazılan tip metni için sorgu (`typeQueryOf`: ilk `(`/`[` öncesi; boş → null = ilk öneriler). */
  query: (text: string) => void;
  dispose: () => void;
}

export function createTypeSuggestions(lookup: (query: string | null) => Promise<LookupItem[]>, timers: PreviewDeps = browserTimers): TypeSuggestions {
  const [items, setItems] = createSignal<LookupItem[]>([]);
  const [busy, setBusy] = createSignal(false);
  let token = 0;
  let timer: unknown = null;
  let disposed = false;
  /** Son gönderilen (bekleyen ya da yanıtlanmış) sorgu; hata olursa unutulur (yeniden denenebilsin). */
  let lastQuery: string | null | undefined;
  const cancel = () => {
    token++;
    if (timer !== null) {
      timers.clearTimer(timer);
      timer = null;
    }
  };
  const done = (mine: number, r: LookupItem[], failed = false) => {
    if (disposed || mine !== token) return;
    if (failed) lastQuery = undefined;
    setItems(r);
    setBusy(false);
  };
  return {
    items,
    busy,
    query: (text) => {
      if (disposed) return;
      const q = typeQueryOf(text);
      // Aynı sorgu (ör. `numeric(` sonrası her vuruş): yeniden sorgulanmaz, aria-busy titremez.
      if (q === lastQuery) return;
      lastQuery = q;
      cancel();
      const mine = token;
      setBusy(true);
      timer = timers.setTimer(() => {
        timer = null;
        let p: Promise<LookupItem[]>;
        try {
          p = lookup(q);
        } catch (e) {
          p = Promise.reject(e);
        }
        p.then((r) => done(mine, r), () => done(mine, [], true));
      }, TYPE_QUERY_DELAY_MS);
    },
    dispose: () => {
      disposed = true;
      lastQuery = undefined;
      cancel();
      setBusy(false);
    },
  };
}
