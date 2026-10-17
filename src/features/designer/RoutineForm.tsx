// Function / Procedure formu (tembel parça; DesignerView'da iki tür aynı `lazy` girdisini paylaşır): Genel, Argümanlar,
// Dönüş (yalnız Function), Gövde, Seçenekler, Ayarlar, Yeniden oluşturma. İstemci SQL üretmez: gövde/varsayılan/tip
// metinleri olduğu gibi gider; gövdenin dolar tırnaklaması arka uçtadır. `editable = false` tanım (ya da `readOnly`
// sekme) tamamen salt okunurdur; gerekçeler üstte (`ReadOnlyReasons`). Kobalte yok; metinler tembel `strings-routine`.
import { For, Show, createEffect, createMemo, createResource, createUniqueId, on, type JSX } from "solid-js";
import { trout } from "../../app/strings-routine";
import type { ArgMode } from "../../ipc/bindings/ArgMode";
import type { Parallel } from "../../ipc/bindings/Parallel";
import type { ReturnColumn } from "../../ipc/bindings/ReturnColumn";
import type { Volatility } from "../../ipc/bindings/Volatility";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CheckRow, CommentField, FieldRow, FreeSqlNote, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import {
  argModes, asRoutine, hasRecreateError, languageChoices, newArg, newReturnColumn, recreateLoss, recreateNeedLabel, recreateNeeds,
  returnColumns, returnKindOf, returnTypeName, routinePatch, rowsVisible, securityDefinerWarning, syncRecreate, tableOutArgsWarning, updateAt, withReturns,
  type ReturnChoice,
} from "./routine-model";
import { RowList } from "./RowList";
import { SettingsEditor } from "./SettingsEditor";
import { SqlBodyEditor } from "./SqlBodyEditor";
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

/** Liste düzeyi hata metni (ör. `args`, `returns.columns`) + listede olmayan satırların hataları. */
function ListErrors(props: { messages: string[] }) {
  return (
    <Show when={props.messages.length > 0}>
      <p role="alert" class="text-xs text-danger">
        {props.messages.join(" ")}
      </p>
    </Show>
  );
}

const RETURN_CHOICES: ReturnChoice[] = ["type", "setof", "table", "trigger", "void"];
const VOLATILITIES: Volatility[] = ["volatile", "stable", "immutable"];
const PARALLELS: Parallel[] = ["unsafe", "restricted", "safe"];

