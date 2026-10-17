// Trigger formu (tembel parça): Genel (tablo salt okunur, ad, yorum, etkinlik), Zamanlama ve olaylar (BEFORE/AFTER,
// INSERT/UPDATE/DELETE/TRUNCATE, UPDATE OF kolon çipleri), Düzey, WHEN, Fonksiyon (lookup + şema/ad), Argümanlar,
// Geçiş tabloları, Constraint trigger. İstemci SQL üretmez: WHEN olduğu gibi gider. Açıkça geçersiz birleşimler kapalıdır
// (Karar 6; `availability`), kapanan seçeneğin değeri her düzenlemede temizlenir (`normalizeTrigger`); diğer kurallar arka
// ucun alan hatasıdır. `editable = false` tanım (ya da `readOnly` sekme) tamamen salt okunurdur (`ReadOnlyReasons`).
// Kobalte yok; metinler tembel `strings-trigger`.
import { For, Show, createMemo, createResource, createUniqueId, type JSX } from "solid-js";
import { ttrg } from "../../app/strings-trigger";
import type { TriggerEnabled } from "../../ipc/bindings/TriggerEnabled";
import type { TriggerDef } from "../../ipc/bindings/TriggerDef";
import type { TriggerEvents } from "../../ipc/bindings/TriggerEvents";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CheckRow, CommentField, FieldRow, FreeSqlNote, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import { RowList } from "./RowList";
import {
  asTrigger, availability, functionFromLookup, functionOptionIndex, normalizeTrigger, rebuildNote, restoreCleared, setAt,
  stashFor, toggleUpdateColumn, triggerPatch, updateColumnChoices, type TriggerStash,
} from "./trigger-model";

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

function ListErrors(props: { messages: string[]; id?: string }) {
  return (
    <Show when={props.messages.length > 0}>
      <p id={props.id} role="alert" class="text-xs text-danger">
        {props.messages.join(" ")}
      </p>
    </Show>
  );
}

/** Radyo/onay kutusu satırı; kapalıysa gerekçe `title` ile (görünür gerekçe çağıranda, `describedBy` ile bağlı). */
function Choice(props: {
  type: "radio" | "checkbox";
  name?: string;
  label: string;
  checked: boolean;
  disabled: boolean;
  reason?: string;
  describedBy?: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label class="flex items-center gap-2 text-[13px]" classList={{ "text-muted": props.disabled }} title={props.reason}>
      <input
        type={props.type}
        name={props.name}
        aria-describedby={props.describedBy}
        disabled={props.disabled}
        checked={props.checked}
        onChange={(ev) => props.onChange(ev.currentTarget.checked)}
      />
      {props.label}
    </label>
  );
}

const ENABLED: TriggerEnabled[] = ["origin", "replica", "always", "disabled"];
const EVENTS = ["insert", "update", "delete", "truncate"] as const;

