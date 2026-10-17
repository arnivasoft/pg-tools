// Designer formlarının ortak parçaları (tembel parça): alan satırı (etiket + hata metni), sahip seçimi, yorum alanı,
// serbest SQL notu ve form bileşenlerinin ortak prop tipi. Formlar (Task 5–6) yalnızca bunları kullanır; SQL üretmez.
import { For, Show, createEffect, createResource, createUniqueId, type Component, type JSX } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { inputClass } from "../../ui/Field";
import type { DesignerController, ObjectDesignerModel } from "./designer-controller";
import { issuesFor, type FieldIssues } from "./designer-model";
import { normalizeEmpty } from "./forms-model";

/** Her tür formunun aldığı prop'lar. `readOnly`: sekme düzenlenemez (yükleniyor/kesik/uygulanıyor/kilit dışı nedenler). */
export interface FormProps {
  designer: DesignerController;
  tabId: string;
  /** Nesne sekmesinin modeli (yetki sekmeleri kendi formlarını kullanır). */
  model: ObjectDesignerModel;
  issues: FieldIssues;
  readOnly: boolean;
}

/** `FORMS` girdisi: form bileşeni + alan yolları (bilinmeyen yollardaki hatalar genel şeride düşer). */
export interface FormEntry {
  view: Component<FormProps>;
  paths: ReadonlySet<string>;
}

/**
 * Etiket + denetim + hata/ipucu metni. Çocuk denetime `id` ve `aria-describedby` vermek için render işlevi alır
 * (etiket `for` ile bağlanır; hata varsa `aria-invalid`).
 */
export function FieldRow(props: {
  label: string;
  /** `FieldError.path` (ör. `name`, `columns[1].key`); `issuesFor` alt yolları da toplar. */
  path: string;
  issues: FieldIssues;
  hint?: string;
  /** İstemci tarafı ön uyarı (ör. sayı biçimi); hata değildir, arka uç asıl doğrulamayı yapar. */
  warning?: string | null;
  /** `issuesFor` yerine yalnızca tam yol (satır düzeyi hata alt alan hatalarını tekrarlamasın). */
  exact?: boolean;
  children: (a: { id: string; describedBy: string | undefined; invalid: boolean }) => JSX.Element;
}) {
  const id = createUniqueId();
  const errId = `${id}-err`;
  const warnId = `${id}-warn`;
  const hintId = `${id}-hint`;
  const errors = () => (props.exact ? (props.issues.byPath.get(props.path) ?? []) : issuesFor(props.issues, props.path));
  const describedBy = () =>
    [errors().length > 0 ? errId : null, props.warning ? warnId : null, props.hint ? hintId : null]
      .filter((x) => x !== null)
      .join(" ") || undefined;
  return (
    <div class="flex flex-col gap-1">
      <label for={id} class="text-xs text-muted">
        {props.label}
      </label>
      {props.children({ id, get describedBy() { return describedBy(); }, get invalid() { return errors().length > 0; } })}
      <Show when={errors().length > 0}>
        <span id={errId} class="text-xs text-danger">
          {errors().join(" ")}
        </span>
      </Show>
      <Show when={props.warning}>
        <span id={warnId} class="text-xs text-warn-text">
          {props.warning}
        </span>
      </Show>
      <Show when={props.hint}>
        <span id={hintId} class="text-xs text-muted">
          {props.hint}
        </span>
      </Show>
    </div>
  );
}

export const invalidClass = (invalid: boolean) => (invalid ? " !border-danger" : "");

/**
 * Denetleyiciden gelen odak isteği (`requestFocus`) bu alana (`field`) ise denetimi odaklar; bekleyen istek form bağlandığında
 * ya da düzenlenebilir olduğunda bir kez uygulanır.
 */
export function createFocusOnRequest(
  props: { designer: DesignerController; tabId: string; readOnly: boolean },
  field: string,
  el: () => HTMLElement | undefined,
) {
  createEffect(() => {
    const req = props.designer.focusRequest(props.tabId);
    // Form düzenlenebilir olana (yüklendi/uygulama bitti) kadar istek bekler; uygulanınca tüketilir (yeniden bağlanma tekrar odaklamaz).
    if (!req || req.field !== field || props.readOnly) return;
    const node = el();
    if (!node) return;
    node.focus();
    props.designer.consumeFocus(props.tabId, req.seq);
  });
}

