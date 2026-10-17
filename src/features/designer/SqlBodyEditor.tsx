// Düzenlenebilir gövde düzenleyicisi (Plan 19 Karar 1; tembel parça — yalnızca formlar içe aktarır). Görünüm paylaşılan
// `inspector/codemirror-setup` parçasından (`sqlLookExtensions`); geçmiş + varsayılan kısayollar yalnızca burada.
//
// Kurallar:
// - Metin olduğu gibi tutulur: istemci gövdeyi değiştirmez/taramaz/tırnaklamaz. Satır sonu korunur: değer `\r\n`
//   içeriyorsa ayırıcı `\r\n`, aksi halde `\n` (tek başına `\r` sıradan karakter kalır); `onChange` aynı ayırıcıyla
//   birleştirilmiş metni alır.
// - Dışarıdan gelen değer değişimi (yeniden yükleme, geri alma) yalnızca farklı orta bölümü değiştirir: imleç/seçim
//   değişiklikle eşlenir (korunur) ve `onChange` çağrılmaz.
// - Klavye: Tab girinti (Shift+Tab geri), Esc ardından Tab odaktan çıkar (bayrak; araya başka tuş girerse düşer).
//   Salt okunurda Tab hiçbir şey yapmaz → odak doğrudan çıkar (tuzak yok); metin seçilebilir/kopyalanabilir.
// - Gövde metni `console.*`'a/depolamaya yazılmaz.
// - PAKET (ölçüldü, Task 2): `@codemirror/commands` ikinci tüketiciyle QueryTab'dan ayrı parçaya (`dist-*.js`, ~23 KB)
//   bölünür → ana `__vite__mapDeps` 21 → 22, ana +29 B. Bir forma bağlanmadan önce Karar 1 koşulu yeniden ölçülmelidir.
import { defaultKeymap, history, historyKeymap, indentLess, indentMore } from "@codemirror/commands";
import { Annotation, Compartment, EditorState, Prec, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { createEffect, createUniqueId, on, onCleanup, onMount } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { sqlLookExtensions } from "../inspector/codemirror-setup";
import { minimalChange, separatorOf } from "./sql-body-model";

export interface SqlBodyEditorProps {
  /** Düzenlenebilir öğenin (contenteditable) kimliği: odak isteği / `getElementById` için. */
  id: string;
  value: string;
  onChange: (v: string) => void;
  readOnly: boolean;
  /** Ek açıklama kimlikleri (alan hatası, FreeSqlNote); klavye ipucu her zaman eklenir. */
  describedBy?: string;
  label: string;
  invalid?: boolean;
  /** Dış kabın yükseklik/kenar sınıfı (varsayılan `h-64`). */
  class?: string;
}

/** Dışarıdan gelen değişiklik: `onChange` tetiklenmez. */
const external = Annotation.define<boolean>();

export function SqlBodyEditor(props: SqlBodyEditorProps) {
  const hintId = createUniqueId();
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  const ro = new Compartment();
  const attrs = new Compartment();
  /** Esc'e basıldı: sonraki Tab girinti yerine odağı taşır (tarayıcı varsayılanı). */
  let escaped = false;

  const describedBy = () => [props.describedBy, hintId].filter(Boolean).join(" ");
  const roExt = (r: boolean): Extension => [EditorState.readOnly.of(r)];
  const attrExt = (): Extension =>
    EditorView.contentAttributes.of({
      id: props.id,
      "aria-label": props.label,
      "aria-describedby": describedBy(),
      "aria-multiline": "true",
      "aria-readonly": props.readOnly ? "true" : "false",
      ...(props.invalid ? { "aria-invalid": "true" } : {}),
    });

  // Esc sonrası Tab ve salt okunurda Tab tüketilmez (tarayıcı odağı taşır); aksi halde girinti.
  const tab = (f: typeof indentMore) => (v: EditorView) => {
    if (escaped) {
      escaped = false;
      return false;
    }
    if (v.state.readOnly) return false;
    f(v);
    return true;
  };
  const tabKeys = Prec.high(keymap.of([{ key: "Tab", run: tab(indentMore), shift: tab(indentLess) }]));
  // En önce çalışır, hiçbir şeyi tüketmez: Esc bayrağı kurar; Tab/değiştirici dışındaki her tuş düşürür.
  const escTracker = Prec.highest(
    EditorView.domEventHandlers({
      keydown: (ev) => {
        if (ev.key === "Escape") escaped = true;
        else if (ev.key !== "Tab" && ev.key !== "Shift") escaped = false;
        return false;
      },
      blur: () => {
        escaped = false;
        return false;
      },
    }),
  );

  const stateFor = (doc: string, selection?: { anchor: number; head?: number }) =>
    EditorState.create({
      doc,
      selection,
      extensions: [
        EditorState.lineSeparator.of(separatorOf(doc)),
        lineNumbers(),
        history(),
        escTracker,
        tabKeys,
        keymap.of([...defaultKeymap, ...historyKeymap]),
        sqlLookExtensions(),
        ro.of(roExt(props.readOnly)),
        attrs.of(attrExt()),
        EditorView.updateListener.of((u) => {
          if (!u.docChanged || u.transactions.some((t) => t.annotation(external))) return;
          props.onChange(u.state.sliceDoc());
        }),
      ],
    });

  onMount(() => {
    view = new EditorView({ parent: host, state: stateFor(props.value) });
  });

  // Dışarıdan gelen değer: fark yoksa dokunulmaz (kendi `onChange` yankısı); ayırıcı değiştiyse durum yeniden kurulur
  // (seçim sınırlandırılarak korunur), aksi halde en küçük değişiklik gönderilir (seçim eşlenir).
  createEffect(
    on(
      () => props.value,
      (value) => {
        if (!view) return;
        const cur = view.state.sliceDoc();
        if (cur === value) return;
        const sep = separatorOf(value);
        if (sep !== view.state.lineBreak) {
          // Ayırıcı değişti (ör. CRLF'li tanım yüklendi): durum yeniden kurulur (geçmiş sıfırlanır), seçim sınırlanır.
          const sel = view.state.selection.main;
          const len = stateFor(value).doc.length;
          view.setState(stateFor(value, { anchor: Math.min(sel.anchor, len), head: Math.min(sel.head, len) }));
          return;
        }
        const ch = minimalChange(cur, value, sep);
        if (ch) view.dispatch({ changes: ch, annotations: [external.of(true)] });
      },
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => [props.readOnly, props.id, props.label, props.describedBy, props.invalid] as const,
      () => view?.dispatch({ effects: [ro.reconfigure(roExt(props.readOnly)), attrs.reconfigure(attrExt())] }),
      { defer: true },
    ),
  );
  onCleanup(() => {
    view?.destroy();
    view = undefined;
  });

  return (
    <div class="flex flex-col gap-1">
      <div
        ref={host}
        class={`${props.class ?? "h-64"} overflow-hidden rounded border focus-within:border-accent`}
        classList={{ "border-danger": !!props.invalid, "border-border": !props.invalid, "opacity-80": props.readOnly }}
      />
      <p id={hintId} class="text-xs text-muted">
        {tdes.body.keyboardHint}
      </p>
    </div>
  );
}
