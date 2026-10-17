// Type formu (tembel parça): Genel (şema, ad, sahip, yorum), Tür (create: Enum/Composite/Range radyo; alter: salt okunur
// rozet), Enum değerleri, Composite öznitelikleri, Range tanımı. İstemci SQL üretmez; kurallar `type-model`'de (Karar 6:
// yüklü enum değeri Sil/Taşı kapalı, yüklü öznitelik Taşı kapalı ve yeni öznitelik yalnız sona, range alanları alter'da
// salt okunur); diğer kurallar arka ucun alan hatasıdır. `editable = false` tanım (ya da `readOnly` sekme) tamamen salt
// okunurdur (`ReadOnlyReasons`); yükleyicinin boş composite döndürdüğü base/multirange vb. tipler boş bir düzenleyici
// olarak değil, "bu tip türü düzenlenemez" notuyla gösterilir (Karar 3, spec §8.1). Kobalte yok; metinler tembel `strings-type`.
import { Undo } from "lucide-solid";
import { For, Show, createResource, createSignal, createUniqueId, type JSX } from "solid-js";
import { ttyp } from "../../app/strings-type";
import type { CompositeAttr } from "../../ipc/bindings/CompositeAttr";
import type { EnumValue } from "../../ipc/bindings/EnumValue";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { TypeBody } from "../../ipc/bindings/TypeBody";
import type { TypeDef } from "../../ipc/bindings/TypeDef";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CommentField, FieldRow, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import { RowList } from "./RowList";
import {
  asType, attrLoaded, attrRestore, attrUpdate, compositeLoss, enumAddIndex, enumHasNew, enumMoveAllowed, enumRemoveAllowed,
  enumRename, isReadOnlyVariant, newAttr, switchVariant, typePatch, type TypeVariant, type VariantStash,
} from "./type-model";
import { TypeInput } from "./TypeInput";

function Section(props: { title: string; children: JSX.Element }) {
  const id = createUniqueId();
  return (
    <section aria-labelledby={id} class="flex flex-col gap-3">
      <h3 id={id} class="text-[13px] font-medium">
        {props.title}
      </h3>
      {props.children}
    </section>
  );
}

function ListErrors(props: { messages: string[] }) {
  return (
    <Show when={props.messages.length > 0}>
      <p role="alert" class="text-xs text-danger">
        {props.messages.join(" ")}
      </p>
    </Show>
  );
}

/** "yeni" rozeti; satırın girişine `id` + `aria-describedby` ile bağlanır (ekran okuyucu tam metni okur). */
function NewBadge(props: { text: string; sr: string; id: string }) {
  return (
    <span class="inline-flex h-5 shrink-0 items-center rounded border border-accent px-1.5 text-[11px] text-accent">
      <span aria-hidden="true">{props.text}</span>
      <span id={props.id} class="sr-only">
        {props.sr}
      </span>
    </span>
  );
}

const VARIANTS: TypeVariant[] = ["enum", "composite", "range"];
type RangeBody = Extract<TypeBody, { variant: "range" }>;
type RangeField = Exclude<keyof RangeBody, "variant" | "subtype">;

/** Odaklı satırın indeksi (satır listesi içindeki `li[data-row]`); dışarıdaysa null. */
function rowIndexOf(target: EventTarget | null): number | null {
  const li = target instanceof Element ? target.closest("li[data-row]") : null;
  const parent = li?.parentElement;
  if (!li || !parent) return null;
  const i = [...parent.children].indexOf(li);
  return i < 0 ? null : i;
}