/**
 * Sahip: yazılabilir metin + rol listesi (`datalist`; yüzlerce rol olabilir, Select yerine). Boş = varsayılan (`null`).
 * Roller sekmenin sunucu/veritabanı önbelleğinden (`lookup`); hata olursa liste boş kalır, serbest yazım sürer.
 */
export function OwnerSelect(props: {
  designer: DesignerController;
  tabId: string;
  value: string | null;
  onChange: (v: string | null) => void;
  issues: FieldIssues;
  path?: string;
  disabled?: boolean;
}) {
  const listId = createUniqueId();
  const [roles] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "roles" }).catch(() => []),
  );
  return (
    <FieldRow label={tdes.form.owner} path={props.path ?? "owner"} issues={props.issues} hint={tdes.form.ownerHint}>
      {(a) => (
        <>
          <input
            id={a.id}
            class={inputClass + invalidClass(a.invalid)}
            list={listId}
            autocomplete="off"
            spellcheck={false}
            placeholder={tdes.form.ownerPlaceholder}
            aria-describedby={a.describedBy}
            aria-invalid={a.invalid}
            disabled={props.disabled}
            value={props.value ?? ""}
            onInput={(ev) => {
              // Yazıldığı gibi (boşluk içeren rol adı yazılabilsin); yalnızca boş/boşluk → varsayılan.
              props.onChange(normalizeEmpty(ev.currentTarget.value));
            }}
          />
          <datalist id={listId}>
            <For each={roles() ?? []}>{(r) => <option value={r.value} />}</For>
          </datalist>
        </>
      )}
    </FieldRow>
  );
}

/** Yorum (COMMENT): çok satırlı; boş = yorum yok (`null`). */
export function CommentField(props: {
  value: string | null;
  onChange: (v: string | null) => void;
  issues: FieldIssues;
  path?: string;
  disabled?: boolean;
}) {
  return (
    <FieldRow label={tdes.form.comment} path={props.path ?? "comment"} issues={props.issues}>
      {(a) => (
        <textarea
          id={a.id}
          rows={3}
          class={`${inputClass} !h-auto resize-y py-1${invalidClass(a.invalid)}`}
          aria-describedby={a.describedBy}
          aria-invalid={a.invalid}
          disabled={props.disabled}
          value={props.value ?? ""}
          onInput={(ev) => {
            const v = ev.currentTarget.value;
            props.onChange(v === "" ? null : v);
          }}
        />
      )}
    </FieldRow>
  );
}

/** Serbest SQL alanlarının (indeks ifadesi, `WHERE`) altındaki uyarı notu. */
export function FreeSqlNote(props: { id?: string }) {
  return (
    <p id={props.id} class="text-xs text-warn-text">
      {tdes.form.freeSql}
    </p>
  );
}

/** Onay kutusu + alan hatası/uyarı/ipucu (aria-describedby ile bağlı); hata yolu `path` (alt yollar dahil). */
export function CheckRow(props: {
  label: string;
  path: string;
  issues: FieldIssues;
  hint?: string;
  /** İstemci ön uyarısı (hata değil). */
  warning?: string | null;
  checked: boolean;
  disabled: boolean;
  onChange: (v: boolean) => void;
  /** Onay kutusu öğesi (odak isteği için; ör. Function "Yeniden oluştur"). */
  ref?: (el: HTMLInputElement) => void;
}) {
  const id = createUniqueId();
  const errors = () => issuesFor(props.issues, props.path);
  const describedBy = () =>
    [errors().length > 0 ? `${id}-err` : null, props.warning ? `${id}-warn` : null, props.hint ? `${id}-hint` : null]
      .filter(Boolean)
      .join(" ") || undefined;
  return (
    <div class="flex flex-col gap-1">
      <label class="flex items-center gap-2 text-[13px]">
        <input
          ref={(el) => props.ref?.(el)}
          type="checkbox"
          aria-describedby={describedBy()}
          aria-invalid={errors().length > 0}
          disabled={props.disabled}
          checked={props.checked}
          onChange={(ev) => props.onChange(ev.currentTarget.checked)}
        />
        {props.label}
      </label>
      <Show when={errors().length > 0}>
        <span id={`${id}-err`} class="text-xs text-danger">
          {errors().join(" ")}
        </span>
      </Show>
      <Show when={props.warning}>
        <span id={`${id}-warn`} class="text-xs text-warn-text">
          {props.warning}
        </span>
      </Show>
      <Show when={props.hint}>
        <span id={`${id}-hint`} class="text-xs text-muted">
          {props.hint}
        </span>
      </Show>
    </div>
  );
}
