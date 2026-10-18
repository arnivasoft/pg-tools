// Gövde düzenleyicisi (Plan 23: CodeMirror 6; Plan 19 R1 textarea'nın yerine). Tembel parça — yalnızca `RoutineForm`
// içe aktarır; CodeMirror modülleri Sorgu Aracı ile paylaşılan tembel parçalardadır (ana pakete girmez).
//
// Kurallar:
// - Metin olduğu gibi tutulur: istemci gövdeyi değiştirmez/taramaz/tırnaklamaz. Satır sonları bayt bayt korunur:
//   belge LF'ye normalleştirilmiş görüntüdür (`displayText`), her kullanıcı düzenlemesi ham değere en küçük değişiklik
//   olarak uygulanır (`applyDisplayEdit`); fark yoksa `onChange` hiç çağrılmaz.
// - Dış değer değişimi (yeniden yükleme, form düzeyi geri al): karar `syncDecision`. Değerin görüntüsü belgeyle aynıysa
//   (editörün kendi düzenlemesinin yankısı ya da metni değişmeyen yeniden yükleme) hiçbir şey yapılmaz — geri alma
//   geçmişi de korunur. Metin farklıysa yeni bir `EditorState` (`resyncState`): `onChange` yok, geri alma geçmişi
//   sıfırlanır (Ctrl+Z eski düzenlemeleri yeni metne uygulayamaz), seçim ve kaydırma konumu eşlenir.
// - IME: kullanıcı düzenlemeleri (bileşim dahil) her zaman taslağa yazılır. Gerçek dış değişiklik bileşim sırasında
//   gelirse ertelenir ve bileşim bitince yeniden karar verilir: araya kullanıcı yazdıysa taslak zaten belgeyle aynıdır
//   (`noop`, kullanıcının metni kalır), yazmadıysa dış değer uygulanır.
// - Klavye: `indentWithTab` (Tab iki boşluk girinti / Shift+Tab geri). Tuzaktan çıkış: Esc ardından Tab (2 sn içinde)
//   odağı taşır — CodeMirror'ın tab-odak kipi (ayrıntı ve paket gerekçesi: `codemirror-setup.bodyEditingExtensions`).
//   Salt okunurda `indentMore` `false` döner → Tab tarayıcıya kalır, odak doğrudan çıkar. `closeBrackets` yok (gövde
//   yazarken tahmin edilemez ekleme yapılmasın). IME: CodeMirror'ın yerleşik desteği; ek tuş işleyicisi yok.
// - Salt okunur: `EditorState.readOnly` (odaklanabilir, seçilebilir/kopyalanabilir); dil ve salt okunur `Compartment`
//   ile değişir — editör yeniden kurulmaz (geçmiş ve imleç korunur).
// - Gövde metni `console.*`'a/depolamaya yazılmaz.
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { createEffect, createUniqueId, on, onCleanup, onMount, untrack } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { bodyEditingExtensions, sharedTheme, sqlLookExtensions } from "../inspector/codemirror-setup";
import { INDENT, applyDisplayEdit, bodyLanguageMode, displayText } from "./sql-body-model";
import { resyncState, syncDecision } from "./sql-body-sync";

export interface SqlBodyEditorProps {
  /** Düzenlenebilir içerik öğesinin (contentDOM) kimliği: etiket `for` ve odak isteği için. */
  id: string;
  value: string;
  onChange: (v: string) => void;
  readOnly: boolean;
  /** Gövde dili (`pg_language.lanname`); `sql`/`plpgsql` SQL vurgusu, diğerleri düz metin. */
  language?: string;
  /** Ek açıklama kimlikleri (alan hatası, FreeSqlNote); klavye ipucu her zaman eklenir. */
  describedBy?: string;
  label: string;
  invalid?: boolean;
  /** Yükseklik sınıfı (varsayılan `h-64`; çerçeve sınıfı yerel olduğundan `!` gerekmez — `resize-y` satır içi yüksekliği ezilmez). */
  class?: string;
  /** contentDOM'u verir (`focus()` için). */
  ref?: (el: HTMLElement) => void;
}

/** Çerçeve: `inputClass`'a yakın (ana paket modülünü içe aktarmamak için yerel). */
const frameClass =
  "w-full overflow-hidden rounded border border-border bg-bg font-mono resize-y focus-within:border-accent";

const bodyTheme = EditorView.theme({
  ".cm-content": { padding: "4px 0" },
  ".cm-scroller": { overflow: "auto" },
});

const languageExt = (language: string | undefined): Extension =>
  bodyLanguageMode(language ?? "") === "sql" ? sqlLookExtensions() : sharedTheme;

const readOnlyExt = (ro: boolean): Extension => (ro ? EditorState.readOnly.of(true) : []);

