// Gövde düzenleyicisi (Plan 19 R1: Karar 1 koşulu sağlanamadı → `textarea`; tembel parça — yalnızca formlar içe aktarır).
// CodeMirror düzenleyicisi backlog'dadır: `@codemirror/commands`'ı ikinci bir tembel tüketiciden içe aktarmak QueryTab'dan
// paylaşılan bir parça böler (ana `__vite__mapDeps` 21 → 22, ana +29 B; Task 2'de ölçüldü).
//
// Kurallar:
// - Metin olduğu gibi tutulur: istemci gövdeyi değiştirmez/taramaz/tırnaklamaz. Satır sonları bayt bayt korunur:
//   textarea LF'ye normalleştirilmiş görüntüyü (`displayText`) gösterir, her düzenleme ham değere en küçük değişiklik
//   olarak uygulanır (`applyDisplayEdit`); düzenleme yoksa `onChange` hiç çağrılmaz.
// - Klavye: Tab iki boşluk girinti ekler / seçili satırları girintiler, Shift+Tab geri alır; Esc ardından Tab odaktan
//   çıkar (bayrak; araya başka tuş girerse düşer). Salt okunurda Tab hiçbir şey yapmaz → odak doğrudan çıkar (tuzak yok);
//   metin seçilebilir/kopyalanabilir (`readonly`, `disabled` değil). Geri al: tarayıcının yerel textarea geçmişi
//   (girinti `execCommand("insertText")` ile yapılır ki geçmişe girsin; yoksa `setRangeText` + `input` olayı).
// - Gövde metni `console.*`'a/depolamaya yazılmaz.
import { createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { inputClass } from "../../ui/Field";
import { applyDisplayEdit, displayText, tabEdit, type TabEdit } from "./sql-body-model";

export interface SqlBodyEditorProps {
  /** Textarea kimliği: etiket `for`, odak isteği / `getElementById` için. */
  id: string;
  value: string;
  onChange: (v: string) => void;
  readOnly: boolean;
  /** Ek açıklama kimlikleri (alan hatası, FreeSqlNote); klavye ipucu her zaman eklenir. */
  describedBy?: string;
  label: string;
  invalid?: boolean;
  /** Yükseklik sınıfı; `inputClass`'ın `h-7`'sini ezmek için `!` önekli (varsayılan `!h-64`). */
  class?: string;
  ref?: (el: HTMLTextAreaElement) => void;
}

/** Düzenlemeyi geri alma geçmişine girecek biçimde uygular. */
function applyEdit(el: HTMLTextAreaElement, e: TabEdit) {
  el.setSelectionRange(e.from, e.to);
  let done = false;
  try {
    done = typeof document.execCommand === "function" && document.execCommand("insertText", false, e.insert);
  } catch {
    done = false;
  }
  if (!done) {
    el.setRangeText(e.insert, e.from, e.to, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
  el.setSelectionRange(e.selStart, e.selEnd);
}

export function SqlBodyEditor(props: SqlBodyEditorProps) {
  const hintId = createUniqueId();
  /** Esc'e basıldı: sonraki Tab girinti yerine odağı taşır (tarayıcı varsayılanı). */
  let escaped = false;
  const describedBy = () => [props.describedBy, hintId].filter(Boolean).join(" ");

  const onKeyDown = (ev: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) => {
    if (ev.key === "Escape") {
      escaped = true;
      return;
    }
    if (ev.key !== "Tab") {
      if (ev.key !== "Shift") escaped = false;
      return;
    }
    if (escaped) {
      escaped = false;
      return;
    }
    if (props.readOnly || ev.ctrlKey || ev.altKey || ev.metaKey) return;
    ev.preventDefault();
    const el = ev.currentTarget;
    const e = tabEdit(el.value, el.selectionStart, el.selectionEnd, ev.shiftKey);
    if (e) applyEdit(el, e);
  };

  return (
    <div class="flex flex-col gap-1">
      <textarea
        ref={props.ref}
        id={props.id}
        wrap="off"
        spellcheck={false}
        autocomplete="off"
        autocapitalize="off"
        aria-label={props.label}
        aria-describedby={describedBy()}
        aria-invalid={props.invalid ? true : undefined}
        readOnly={props.readOnly}
        aria-readonly={props.readOnly}
        class={`${inputClass} ${props.class ?? "!h-64"} resize-y overflow-auto whitespace-pre py-1 font-mono leading-snug`}
        classList={{ "!border-danger": !!props.invalid, "opacity-80": props.readOnly }}
        value={displayText(props.value)}
        onKeyDown={onKeyDown}
        onBlur={() => (escaped = false)}
        onInput={(ev) => {
          const next = applyDisplayEdit(props.value, ev.currentTarget.value);
          if (next !== props.value) props.onChange(next);
        }}
      />
      <p id={hintId} class="text-xs text-muted">
        {tdes.body.keyboardHint}
      </p>
    </div>
  );
}