export default function TriggerForm(props: FormProps) {
  const draft = () => asTrigger(props.model.draft);
  const original = () => asTrigger(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez (Karar 3): her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);

  /**
   * Kapanınca temizlenen UPDATE OF / OLD TABLE / NEW TABLE değerleri; alan yeniden açılınca geri gelir. Yalnız son
   * `patch`'in ürettiği taslak sürümünde (`stashAt`) geçerlidir (`stashFor`): yükleme/yeniden yükleme sürümü değiştirir
   * ve saklananlar düşer. Sekme yeniden açılınca form örneği yenidir.
   */
  let stash: TriggerStash = {};
  let stashAt: number | null = null;
  const patch = (p: Partial<TriggerDef>) => {
    stash = stashFor(stash, stashAt, props.model.version);
    props.designer.edit(props.tabId, (x) => {
      const prev = asTrigger(x);
      const n = asTrigger(triggerPatch(x, p));
      if (!prev || !n) return x;
      const r = restoreCleared(prev, normalizeTrigger(n), stash);
      stash = r.stash;
      return r.def;
    });
    stashAt = props.model.version;
  };
  const patchEvents = (p: Partial<TriggerEvents>) => {
    const d = draft();
    if (d) patch({ events: { ...d.events, ...p } });
  };

  const timingName = createUniqueId();
  const levelName = createUniqueId();
  const timingErrId = createUniqueId();
  const eventsErrId = createUniqueId();
  const levelErrId = createUniqueId();
  const beforeWhyId = createUniqueId();
  const truncateWhyId = createUniqueId();
  const whenNoteId = createUniqueId();
  const whenEscId = createUniqueId();
  const fnPickId = createUniqueId();
  const fnPickHintId = createUniqueId();
  const argsHintId = createUniqueId();
  const transitionHintId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const tableKey = createMemo(() => {
    const d = draft();
    return d && d.schema !== "" && d.table !== "" ? JSON.stringify([d.schema, d.table]) : null;
  });
  const [tableColumns] = createResource(tableKey, (key) => {
    const [schema, table] = JSON.parse(key) as [string, string];
    return props.designer.lookup(props.tabId, { kind: "table_columns", schema, table }).catch(() => null);
  });
  const [functions] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "trigger_functions", schema: null }).catch(() => []),
  );

  return (
    <Show when={draft()}>
      {(d) => {
        const a = createMemo(() => availability(d()));
        const fnItems = () => functions() ?? [];
        const fnIndex = () => functionOptionIndex(fnItems(), d());
        const note = () => (alter() ? rebuildNote(original(), d()) : null);
        const columnChoices = () =>
          updateColumnChoices((tableColumns() ?? []).map((c) => c.value), d().events.updateColumns);
        // `schema`/`object` hataları formun üstünde (tablonun şeması salt okunurdur).
        const topErrors = () => [...issuesFor(props.issues, "schema"), ...issuesFor(props.issues, "object")];
        const timingErrors = () => issuesFor(props.issues, "timing");
        const eventErrors = () => [...exactIssues(props.issues, "events"), ...exactIssues(props.issues, "events.truncate")];
        const levelErrors = () => issuesFor(props.issues, "level");
        // Çip başına hata (`events.updateColumns[i]`) kolon adıyla; listenin kendi hataları olduğu gibi.
        const updateColErrors = () => {
          const out: string[] = [];
          for (const [p, msgs] of props.issues.byPath) {
            const m = /^events\.updateColumns\[(\d+)\]$/.exec(p);
            if (m) {
              const i = Number(m[1]);
              const col = d().events.updateColumns[i];
              for (const msg of msgs) out.push(`${col ?? "#" + (i + 1)}: ${msg}`);
            } else if (p === "events.updateColumns" || p.startsWith("events.updateColumns.") || p.startsWith("events.updateColumns[")) {
              out.push(...msgs);
            }
          }
          return out;
        };
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />

            <Section title={ttrg.sections.general}>
              <FieldRow label={ttrg.general.table} path="table" issues={props.issues} hint={ttrg.general.tableHint}>
                {(f) => (
                  <input
                    id={f.id}
                    class={`${inputClass} font-mono text-muted${invalidClass(f.invalid)}`}
                    readOnly
                    aria-readonly="true"
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    value={`${d().schema}.${d().table}`}
                  />
                )}
              </FieldRow>
              <FieldRow label={alter() ? ttrg.general.rename : ttrg.general.name} path="name" issues={props.issues}>
                {(f) => (
                  <input
                    id={f.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(f.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={ttrg.general.namePlaceholder}
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().name}
                    onInput={(ev) => patch({ name: ev.currentTarget.value })}
                  />
                )}
              </FieldRow>
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={locked()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
              <FieldRow label={ttrg.general.enabled} path="enabled" issues={props.issues} hint={ttrg.general.enabledHint}>
                {(f) => (
                  <select
                    id={f.id}
                    class={inputClass + invalidClass(f.invalid)}
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().enabled}
                    onChange={(ev) => patch({ enabled: ev.currentTarget.value as TriggerEnabled })}
                  >
                    <For each={ENABLED}>
                      {(e) => (
                        <option value={e} selected={e === d().enabled}>
                          {ttrg.general.enabledStates[e]}
                        </option>
                      )}
                    </For>
                  </select>
                )}
              </FieldRow>
            </Section>

            <Section title={ttrg.sections.timing}>
              <fieldset class="flex flex-col gap-1" aria-describedby={timingErrors().length > 0 ? timingErrId : undefined}>
                <legend class="text-xs text-muted">{ttrg.timing.label}</legend>
                <div class="flex gap-4">
                  <Choice
                    type="radio"
                    name={timingName}
                    label={ttrg.timing.before}
                    checked={d().timing === "before"}
                    disabled={locked() || !a().before}
                    reason={a().before ? undefined : ttrg.timing.beforeClosed}
                    describedBy={a().before ? undefined : beforeWhyId}
                    onChange={(on) => on && patch({ timing: "before" })}
                  />
                  <Choice
                    type="radio"
                    name={timingName}
                    label={ttrg.timing.after}
                    checked={d().timing === "after"}
                    disabled={locked()}
                    onChange={(on) => on && patch({ timing: "after" })}
                  />
                </div>
                <Show when={!a().before}>
                  <p id={beforeWhyId} class="text-xs text-muted">
                    {ttrg.timing.beforeClosed}
                  </p>
                </Show>
                <ListErrors id={timingErrId} messages={timingErrors()} />
              </fieldset>

              <fieldset class="flex flex-col gap-1" aria-describedby={eventErrors().length > 0 ? eventsErrId : undefined}>
                <legend class="text-xs text-muted">{ttrg.timing.events}</legend>
                <div class="flex flex-wrap gap-4">
                  <For each={EVENTS}>
                    {(k) => (
                      <Choice
                        type="checkbox"
                        label={ttrg.timing[k]}
                        checked={d().events[k]}
                        disabled={locked() || (k === "truncate" && !a().truncate)}
                        reason={k === "truncate" && !a().truncate ? ttrg.timing.truncateClosed : undefined}
                        describedBy={k === "truncate" && !a().truncate ? truncateWhyId : undefined}
                        onChange={(on) => patchEvents({ [k]: on })}
                      />
                    )}
                  </For>
                </div>
                <Show when={!a().truncate}>
                  <p id={truncateWhyId} class="text-xs text-muted">
                    {ttrg.timing.truncateClosed}
                  </p>
                </Show>
                <ListErrors id={eventsErrId} messages={eventErrors()} />
              </fieldset>

              <fieldset class="flex flex-col gap-1">
                <legend class="text-xs text-muted">{ttrg.timing.updateOf}</legend>
                <Show
                  when={a().updateColumns}
                  fallback={<p class="text-xs text-muted">{ttrg.timing.updateOfClosed}</p>}
                >
                  <Show when={tableColumns() !== null || d().events.updateColumns.length > 0} fallback={<p class="text-xs text-muted">{ttrg.timing.noColumns}</p>}>
                    <div class="flex flex-wrap gap-1.5">
                      <For each={columnChoices()}>
                        {(c) => {
                          const on = () => d().events.updateColumns.includes(c);
                          return (
                            <label
                              class="inline-flex h-6 items-center gap-1 rounded-full border px-2 font-mono text-[12px] focus-within:border-accent"
                              classList={{
                                "border-accent": on(),
                                "border-border": !on(),
                                "text-muted": locked(),
                                "cursor-pointer": !locked(),
                                "cursor-default": locked(),
                              }}
                            >
                              <input
                                type="checkbox"
                                class="sr-only"
                                disabled={locked()}
                                checked={on()}
                                onChange={(ev) =>
                                  patchEvents({ updateColumns: toggleUpdateColumn(d().events.updateColumns, c, ev.currentTarget.checked) })
                                }
                              />
                              <span aria-hidden="true">{on() ? "✓" : "+"}</span>
                              {c}
                            </label>
                          );
                        }}
                      </For>
                    </div>
                  </Show>
                  <p class="text-xs text-muted">{ttrg.timing.updateOfHint}</p>
                </Show>
                <ListErrors messages={updateColErrors()} />
              </fieldset>
            </Section>

            <Section title={ttrg.sections.level}>
              <fieldset class="flex flex-col gap-1" aria-describedby={levelErrors().length > 0 ? levelErrId : undefined}>
                <legend class="sr-only">{ttrg.level.label}</legend>
                <div class="flex gap-4">
                  <Choice
                    type="radio"
                    name={levelName}
                    label={ttrg.level.row}
                    checked={d().level === "row"}
                    disabled={locked() || !a().row}
                    reason={a().row ? undefined : ttrg.level.rowClosed}
                    onChange={(on) => on && patch({ level: "row" })}
                  />
                  <Choice
                    type="radio"
                    name={levelName}
                    label={ttrg.level.statement}
                    checked={d().level === "statement"}
                    disabled={locked() || !a().statement}
                    reason={a().statement ? undefined : ttrg.level.statementClosed}
                    onChange={(on) => on && patch({ level: "statement" })}
                  />
                </div>
                <Show when={!a().row}>
                  <p class="text-xs text-muted">{ttrg.level.rowClosed}</p>
                </Show>
                <Show when={!a().statement}>
                  <p class="text-xs text-muted">{ttrg.level.statementClosed}</p>
                </Show>
                <ListErrors id={levelErrId} messages={levelErrors()} />
              </fieldset>
            </Section>

            <Section title={ttrg.sections.when}>
              <FieldRow label={ttrg.when.label} path="when" issues={props.issues} hint={ttrg.when.hint}>
                {(f) => (
                  <textarea
                    id={f.id}
                    rows={3}
                    class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(f.invalid)}`}
                    spellcheck={false}
                    placeholder={ttrg.when.placeholder}
                    aria-describedby={[f.describedBy, whenNoteId, whenEscId].filter(Boolean).join(" ")}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().when ?? ""}
                    onInput={(ev) => patch({ when: normalizeEmpty(ev.currentTarget.value) })}
                  />
                )}
              </FieldRow>
              <FreeSqlNote id={whenNoteId} />
              <p id={whenEscId} class="text-xs text-muted">
                {ttrg.when.escapeHint}
              </p>
            </Section>

            <Section title={ttrg.sections.fn}>
              <div class="flex flex-col gap-1">
                <label for={fnPickId} class="text-xs text-muted">
                  {ttrg.fn.pick}
                </label>
                <select
                  id={fnPickId}
                  class={inputClass}
                  aria-describedby={fnPickHintId}
                  disabled={locked() || fnItems().length === 0}
                  value={String(fnIndex())}
                  onChange={(ev) => {
                    const item = fnItems()[Number(ev.currentTarget.value)];
                    if (item) patch(functionFromLookup(item, d().functionSchema));
                  }}
                >
                  <Show when={fnIndex() < 0}>
                    <option value="-1" selected>
                      {d().functionName.trim() === "" ? ttrg.fn.pickEmpty : ttrg.fn.pickNone}
                    </option>
                  </Show>
                  <For each={fnItems()}>
                    {(item, i) => (
                      <option value={String(i())} selected={i() === fnIndex()}>
                        {item.detail ? `${item.detail}.${item.value}` : item.value}
                      </option>
                    )}
                  </For>
                </select>
                <span id={fnPickHintId} class="text-xs text-muted">
                  {ttrg.fn.pickHint}
                </span>
              </div>
              <div class="grid grid-cols-2 gap-2">
                <FieldRow label={ttrg.fn.schema} path="functionSchema" issues={props.issues}>
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      autocomplete="off"
                      spellcheck={false}
                      required
                      aria-required="true"
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().functionSchema}
                      onInput={(ev) => patch({ functionSchema: ev.currentTarget.value })}
                    />
                  )}
                </FieldRow>
                <FieldRow label={ttrg.fn.name} path="functionName" issues={props.issues}>
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      autocomplete="off"
                      spellcheck={false}
                      required
                      aria-required="true"
                      placeholder={ttrg.fn.namePlaceholder}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().functionName}
                      onInput={(ev) => patch({ functionName: ev.currentTarget.value })}
                    />
                  )}
                </FieldRow>
              </div>
            </Section>

            <div class="flex flex-col gap-2">
              <RowList
                label={ttrg.sections.args}
                items={d().args}
                onChange={(args) => patch({ args })}
                name={(v) => v}
                newItem={() => ""}
                addLabel={ttrg.args.add}
                emptyText={ttrg.args.empty}
                readOnly={locked()}
              >
                {(v, i) => (
                  <FieldRow label={ttrg.args.value} path={`args[${i}]`} issues={props.issues} exact>
                    {(f) => (
                      <input
                        id={f.id}
                        class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                        autocomplete="off"
                        spellcheck={false}
                        placeholder={ttrg.args.placeholder}
                        aria-describedby={[f.describedBy, argsHintId].filter(Boolean).join(" ")}
                        aria-invalid={f.invalid}
                        disabled={locked()}
                        value={v()}
                        onInput={(ev) => patch({ args: setAt(d().args, i, ev.currentTarget.value) })}
                      />
                    )}
                  </FieldRow>
                )}
              </RowList>
              <ListErrors
                messages={[...exactIssues(props.issues, "args"), ...orphanRowIssues(props.issues, "args", d().args.length)]}
              />
              <p id={argsHintId} class="text-xs text-muted">
                {ttrg.args.hint}
              </p>
            </div>

            <Section title={ttrg.sections.transition}>
              <div class="grid grid-cols-2 gap-2">
                <FieldRow
                  label={ttrg.transition.oldTable}
                  path="oldTable"
                  issues={props.issues}
                  hint={a().referencing && !a().oldTable ? ttrg.transition.oldClosed : undefined}
                >
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      autocomplete="off"
                      spellcheck={false}
                      placeholder={ttrg.transition.placeholder}
                      aria-describedby={[f.describedBy, a().referencing ? null : transitionHintId].filter(Boolean).join(" ") || undefined}
                      aria-invalid={f.invalid}
                      disabled={locked() || !a().oldTable}
                      value={d().oldTable ?? ""}
                      onInput={(ev) => patch({ oldTable: normalizeEmpty(ev.currentTarget.value) })}
                    />
                  )}
                </FieldRow>
                <FieldRow
                  label={ttrg.transition.newTable}
                  path="newTable"
                  issues={props.issues}
                  hint={a().referencing && !a().newTable ? ttrg.transition.newClosed : undefined}
                >
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      autocomplete="off"
                      spellcheck={false}
                      placeholder={ttrg.transition.placeholder}
                      aria-describedby={[f.describedBy, a().referencing ? null : transitionHintId].filter(Boolean).join(" ") || undefined}
                      aria-invalid={f.invalid}
                      disabled={locked() || !a().newTable}
                      value={d().newTable ?? ""}
                      onInput={(ev) => patch({ newTable: normalizeEmpty(ev.currentTarget.value) })}
                    />
                  )}
                </FieldRow>
              </div>
              <Show when={!a().referencing}>
                <p id={transitionHintId} class="text-xs text-muted">
                  {ttrg.transition.closed}
                </p>
              </Show>
            </Section>

            <Section title={ttrg.sections.constraint}>
              <CheckRow
                label={ttrg.constraint.label}
                path="constraint"
                issues={props.issues}
                hint={a().constraint ? ttrg.constraint.hint : ttrg.constraint.closed}
                disabled={locked() || !a().constraint}
                checked={d().constraint}
                onChange={(constraint) => patch({ constraint })}
              />
              <Show when={a().deferrable}>
                <div class="flex flex-col gap-2 pl-5">
                  <CheckRow
                    label={ttrg.constraint.deferrable}
                    path="deferrable"
                    issues={props.issues}
                    disabled={locked()}
                    checked={d().deferrable}
                    onChange={(deferrable) => patch({ deferrable })}
                  />
                  <CheckRow
                    label={ttrg.constraint.initiallyDeferred}
                    path="initiallyDeferred"
                    issues={props.issues}
                    hint={d().deferrable ? undefined : ttrg.constraint.initiallyDeferredHint}
                    disabled={locked() || !d().deferrable}
                    checked={d().initiallyDeferred}
                    onChange={(initiallyDeferred) => patch({ initiallyDeferred })}
                  />
                </div>
              </Show>
              {/* Kutu gizliyken gelen DEFERRABLE hataları kaybolmasın. */}
              <Show when={!a().deferrable}>
                <ListErrors messages={[...issuesFor(props.issues, "deferrable"), ...issuesFor(props.issues, "initiallyDeferred")]} />
              </Show>
            </Section>

            <Show when={note()}>
              {(n) => (
                <div role="note" class="flex flex-col gap-1 rounded border border-border px-3 py-2 text-xs">
                  <p class="font-medium">{ttrg.rebuild.title}</p>
                  <p class="text-warn-text">{n()}</p>
                </div>
              )}
            </Show>
          </form>
        );
      }}
    </Show>
  );
}
