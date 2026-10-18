// Gövde düzenleyicisi (Plan 23: CodeMirror 6; Plan 19 R1 textarea'nın yerine). Tembel parça — yalnızca `RoutineForm`
// içe aktarır; CodeMirror modülleri Sorgu Aracı ile paylaşılan tembel parçalardadır (ana pakete girmez).
//
// Kurallar:
// - Metin olduğu gibi tutulur: istemci gövdeyi değiştirmez/taramaz/tırnaklamaz. Satır sonları bayt bayt korunur:
//   belge LF'ye normalleştirilmiş görüntüdür (`displayText`), her kullanıcı düzenlemesi ham değere en küçük değişiklik
//   olarak uygulanır (`applyDisplayEdit`); fark yoksa `onChange` hiç çağrılmaz.
// - Dış değer değişimi (yeniden yükleme, geri al) belgeye programatik uygulanır (`programmatic` işareti; `onChange` yok,
//   geri alma geçmişine girmez). Görüntüsü belgeyle aynıysa hiçbir şey yapılmaz.
// - Klavye: `indentWithTab` (Tab iki boşluk girinti / Shift+Tab geri). Tuzaktan çıkış: Esc ardından Tab (2 sn içinde)
//   odağı taşır — CodeMirror'ın tab-odak kipi (ayrıntı ve paket gerekçesi: `codemirror-setup.bodyEditingExtensions`).
//   Salt okunurda `indentMore` `false` döner → Tab tarayıcıya kalır, odak doğrudan çıkar. `closeBrackets` yok (gövde yazarken tahmin edilemez ekleme yapılmasın). IME: CodeMirror'ın yerleşik desteği; ek
//   tuş işleyicisi yok.
// - Salt okunur: `EditorState.readOnly` (odaklanabilir, seçilebilir/kopyalanabilir); dil ve salt okunur `Compartment`
//   ile değişir — editör yeniden kurulmaz (geçmiş ve imleç korunur).
// - Gövde metni `console.*`'a/depolamaya yazılmaz.
import { Annotation, Compartment, EditorState, Transaction, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { createEffect, createUniqueId, on, onCleanup, onMount } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { bodyEditingExtensions, sharedTheme, sqlLookExtensions } from "../inspector/codemirror-setup";
import { INDENT, applyDisplayEdit, bodyLanguageMode, displayDiff, displayText } from "./sql-body-model";

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
  /** Yükseklik sınıfı (varsayılan `!h-64`). */
  class?: string;
  /** contentDOM'u verir (`focus()` için). */
  ref?: (el: HTMLElement) => void;
}

/** Dış değerden gelen (kullanıcı olmayan) belge değişikliği. */
const programmatic = Annotation.define<boolean>();

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

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: displayText(props.value),
        extensions: [
          bodyEditingExtensions(INDENT),
          lang.of(languageExt(props.language)),
          ro.of(readOnlyExt(props.readOnly)),
          attrs.of(contentAttrs()),
          bodyTheme,
          EditorView.updateListener.of((u) => {
            if (!u.docChanged || u.transactions.some((t) => t.annotation(programmatic))) return;
            const next = applyDisplayEdit(props.value, u.state.doc.toString());
            if (next !== props.value) props.onChange(next);
          }),
        ],
      }),
    });
    props.ref?.(view.contentDOM);

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

  // Dış değer → belge (görüntüsü farklıysa; en küçük değişiklik, imleç eşlenir).
  createEffect(
    on(
      () => props.value,
      (value) => {
        if (!view) return;
        const d = displayDiff(view.state.doc.toString(), displayText(value));
        if (!d) return;
        view.dispatch({ changes: d, annotations: [programmatic.of(true), Transaction.addToHistory.of(false)] });
      },
      { defer: true },
    ),
  );
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
        class={`${frameClass} ${props.class ?? "!h-64"}`}
        classList={{ "!border-danger": !!props.invalid, "opacity-80": props.readOnly }}
      />
      <p id={hintId} class="text-xs text-muted">
        {tdes.body.keyboardHint}
      </p>
    </div>
  );
}
