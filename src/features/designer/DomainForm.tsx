// Domain formu (tembel parça): Genel (şema, ad, sahip, yorum), Tanım (temel tip, collation — ikisi alter'da salt okunur;
// DEFAULT ifadesi, NOT NULL), Domain kısıtları (CHECK; `RowList`). İstemci SQL üretmez; kurallar `domain-model`'de
// (Karar 6: alter'da yeni kısıt adı zorunlu — yerel ön uyarı; NOT VALID yalnız eklenen kısıtta; "Doğrula" yalnız yüklü,
// doğrulanmamış ve ifadesi değişmeyen kısıtta); diğer kurallar arka ucun alan hatasıdır. `editable = false` tanım (ya
// da `readOnly` sekme) tamamen salt okunurdur (`ReadOnlyReasons`). Serbest SQL alanlarının (DEFAULT, CHECK) altında
// `FreeSqlNote` + `E'…'` ipucu (spec §8.7). Kobalte yok; metinler tembel `strings-domain`.
import { Undo } from "lucide-solid";
import { For, Show, createResource, createSignal, createUniqueId, type JSX } from "solid-js";
import { tdom } from "../../app/strings-domain";
import type { DomainCheck } from "../../ipc/bindings/DomainCheck";
import type { DomainDef } from "../../ipc/bindings/DomainDef";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import {
  asDomain, canValidate, checkRename, checkRestore, checkUpdate, domainPatch, needsName, newCheck, notValidAllowed, removedChecks,
} from "./domain-model";
import { CheckRow, CommentField, FieldRow, FreeSqlNote, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import { RowList } from "./RowList";
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

/** Rozet; `id` verilirse sr metni o id ile satır girişine `aria-describedby` olarak bağlanır. */
function Badge(props: { text: string; sr?: string; id?: string; warn?: boolean }) {
  return (
    <span
      class="inline-flex h-5 w-fit shrink-0 items-center rounded border px-1.5 text-[11px]"
      classList={{ "border-accent text-accent": !props.warn, "border-[color:var(--srv-amber)] text-warn-text": !!props.warn }}
    >
      {props.text}
      <Show when={props.sr}>
        <span id={props.id} class="sr-only">
          {" "}
          — {props.sr}
        </span>
      </Show>
    </span>
  );
}

export default function DomainForm(props: FormProps) {
  const draft = () => asDomain(props.model.draft);
  const original = () => asDomain(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez (Karar 3): her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);

  const patch = (p: Partial<DomainDef>) => props.designer.edit(props.tabId, (x) => domainPatch(x, p));
  const edit = (f: (d: DomainDef) => DomainDef) =>
    props.designer.edit(props.tabId, (x) => {
      const d = asDomain(x);
      return d ? { ...f(d), kind: "domain" as const } : x;
    });
  const setChecks = (constraints: DomainCheck[]) => patch({ constraints });

  const schemaList = createUniqueId();
  const collationList = createUniqueId();
  const defaultNoteId = createUniqueId();
  const defaultEscId = createUniqueId();
  const checksNoteId = createUniqueId();
  const checksEscId = createUniqueId();
  const createOnlyId = createUniqueId();
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
        const checks = () => d().constraints;
        const createOnlyLocked = () => locked() || alter();
        const topErrors = () => [...issuesFor(props.issues, "object")];
        const loadedCheck = (c: DomainCheck) =>
          c.original === null ? undefined : original()?.constraints.find((o) => o.name === c.original);
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />
            <datalist id={collationList}>
              <For each={collations()}>{(it) => <option value={it.value} />}</For>
            </datalist>

            <Section title={tdom.sections.general}>
              <FieldRow
                label={tdom.general.schema}
                path="schema"
                issues={props.issues}
                hint={alter() ? tdom.general.schemaAlterHint : tdom.general.schemaCreateHint}
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
              <FieldRow label={alter() ? tdom.general.rename : tdom.general.name} path="name" issues={props.issues}>
                {(a) => (
                  <input
                    id={a.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(a.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={tdom.general.namePlaceholder}
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

            <Section title={tdom.sections.definition}>
              <div class="grid grid-cols-[1fr_12rem] items-start gap-2">
                <FieldRow label={tdom.definition.baseType} path="baseType" issues={props.issues}>
                  {(f) => (
                    <TypeInput
                      designer={props.designer}
                      tabId={props.tabId}
                      id={f.id}
                      value={d().baseType}
                      describedBy={[f.describedBy, alter() ? createOnlyId : null].filter(Boolean).join(" ") || undefined}
                      invalid={f.invalid}
                      disabled={createOnlyLocked()}
                      onInput={(baseType) => patch({ baseType })}
                    />
                  )}
                </FieldRow>
                <FieldRow label={tdom.definition.collation} path="collation" issues={props.issues}>
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      list={collationList}
                      autocomplete="off"
                      spellcheck={false}
                      placeholder={tdom.definition.collationPlaceholder}
                      aria-describedby={[f.describedBy, alter() ? createOnlyId : null].filter(Boolean).join(" ") || undefined}
                      aria-invalid={f.invalid}
                      disabled={createOnlyLocked()}
                      value={d().collation ?? ""}
                      onFocus={askCollations}
                      onInput={(ev) => patch({ collation: normalizeEmpty(ev.currentTarget.value) })}
                    />
                  )}
                </FieldRow>
              </div>
              <Show when={alter()}>
                <p id={createOnlyId} class="text-xs text-muted">
                  {tdom.definition.createOnlyHint}
                </p>
              </Show>
              <FieldRow label={tdom.definition.default} path="default" issues={props.issues}>
                {(f) => (
                  <textarea
                    id={f.id}
                    rows={2}
                    class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(f.invalid)}`}
                    spellcheck={false}
                    placeholder={tdom.definition.defaultPlaceholder}
                    aria-describedby={[f.describedBy, defaultNoteId, defaultEscId].filter(Boolean).join(" ")}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().default ?? ""}
                    onInput={(ev) => patch({ default: normalizeEmpty(ev.currentTarget.value) })}
                  />
                )}
              </FieldRow>
              <FreeSqlNote id={defaultNoteId} />
              <p id={defaultEscId} class="text-xs text-muted">
                {tdom.escapeHint}
              </p>
              <CheckRow
                label={tdom.definition.notNull}
                path="notNull"
                issues={props.issues}
                hint={tdom.definition.notNullHint}
                checked={d().notNull}
                disabled={locked()}
                onChange={(notNull) => patch({ notNull })}
              />
              <Show when={alter()}>
                <p role="note" class="text-xs text-muted">
                  {tdom.definition.pg17Note}
                </p>
              </Show>
            </Section>

            <div class="flex flex-col gap-2">
              <ListErrors
                messages={[...exactIssues(props.issues, "constraints"), ...orphanRowIssues(props.issues, "constraints", checks().length)]}
              />
              <RowList
                label={tdom.sections.checks}
                items={checks()}
                onChange={setChecks}
                name={(c) => c.name ?? ""}
                // Create'te ad isteğe bağlıdır (boş ad `null` = sunucu verir); alter'da boş başlar ve zorunludur.
                newItem={() => (alter() ? newCheck() : { ...newCheck(), name: null })}
                addLabel={tdom.checks.add}
                emptyText={tdom.checks.empty}
                readOnly={locked()}
                reorderable={false}
              >
                {(c, i) => {
                  const loaded = () => loadedCheck(c());
                  const badgeId = createUniqueId();
                  const isNew = () => alter() && c().original === null;
                  const exprChanged = () => {
                    const o = loaded();
                    return o !== undefined && o.expression !== c().expression;
                  };
                  const hiddenFlagErrors = () => [
                    ...exactIssues(props.issues, `constraints[${i}].original`),
                    ...(notValidAllowed(c(), original()) ? [] : exactIssues(props.issues, `constraints[${i}].notValid`)),
                    ...(canValidate(c(), original()) ? [] : exactIssues(props.issues, `constraints[${i}].validate`)),
                  ];
                  return (
                    <div class="flex flex-col gap-2">
                      <FieldRow
                        label={tdom.checks.name}
                        path={`constraints[${i}].name`}
                        issues={props.issues}
                        exact
                        warning={needsName(props.model.mode, c()) ? tdom.checks.nameRequired : null}
                        hint={c().original !== null && c().original !== c().name ? tdom.checks.renamedFrom(c().original!) : undefined}
                      >
                        {(f) => (
                          <input
                            id={f.id}
                            class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                            autocomplete="off"
                            spellcheck={false}
                            required={alter()}
                            aria-required={alter() ? "true" : undefined}
                            placeholder={alter() ? tdom.checks.namePlaceholderAlter : tdom.checks.namePlaceholderCreate}
                            aria-describedby={[f.describedBy, isNew() ? badgeId : null].filter(Boolean).join(" ") || undefined}
                            aria-invalid={f.invalid}
                            disabled={locked()}
                            value={c().name ?? ""}
                            onInput={(ev) => {
                              const name = ev.currentTarget.value;
                              edit((x) => checkRename(x, i, name));
                            }}
                          />
                        )}
                      </FieldRow>
                      <FieldRow
                        label={tdom.checks.expression}
                        path={`constraints[${i}].expression`}
                        issues={props.issues}
                        exact
                        warning={exprChanged() ? tdom.checks.exprChanged : null}
                      >
                        {(f) => (
                          <textarea
                            id={f.id}
                            rows={2}
                            class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(f.invalid)}`}
                            spellcheck={false}
                            required
                            aria-required="true"
                            placeholder={tdom.checks.expressionPlaceholder}
                            aria-describedby={[f.describedBy, checksNoteId, checksEscId].filter(Boolean).join(" ")}
                            aria-invalid={f.invalid}
                            disabled={locked()}
                            value={c().expression}
                            onInput={(ev) => {
                              const expression = ev.currentTarget.value;
                              edit((x) => checkUpdate(x, i, { expression }, original()));
                            }}
                          />
                        )}
                      </FieldRow>
                      <div class="flex flex-wrap items-center gap-2">
                        <Show when={isNew()}>
                          <Badge text={tdom.checks.newBadge} sr={tdom.checks.newBadgeSr} id={badgeId} />
                        </Show>
                        <Show when={c().original !== null && !c().validated}>
                          <Badge text={tdom.checks.notValidated} sr={tdom.checks.notValidatedSr} warn />
                        </Show>
                      </div>
                      <Show when={notValidAllowed(c(), original())}>
                        <CheckRow
                          label={tdom.checks.notValid}
                          path={`constraints[${i}].notValid`}
                          issues={props.issues}
                          checked={c().notValid}
                          disabled={locked()}
                          onChange={(notValid) => edit((x) => checkUpdate(x, i, { notValid }, original()))}
                        />
                      </Show>
                      <Show when={canValidate(c(), original())}>
                        <CheckRow
                          label={tdom.checks.validate}
                          path={`constraints[${i}].validate`}
                          issues={props.issues}
                          hint={tdom.checks.validateHint}
                          checked={c().validate}
                          disabled={locked()}
                          onChange={(validate) => edit((x) => checkUpdate(x, i, { validate }, original()))}
                        />
                      </Show>
                      <ListErrors messages={hiddenFlagErrors()} />
                    </div>
                  );
                }}
              </RowList>
              <Show when={removedChecks(original(), d()).length > 0}>
                <div role="note" class="flex flex-col gap-1 rounded border border-border px-3 py-2 text-xs">
                  <p class="font-medium text-warn-text">{tdom.checks.removedTitle}</p>
                  <ul class="flex flex-col gap-1">
                    <For each={removedChecks(original(), d())}>
                      {(n) => (
                        <li class="flex items-center gap-2">
                          <span class="min-w-0 flex-1 font-mono text-warn-text">{tdom.checks.removed(n)}</span>
                          <Show when={!locked()}>
                            <Button
                              variant="ghost"
                              aria-label={tdom.checks.restore(n)}
                              title={tdom.checks.restore(n)}
                              onClick={() => edit((x) => checkRestore(x, original(), n))}
                            >
                              <Undo size={14} aria-hidden="true" /> {tdom.checks.restoreShort}
                            </Button>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </Show>
              <FreeSqlNote id={checksNoteId} />
              <p id={checksEscId} class="text-xs text-muted">
                {tdom.escapeHint}
              </p>
              <Show when={alter() && checks().some((c) => c.original !== null)}>
                <p class="text-xs text-muted">{tdom.checks.removeHint}</p>
              </Show>
            </div>
          </form>
        );
      }}
    </Show>
  );
}