export default function TypeForm(props: FormProps) {
  const draft = () => asType(props.model.draft);
  const original = () => asType(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez (Karar 3): her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);

  const patch = (p: Partial<TypeDef>) =>
    props.designer.edit(props.tabId, (x) => typePatch(x, p));
  const edit = (f: (d: TypeDef) => TypeDef) =>
    props.designer.edit(props.tabId, (x) => {
      const d = asType(x);
      return d ? { ...f(d), kind: "type" as const } : x;
    });
  const setValues = (values: EnumValue[]) => patch({ body: { variant: "enum", values } });
  const setAttrs = (attributes: CompositeAttr[]) => patch({ body: { variant: "composite", attributes } });
  const setRange = (p: Partial<RangeBody>) =>
    edit((d) => (d.body.variant === "range" ? { ...d, body: { ...d.body, ...p } } : d));

  // Create'te tür değişirken bırakılan gövde yedeklenir (gidiş-dönüşte değerler kaybolmasın).
  const stash: VariantStash = {};
  // Enum ekleme konumu: imleçteki satırın altı (odak satır listesinde nerede kaldıysa).
  const [cursor, setCursor] = createSignal<number | null>(null);

  const kindName = createUniqueId();
  const kindHintId = createUniqueId();
  const schemaList = createUniqueId();
  const collationList = createUniqueId();
  const valuesHintId = createUniqueId();
  const attrsHintId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  // Collation listesi ilk odakta bir kez istenir (yüzlerce öğe olabilir); hata olursa serbest yazım sürer.
  const [collations, setCollations] = createSignal<LookupItem[]>([]);
  let collationsAsked = false;
  const askCollations = () => {
    if (collationsAsked) return;
    collationsAsked = true;
    props.designer.lookup(props.tabId, { kind: "collations" }).then(setCollations, () => (collationsAsked = false));
  };

  return (
    <Show when={draft()}>
      {(d) => {
        const variant = () => d().body.variant;
        const readOnlyKind = () => isReadOnlyVariant(d());
        const values = () => (d().body.variant === "enum" ? (d().body as { values: EnumValue[] }).values : []);
        const attributes = () => (d().body.variant === "composite" ? (d().body as { attributes: CompositeAttr[] }).attributes : []);
        const rangeBody = () => (d().body.variant === "range" ? (d().body as RangeBody) : null);
        const loss = () => (alter() ? compositeLoss(original(), d()) : { dropped: [], retyped: [] });
        const anyLoadedValue = () => values().some((v) => v.original !== null);
        // `object` ve `body` (tür değişimi) hataları formun üstünde; gövde gösterilmiyorsa tüm `body.*` hataları da.
        const topErrors = () => [
          ...issuesFor(props.issues, "object"),
          ...(readOnlyKind() ? issuesFor(props.issues, "body") : exactIssues(props.issues, "body")),
        ];
        const rangeLocked = () => locked() || alter();
        const rangeInput = (field: RangeField, label: string, hint?: string, collation = false) => (
          <FieldRow label={label} path={`body.${field}`} issues={props.issues} hint={hint}>
            {(f) => (
              <input
                id={f.id}
                class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                list={collation ? collationList : undefined}
                autocomplete="off"
                spellcheck={false}
                placeholder={ttyp.range.optional}
                aria-describedby={f.describedBy}
                aria-invalid={f.invalid}
                disabled={rangeLocked()}
                value={rangeBody()?.[field] ?? ""}
                onFocus={collation ? askCollations : undefined}
                onInput={(ev) => setRange({ [field]: normalizeEmpty(ev.currentTarget.value) })}
              />
            )}
          </FieldRow>
        );
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />
            <datalist id={collationList}>
              <For each={collations()}>{(it) => <option value={it.value} />}</For>
            </datalist>

            <Section title={ttyp.sections.general}>
              <FieldRow
                label={ttyp.general.schema}
                path="schema"
                issues={props.issues}
                hint={alter() ? ttyp.general.schemaAlterHint : ttyp.general.schemaCreateHint}
              >
                {(a) => (
                  <>
                    <input
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      list={schemaList}
                      autocomplete="off"
                      spellcheck={false}
                      required
                      aria-required="true"
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={locked()}
                      value={d().schema}
                      onInput={(ev) => patch({ schema: ev.currentTarget.value })}
                    />
                    <datalist id={schemaList}>
                      <For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>
                    </datalist>
                  </>
                )}
              </FieldRow>
              <FieldRow label={alter() ? ttyp.general.rename : ttyp.general.name} path="name" issues={props.issues}>
                {(a) => (
                  <input
                    id={a.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(a.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={ttyp.general.namePlaceholder}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={locked()}
                    value={d().name}
                    onInput={(ev) => patch({ name: ev.currentTarget.value })}
                  />
                )}
              </FieldRow>
              <OwnerSelect
                designer={props.designer}
                tabId={props.tabId}
                value={d().owner}
                issues={props.issues}
                disabled={locked()}
                onChange={(v) => patch({ owner: v })}
              />
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={locked()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
            </Section>

            <Section title={ttyp.sections.kind}>
              <Show
                when={!alter() && !readOnlyKind()}
                fallback={
                  <div class="flex flex-col gap-1">
                    <span class="text-xs text-muted">{ttyp.kind.label}</span>
                    <span class="inline-flex h-6 w-fit items-center rounded border border-border px-2 text-[12px] font-medium">
                      {readOnlyKind() ? ttyp.kind.other : ttyp.kind[variant()]}
                    </span>
                    <span class="text-xs text-muted">{readOnlyKind() ? ttyp.readOnlyKind : ttyp.kind.alterHint}</span>
                  </div>
                }
              >
                <fieldset class="flex flex-col gap-1" aria-describedby={kindHintId}>
                  <legend class="text-xs text-muted">{ttyp.kind.label}</legend>
                  <div class="flex gap-4">
                    <For each={VARIANTS}>
                      {(v) => (
                        <label class="flex items-center gap-2 text-[13px]">
                          <input
                            type="radio"
                            name={kindName}
                            disabled={locked()}
                            checked={variant() === v}
                            onChange={(ev) => {
                              if (ev.currentTarget.checked) edit((x) => switchVariant(x, v, null, stash));
                            }}
                          />
                          {ttyp.kind[v]}
                        </label>
                      )}
                    </For>
                  </div>
                  <span id={kindHintId} class="text-xs text-muted">
                    {ttyp.kind.createHint}
                  </span>
                </fieldset>
              </Show>
            </Section>

            <Show when={!readOnlyKind() && variant() === "enum"}>
              <div class="flex flex-col gap-2" onFocusIn={(ev) => {
                  // Ekle düğmesine geçiş imleci silmez (satır dışı odak son satırı korur).
                  const at = rowIndexOf(ev.target);
                  if (at !== null) setCursor(at);
                }}>
                <ListErrors
                  messages={[...exactIssues(props.issues, "body.values"), ...orphanRowIssues(props.issues, "body.values", values().length)]}
                />
                <RowList
                  label={ttyp.sections.values}
                  items={values()}
                  onChange={setValues}
                  name={(v) => v.label}
                  newItem={() => ({ label: "", original: null })}
                  addIndex={(list) => enumAddIndex(list.length, cursor())}
                  addLabel={ttyp.values.add}
                  emptyText={ttyp.values.empty}
                  readOnly={locked()}
                  removeLocked={(i) => !enumRemoveAllowed(d(), i)}
                  moveAllowed={(i, dir) => enumMoveAllowed(d(), i, dir)}
                >
                  {(v, i) => {
                    const badgeId = createUniqueId();
                    const isNew = () => alter() && v().original === null;
                    return (
                    <div class="flex items-end gap-2">
                      <div class="min-w-0 flex-1">
                        <FieldRow
                          label={ttyp.values.label}
                          path={`body.values[${i}]`}
                          issues={props.issues}
                          hint={v().original !== null && v().original !== v().label ? ttyp.values.renamedFrom(v().original!) : undefined}
                        >
                          {(f) => (
                            <input
                              id={f.id}
                              class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                              autocomplete="off"
                              spellcheck={false}
                              required
                              aria-required="true"
                              placeholder={ttyp.values.placeholder}
                              aria-describedby={[f.describedBy, valuesHintId, isNew() ? badgeId : null].filter(Boolean).join(" ")}
                              aria-invalid={f.invalid}
                              disabled={locked()}
                              value={v().label}
                              onInput={(ev) => {
                                const label = ev.currentTarget.value;
                                edit((x) => enumRename(x, i, label));
                              }}
                            />
                          )}
                        </FieldRow>
                      </div>
                      <Show when={isNew()}>
                        <div class="pb-1.5">
                          <NewBadge text={ttyp.values.newBadge} sr={ttyp.values.newBadgeSr} id={badgeId} />
                        </div>
                      </Show>
                    </div>
                    );
                  }}
                </RowList>
                <div id={valuesHintId} class="flex flex-col gap-1">
                  <Show when={!locked()}>
                    <p class="text-xs text-muted">{ttyp.values.addHint}</p>
                  </Show>
                  <Show when={anyLoadedValue()}>
                    <p class="text-xs text-muted">{ttyp.values.loadedHint}</p>
                  </Show>
                </div>
                <Show when={alter() && enumHasNew(d())}>
                  <p role="note" class="text-xs text-warn-text">
                    {ttyp.values.pre12Note}
                  </p>
                </Show>
              </div>
            </Show>

            <Show when={!readOnlyKind() && variant() === "composite"}>
              <div class="flex flex-col gap-2">
                <ListErrors
                  messages={[
                    ...exactIssues(props.issues, "body.attributes"),
                    ...orphanRowIssues(props.issues, "body.attributes", attributes().length),
                  ]}
                />
                <RowList
                  label={ttyp.sections.attributes}
                  items={attributes()}
                  onChange={setAttrs}
                  name={(a) => a.name}
                  newItem={newAttr}
                  addLabel={ttyp.attributes.add}
                  emptyText={ttyp.attributes.empty}
                  readOnly={locked()}
                  moveLocked={(i) => attrLoaded(d(), i)}
                >
                  {(a, i) => {
                    const retyped = () => a().original !== null && loss().retyped.includes(a().name);
                    const badgeId = createUniqueId();
                    const isNew = () => alter() && a().original === null;
                    return (
                      <div class="flex flex-col gap-1">
                        <div class="grid grid-cols-[1fr_1fr_10rem] items-start gap-2">
                          <FieldRow
                            label={ttyp.attributes.name}
                            path={`body.attributes[${i}].name`}
                            issues={props.issues}
                            exact
                            hint={a().original !== null && a().original !== a().name ? ttyp.attributes.renamedFrom(a().original!) : undefined}
                          >
                            {(f) => (
                              <input
                                id={f.id}
                                class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                                autocomplete="off"
                                spellcheck={false}
                                required
                                aria-required="true"
                                placeholder={ttyp.attributes.namePlaceholder}
                                aria-describedby={[f.describedBy, isNew() ? badgeId : null].filter(Boolean).join(" ") || undefined}
                                aria-invalid={f.invalid}
                                disabled={locked()}
                                value={a().name}
                                onInput={(ev) => {
                                  const name = ev.currentTarget.value;
                                  edit((x) => attrUpdate(x, i, { name }));
                                }}
                              />
                            )}
                          </FieldRow>
                          <FieldRow
                            label={ttyp.attributes.type}
                            path={`body.attributes[${i}].typeName`}
                            issues={props.issues}
                            exact
                            warning={retyped() ? ttyp.attributes.retypeWarning : null}
                          >
                            {(f) => (
                              <TypeInput
                                designer={props.designer}
                                tabId={props.tabId}
                                id={f.id}
                                value={a().typeName}
                                describedBy={[f.describedBy, a().original !== null ? attrsHintId : null].filter(Boolean).join(" ") || undefined}
                                invalid={f.invalid}
                                disabled={locked()}
                                onInput={(typeName) => edit((x) => attrUpdate(x, i, { typeName }))}
                              />
                            )}
                          </FieldRow>
                          <FieldRow label={ttyp.attributes.collation} path={`body.attributes[${i}].collation`} issues={props.issues} exact>
                            {(f) => (
                              <input
                                id={f.id}
                                class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                                list={collationList}
                                autocomplete="off"
                                spellcheck={false}
                                placeholder={ttyp.attributes.collationPlaceholder}
                                aria-describedby={f.describedBy}
                                aria-invalid={f.invalid}
                                disabled={locked()}
                                value={a().collation ?? ""}
                                onFocus={askCollations}
                                onInput={(ev) => {
                                  const collation = normalizeEmpty(ev.currentTarget.value);
                                  edit((x) => attrUpdate(x, i, { collation }));
                                }}
                              />
                            )}
                          </FieldRow>
                        </div>
                        <div class="flex items-center gap-2">
                          <Show when={isNew()}>
                            <NewBadge text={ttyp.attributes.newBadge} sr={ttyp.attributes.newBadgeSr} id={badgeId} />
                          </Show>
                          <ListErrors messages={exactIssues(props.issues, `body.attributes[${i}].original`)} />
                        </div>
                      </div>
                    );
                  }}
                </RowList>
                <div id={attrsHintId} class="flex flex-col gap-1">
                  <Show when={alter()}>
                    <p class="text-xs text-muted">{ttyp.attributes.orderHint}</p>
                    <p class="text-xs text-muted">{ttyp.attributes.usedHint}</p>
                  </Show>
                </div>
                <Show when={loss().dropped.length > 0 || loss().retyped.length > 0}>
                  <div role="note" class="flex flex-col gap-1 rounded border border-border px-3 py-2 text-xs">
                    <p class="font-medium text-warn-text">{ttyp.attributes.lossTitle}</p>
                    <ul class="flex flex-col gap-1">
                      <For each={loss().dropped}>
                        {(n) => (
                          <li class="flex items-center gap-2">
                            <span class="min-w-0 flex-1 text-warn-text">{ttyp.attributes.dropped(n)}</span>
                            <Show when={!locked()}>
                              <Button
                                variant="ghost"
                                aria-label={ttyp.attributes.restore(n)}
                                title={ttyp.attributes.restore(n)}
                                onClick={() => edit((x) => attrRestore(x, original(), n))}
                              >
                                <Undo size={14} aria-hidden="true" /> {ttyp.attributes.restoreShort}
                              </Button>
                            </Show>
                          </li>
                        )}
                      </For>
                      <For each={loss().retyped}>{(n) => <li class="text-warn-text">{ttyp.attributes.retyped(n)}</li>}</For>
                    </ul>
                    <p class="text-muted">{ttyp.attributes.usedHint}</p>
                  </div>
                </Show>
              </div>
            </Show>

            <Show when={!readOnlyKind() && rangeBody()}>
              <Section title={ttyp.sections.range}>
                <Show when={alter()}>
                  <p class="text-xs text-muted">{ttyp.range.alterHint}</p>
                </Show>
                <FieldRow label={ttyp.range.subtype} path="body.subtype" issues={props.issues}>
                  {(f) => (
                    <TypeInput
                      designer={props.designer}
                      tabId={props.tabId}
                      id={f.id}
                      value={rangeBody()?.subtype ?? ""}
                      describedBy={f.describedBy}
                      invalid={f.invalid}
                      disabled={rangeLocked()}
                      onInput={(subtype) => setRange({ subtype })}
                    />
                  )}
                </FieldRow>
                <div class="grid grid-cols-2 gap-2">
                  {rangeInput("subtypeOpclass", ttyp.range.opclass)}
                  {rangeInput("collation", ttyp.range.collation, undefined, true)}
                  {rangeInput("canonical", ttyp.range.canonical, ttyp.range.fnHint)}
                  {rangeInput("subtypeDiff", ttyp.range.subtypeDiff, ttyp.range.fnHint)}
                </div>
                {rangeInput("multirangeTypeName", ttyp.range.multirange, ttyp.range.multirangeHint)}
              </Section>
            </Show>
          </form>
        );
      }}
    </Show>
  );
}