export function SqlBodyEditor(props: SqlBodyEditorProps) {
  const hintId = createUniqueId();
  const describedBy = () => [props.describedBy, hintId].filter(Boolean).join(" ");
  const lang = new Compartment();
  const ro = new Compartment();
  const attrs = new Compartment();
  let host!: HTMLDivElement;
  let view: EditorView | undefined;

  const contentAttrs = () =>
    EditorView.contentAttributes.of({
      id: props.id,
      "aria-label": props.label,
      "aria-describedby": describedBy(),
      // `aria-readonly`, `spellcheck`/`autocorrect`/`autocapitalize` kapalı: CodeMirror kendisi ekler.
      ...(props.invalid ? { "aria-invalid": "true" } : {}),
    });

  /** IME bileşimi sırasında gelen gerçek dış değişiklik: bileşim bitince yeniden karar verilir. */
  let pendingSync = false;
  const composing = () => !!view && (view.compositionStarted || view.composing);

  /** Dış değeri belgeye uygular (bkz. üst yorum); yeni durumda geçmiş sıfır, seçim ve kaydırma eşlenir. */
  const syncFromValue = () => {
    if (!view) return;
    const valueDisplay = displayText(props.value);
    const decision = syncDecision(view.state.doc, valueDisplay, composing());
    pendingSync = decision === "defer";
    if (decision !== "apply") return;
    const r = resyncState(view.state, valueDisplay, extensions());
    if (!r) return;
    const scroll = view.scrollSnapshot().map(r.changes);
    view.setState(r.state);
    if (scroll) view.dispatch({ effects: scroll });
  };

  /** Güncel yapılandırma (compartment'lar şimdiki prop değerleriyle); ilk kurulum ve `resyncState` için. */
  const extensions = (): Extension[] => [
    bodyEditingExtensions(INDENT),
    lang.of(languageExt(props.language)),
    ro.of(readOnlyExt(props.readOnly)),
    attrs.of(contentAttrs()),
    bodyTheme,
    EditorView.updateListener.of((u) => {
      // Kullanıcı düzenlemesi (IME bileşimi dahil) her zaman yazılır; metin asla sessizce düşürülmez.
      if (u.docChanged) {
        const next = applyDisplayEdit(props.value, u.state.doc.toString());
        if (next !== props.value) props.onChange(next);
      }
      // Bekleyen dış değişiklik: bileşim bittiyse yeniden karar (araya yazıldıysa `noop`).
      if (pendingSync && !composing()) queueMicrotask(syncFromValue);
    }),
  ];

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({ doc: displayText(props.value), extensions: untrack(extensions) }),
    });
    props.ref?.(view.contentDOM);
    // Bileşim sonu her zaman bir güncelleme üretmeyebilir: bekleyen eşitleme burada da denenir.
    const onCompositionEnd = () => {
      if (pendingSync) setTimeout(syncFromValue);
    };
    const dom = view.contentDOM;
    dom.addEventListener("compositionend", onCompositionEnd);
    onCleanup(() => dom.removeEventListener("compositionend", onCompositionEnd));
  });

  // Etiket tıklaması: `label for` yalnız form denetimlerini odaklar; contentDOM bir `div` olduğundan elle odaklanır.
  // Etiket bağlanma anında belgede olmayabilir (gizli/tembel sekme) → belge düzeyinde, `for` eşleşmesiyle yakalanır.
  const onLabelClick = (ev: MouseEvent) => {
    const label = ev.target instanceof Element ? ev.target.closest("label") : null;
    if (!label || label.htmlFor !== props.id || !view) return;
    ev.preventDefault();
    view.focus();
  };
  document.addEventListener("click", onLabelClick);
  onCleanup(() => document.removeEventListener("click", onLabelClick));

  onCleanup(() => {
    view?.destroy();
    view = undefined;
  });

  // Dış değer → belge. Kullanıcı düzenlemesinin geri dönüşünde görüntü belgeyle aynıdır → `resyncState` `null`, dokunulmaz.
  createEffect(on(() => props.value, () => untrack(syncFromValue), { defer: true }));
  createEffect(
    on(
      () => props.language,
      (l) => view?.dispatch({ effects: lang.reconfigure(languageExt(l)) }),
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => props.readOnly,
      (r) => view?.dispatch({ effects: ro.reconfigure(readOnlyExt(r)) }),
      { defer: true },
    ),
  );
  createEffect(
    on(
      () => [props.id, props.label, describedBy(), props.invalid],
      () => view?.dispatch({ effects: attrs.reconfigure(contentAttrs()) }),
      { defer: true },
    ),
  );

  return (
    <div class="flex flex-col gap-1">
      <div
        ref={host}
        class={`${frameClass} ${props.class ?? "h-64"}`}
        classList={{ "!border-danger": !!props.invalid, "opacity-80": props.readOnly }}
      />
      <p id={hintId} class="text-xs text-muted">
        {tdes.body.keyboardHint}
      </p>
    </div>
  );
}