export default function RoutineForm(props: FormProps) {
  const draft = () => asRoutine(props.model.draft);
  const original = () => asRoutine(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez (Karar 3): her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);
  const needs = createMemo(() => {
    const d = draft();
    return alter() && d ? recreateNeeds(original(), d) : [];
  });

  const patch = (p: Parameters<typeof routinePatch>[1]) =>
    props.designer.edit(props.tabId, (x) => {
      const n = asRoutine(routinePatch(x, p));
      return n ? syncRecreate(original(), n) : x;
    });

  const schemaList = createUniqueId();
  const bodyNoteId = createUniqueId();
  const dollarId = createUniqueId();
  const argsNoteId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  let recreateEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  // Arka uç "…yeniden oluşturmayı gerektirir…" alan hatası (recreate seçilmemiş) → kutuya odak. Kullanıcı bir metin
  // alanında yazarken odak çalınmaz (yazılan karakterler kutuya gitmesin); kutu zaten görünür (istemci listesi üst küme).
  createEffect(
    on(
      () => hasRecreateError(props.issues) && needs().length > 0 && !draft()?.recreate && !locked(),
      (yes) => {
        if (!yes) return;
        queueMicrotask(() => {
          const a = document.activeElement;
          const typing = a instanceof HTMLTextAreaElement || (a instanceof HTMLInputElement && a.type !== "checkbox");
          if (!typing) recreateEl?.focus();
        });
      },
    ),
  );

  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  const [languages] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "languages" }).catch(() => []),
  );

  return (
    <Show when={draft()}>
      {(d) => {
        const isFunction = () => d().kind === "function";
        const returnKind = () => returnKindOf(d().returns);
        const triggerReturn = () => returnKind() === "trigger";
        const langs = () => languageChoices(languages() ?? [], d().language);
        const untrusted = () => langs().find((l) => l.value === d().language)?.detail === trout.general.languageUntrustedShort;
        // `kind`/`object` (ve kutu gizliyken `recreate`) hataları formun üstünde: kök listede oldukları için genel şeride düşmez.
        const topErrors = () => [
          ...issuesFor(props.issues, "kind"),
          ...issuesFor(props.issues, "object"),
          ...(needs().length === 0 ? issuesFor(props.issues, "recreate") : []),
        ];
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />

            <Section title={trout.sections.general}>
              <FieldRow
                label={trout.general.schema}
                path="schema"
                issues={props.issues}
                hint={alter() ? trout.general.schemaAlterHint : trout.general.schemaCreateHint}
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
              <FieldRow label={alter() ? trout.general.rename : trout.general.name} path="name" issues={props.issues}>
                {(a) => (
                  <input
                    id={a.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(a.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={trout.general.namePlaceholder}
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
              <FieldRow
                label={trout.general.language}
                path="language"
                issues={props.issues}
                hint={untrusted() ? trout.general.languageUntrusted : undefined}
              >
                {(a) => (
                  <select
                    id={a.id}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={locked()}
                    value={d().language}
                    onChange={(ev) => patch({ language: ev.currentTarget.value })}
                  >
                    <For each={langs()}>
                      {(l) => <option value={l.value}>{l.detail ? `${l.value} (${l.detail})` : l.value}</option>}
                    </For>
                  </select>
                )}
              </FieldRow>
            </Section>

            <div class="flex flex-col gap-2">
              <RowList
                label={trout.sections.args}
                items={d().args}
                onChange={(args) => patch({ args })}
                name={(a) => a.name ?? ""}
                // Trigger dönüşünde liste kapalıdır (Ekle yok, alanlar kapalı); kalan satırlar silinebilir (arka uç hatası giderilsin).
                newItem={triggerReturn() ? undefined : newArg}
                addLabel={trout.args.add}
                emptyText={trout.args.empty}
                readOnly={locked()}
              >
                {(a, i) => (
                  <div class="grid grid-cols-[7rem_1fr] gap-2 sm:grid-cols-[7rem_1fr_1fr]">
                    <FieldRow label={trout.args.mode} path={`args[${i}].mode`} issues={props.issues} exact
                      hint={!isFunction() && a().mode === "out" ? trout.args.procedureOutHint : undefined}
                    >
                      {(f) => (
                        <select
                          id={f.id}
                          class={inputClass + invalidClass(f.invalid)}
                          aria-describedby={f.describedBy}
                          aria-invalid={f.invalid}
                          disabled={locked() || triggerReturn()}
                          value={a().mode}
                          onChange={(ev) => patch({ args: updateAt(d().args, i, { mode: ev.currentTarget.value as ArgMode }) })}
                        >
                          <For each={argModes(d().kind)}>{(m) => <option value={m}>{trout.args.modes[m]}</option>}</For>
                        </select>
                      )}
                    </FieldRow>
                    <FieldRow label={trout.args.name} path={`args[${i}].name`} issues={props.issues} exact>
                      {(f) => (
                        <input
                          id={f.id}
                          class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                          autocomplete="off"
                          spellcheck={false}
                          placeholder={trout.args.namePlaceholder}
                          aria-describedby={f.describedBy}
                          aria-invalid={f.invalid}
                          disabled={locked() || triggerReturn()}
                          value={a().name ?? ""}
                          onInput={(ev) => patch({ args: updateAt(d().args, i, { name: normalizeEmpty(ev.currentTarget.value) }) })}
                        />
                      )}
                    </FieldRow>
                    <FieldRow label={trout.args.type} path={`args[${i}].typeName`} issues={props.issues} exact>
                      {(f) => (
                        <TypeInput
                          designer={props.designer}
                          tabId={props.tabId}
                          id={f.id}
                          value={a().typeName}
                          describedBy={f.describedBy}
                          invalid={f.invalid}
                          disabled={locked() || triggerReturn()}
                          onInput={(v) => patch({ args: updateAt(d().args, i, { typeName: v }) })}
                        />
                      )}
                    </FieldRow>
                    <div class="col-span-full">
                      <FieldRow label={trout.args.default} path={`args[${i}].default`} issues={props.issues} exact>
                        {(f) => (
                          <input
                            id={f.id}
                            class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                            autocomplete="off"
                            spellcheck={false}
                            placeholder={trout.args.defaultPlaceholder}
                            aria-describedby={[f.describedBy, argsNoteId].filter(Boolean).join(" ")}
                            aria-invalid={f.invalid}
                            disabled={locked() || triggerReturn()}
                            value={a().default ?? ""}
                            onInput={(ev) =>
                              patch({ args: updateAt(d().args, i, { default: normalizeEmpty(ev.currentTarget.value) }) })
                            }
                          />
                        )}
                      </FieldRow>
                    </div>
                  </div>
                )}
              </RowList>
              <ListErrors messages={[...exactIssues(props.issues, "args"), ...orphanRowIssues(props.issues, "args", d().args.length)]} />
              <Show when={triggerReturn()}>
                <p class="text-xs text-muted">{trout.args.triggerClosed}</p>
              </Show>
              <Show when={tableOutArgsWarning(d())}>
                <p role="note" class="text-xs text-warn-text">
                  {trout.args.tableOutWarning}
                </p>
              </Show>
              <Show when={d().args.length > 0}>
                <FreeSqlNote id={argsNoteId} />
              </Show>
            </div>

            <Show when={isFunction()}>
              <Section title={trout.sections.returns}>
                <FieldRow label={trout.returns.kind} path="returns" issues={props.issues} exact>
                  {(f) => (
                    <select
                      id={f.id}
                      class={inputClass + invalidClass(f.invalid)}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={returnKind() ?? "void"}
                      onChange={(ev) =>
                        props.designer.edit(props.tabId, (x) => {
                          const r = asRoutine(x);
                          return r ? syncRecreate(original(), { ...withReturns(r, ev.currentTarget.value as ReturnChoice), kind: r.kind }) : x;
                        })
                      }
                    >
                      <For each={RETURN_CHOICES}>{(k) => <option value={k}>{trout.returns.kinds[k]}</option>}</For>
                    </select>
                  )}
                </FieldRow>
                <Show when={returnKind() === "type" || returnKind() === "setof"}>
                  <FieldRow label={trout.returns.type} path="returns.typeName" issues={props.issues}>
                    {(f) => (
                      <TypeInput
                        designer={props.designer}
                        tabId={props.tabId}
                        id={f.id}
                        value={returnTypeName(d().returns)}
                        describedBy={f.describedBy}
                        invalid={f.invalid}
                        disabled={locked()}
                        onInput={(v) => {
                          const cur = d().returns;
                          if (cur?.kind === "type") patch({ returns: { ...cur, typeName: v } });
                        }}
                      />
                    )}
                  </FieldRow>
                </Show>
                <Show when={returnKind() === "table"}>
                  {(() => {
                    const cols = () => returnColumns(d().returns);
                    const setCols = (columns: ReturnColumn[]) => patch({ returns: { kind: "table", columns } });
                    return (
                      <>
                        <RowList
                          label={trout.returns.columns}
                          items={cols()}
                          onChange={setCols}
                          name={(c) => c.name}
                          newItem={newReturnColumn}
                          addLabel={trout.returns.addColumn}
                          emptyText={trout.returns.noColumns}
                          readOnly={locked()}
                        >
                          {(c, i) => (
                            <div class="grid grid-cols-2 gap-2">
                              <FieldRow label={trout.returns.columnName} path={`returns.columns[${i}].name`} issues={props.issues} exact>
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
                                    value={c().name}
                                    onInput={(ev) => setCols(updateAt(cols(), i, { name: ev.currentTarget.value }))}
                                  />
                                )}
                              </FieldRow>
                              <FieldRow label={trout.returns.columnType} path={`returns.columns[${i}].typeName`} issues={props.issues} exact>
                                {(f) => (
                                  <TypeInput
                                    designer={props.designer}
                                    tabId={props.tabId}
                                    id={f.id}
                                    value={c().typeName}
                                    describedBy={f.describedBy}
                                    invalid={f.invalid}
                                    disabled={locked()}
                                    onInput={(v) => setCols(updateAt(cols(), i, { typeName: v }))}
                                  />
                                )}
                              </FieldRow>
                            </div>
                          )}
                        </RowList>
                        <ListErrors
                          messages={[
                            ...exactIssues(props.issues, "returns.columns"),
                            ...orphanRowIssues(props.issues, "returns.columns", cols().length),
                          ]}
                        />
                      </>
                    );
                  })()}
                </Show>
              </Section>
            </Show>

            <Section title={trout.sections.body}>
              <FieldRow label={trout.body.label} path="body" issues={props.issues}>
                {(f) => (
                  <SqlBodyEditor
                    id={f.id}
                    label={trout.body.label}
                    value={d().body}
                    onChange={(body) => patch({ body })}
                    readOnly={locked()}
                    invalid={f.invalid}
                    describedBy={[f.describedBy, bodyNoteId, dollarId].filter(Boolean).join(" ")}
                  />
                )}
              </FieldRow>
              <FreeSqlNote id={bodyNoteId} />
              <p id={dollarId} class="text-xs text-muted">
                {trout.body.dollarNote}
              </p>
            </Section>

            <Section title={trout.sections.options}>
              <Show when={isFunction()}>
                <FieldRow label={trout.options.volatility} path="volatility" issues={props.issues}>
                  {(f) => (
                    <select
                      id={f.id}
                      class={inputClass + invalidClass(f.invalid)}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().volatility}
                      onChange={(ev) => patch({ volatility: ev.currentTarget.value as Volatility })}
                    >
                      <For each={VOLATILITIES}>{(v) => <option value={v}>{trout.options.volatilities[v]}</option>}</For>
                    </select>
                  )}
                </FieldRow>
                <CheckRow
                  label={trout.options.strict}
                  path="strict"
                  issues={props.issues}
                  disabled={locked()}
                  checked={d().strict}
                  onChange={(strict) => patch({ strict })}
                />
              </Show>
              <CheckRow
                label={trout.options.securityDefiner}
                path="securityDefiner"
                issues={props.issues}
                warning={securityDefinerWarning(original(), d()) ? trout.options.securityDefinerWarning : null}
                disabled={locked()}
                checked={d().securityDefiner}
                onChange={(securityDefiner) => patch({ securityDefiner })}
              />
              <Show when={isFunction()}>
                <CheckRow
                  label={trout.options.leakproof}
                  path="leakproof"
                  issues={props.issues}
                  hint={trout.options.leakproofHint}
                  disabled={locked()}
                  checked={d().leakproof}
                  onChange={(leakproof) => patch({ leakproof })}
                />
                <FieldRow label={trout.options.parallel} path="parallel" issues={props.issues}>
                  {(f) => (
                    <select
                      id={f.id}
                      class={inputClass + invalidClass(f.invalid)}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().parallel}
                      onChange={(ev) => patch({ parallel: ev.currentTarget.value as Parallel })}
                    >
                      <For each={PARALLELS}>{(p) => <option value={p}>{trout.options.parallels[p]}</option>}</For>
                    </select>
                  )}
                </FieldRow>
                <FieldRow label={trout.options.cost} path="cost" issues={props.issues} hint={trout.options.numberHint}>
                  {(f) => (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      inputmode="decimal"
                      autocomplete="off"
                      spellcheck={false}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().cost ?? ""}
                      onInput={(ev) => patch({ cost: normalizeEmpty(ev.currentTarget.value) })}
                    />
                  )}
                </FieldRow>
                <Show when={rowsVisible(d()) || d().rows !== null}>
                  <FieldRow label={trout.options.rows} path="rows" issues={props.issues} hint={trout.options.numberHint}>
                    {(f) => (
                      <input
                        id={f.id}
                        class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                        inputmode="decimal"
                        autocomplete="off"
                        spellcheck={false}
                        aria-describedby={f.describedBy}
                        aria-invalid={f.invalid}
                        disabled={locked()}
                        value={d().rows ?? ""}
                        onInput={(ev) => patch({ rows: normalizeEmpty(ev.currentTarget.value) })}
                      />
                    )}
                  </FieldRow>
                </Show>
              </Show>
            </Section>

            <SettingsEditor
              {...props}
              readOnly={locked()}
              path="settings"
              list={d().settings}
              original={original()?.settings ?? null}
              onChange={(settings) => patch({ settings })}
            />

            <Show when={needs().length > 0}>
              <Section title={trout.sections.recreate}>
                <CheckRow
                  ref={(el) => (recreateEl = el)}
                  label={trout.recreate.label}
                  path="recreate"
                  issues={props.issues}
                  hint={trout.recreate.note}
                  disabled={locked()}
                  checked={d().recreate}
                  onChange={(recreate) => patch({ recreate })}
                />
                <div role="note" class="flex flex-col gap-2 rounded border border-border px-3 py-2 text-xs">
                  <div>
                    <p class="font-medium">{trout.recreate.changes}</p>
                    <ul class="list-disc pl-4 text-muted">
                      <For each={needs()}>{(p) => <li>{recreateNeedLabel(p)}</li>}</For>
                    </ul>
                  </div>
                  <div class="text-warn-text">
                    <p class="font-medium">{trout.recreate.lossTitle}</p>
                    <ul class="list-disc pl-4">
                      <For each={recreateLoss()}>{(l) => <li>{l}</li>}</For>
                    </ul>
                  </div>
                </div>
              </Section>
            </Show>
          </form>
        );
      }}
    </Show>
  );
}
