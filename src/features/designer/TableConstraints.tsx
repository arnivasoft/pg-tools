// Table formu Kısıtlar bölümü (TableForm parçası): PK / UNIQUE (INCLUDE, NULLS NOT DISTINCT) / CHECK (NO INHERIT) / FK
// kartları, FK hedef tablo/kolon önerileri (kendine başvuran FK'de taslak kolonları), DEFERRABLE / INITIALLY DEFERRED,
// NOT VALID / Doğrula, sarkan referans uyarısı, silinecek kısıtlar (geri al), düzenlenmeyenler listesi ve Explorer odağı.
// Kısıt sayısını ya da FK hedefini değiştiren her düzenleme `editSnap` + `*Snap` sarmalayıcılarıyla yapılır (SelfSnap).
// İstemci SQL üretmez. Not: @kobalte ve ana paketin lucide simgeleri burada kullanılmaz (paket bölünmesi).
import { Trash2, Undo } from "lucide-solid";
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, onCleanup } from "solid-js";
import { ttbl } from "../../app/strings-table";
import type { ConstraintDef } from "../../ipc/bindings/ConstraintDef";
import type { FkAction } from "../../ipc/bindings/FkAction";
import type { FkMatch } from "../../ipc/bindings/FkMatch";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { createDebouncedLookup } from "./debounced-lookup";
import { FieldRow, FreeSqlNote, invalidClass } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import {
  addConstraintColumn, addConstraintSnap, addFkPair, availableColumns, canAddPrimaryKey, canRestoreConstraint, columnChoices, constraintColumns,
  constraintLabel, constraintStructureChanged, danglingColumns, fkRefColumnSource, newConstraint, notValidMode, originalConstraint,
  fkSelfNameOnly, noInheritBlocked, removeConstraintColumn, removeConstraintSnap, removeFkPair, removedConstraints, restoreConstraintWithPk,
  scanNoteVisible, stableAddType, supportsDeferrable, switchConstraintTypeSnap, unsupportedKind, updateConstraint, updateFkPair,
  withDeferrable, withFkActions, withFkTargetSnap, withInitiallyDeferred, withPrimaryKeyNotNull, type ColumnListName, type ConstraintType,
} from "./table-constraints";
import type { TableSectionProps } from "./table-form-state";
import { structureLocked } from "./table-model";

const badge = "inline-flex h-5 items-center rounded border border-border px-1.5 text-[11px]";
const TYPES: ConstraintType[] = ["primary_key", "unique", "check", "foreign_key"];
const MATCH: FkMatch[] = ["simple", "full"];
const ACTIONS: FkAction[] = ["no_action", "restrict", "cascade", "set_null", "set_default"];
const join = (...ids: Array<string | undefined | null | false>) => ids.filter(Boolean).join(" ") || undefined;

interface Lookups {
  schemaList: string;
  tableList: string;
  refColList: string;
  draftColList: string;
  loadSchemas: () => void;
  queryTables: (schema: string) => void;
  queryRefColumns: (schema: string, table: string) => void;
}

export function TableConstraints(props: TableSectionProps) {
  const t = ttbl.constraints;
  const d = () => props.draft;
  const len = () => d().constraints.length;
  const cards = createMemo(() => d().constraints.map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length });
  const locked = () => structureLocked(props.original);
  const editable = () => !props.readOnly && !locked();
  const removed = createMemo(() => removedConstraints(props.original, d()));
  const pkAllowed = () => canAddPrimaryKey(d());
  const [choice, setChoice] = createSignal<ConstraintType>("primary_key");
  // PRIMARY KEY artık eklenemiyorsa seçim kalıcı olarak UNIQUE'e geçer (PK silinince PRIMARY KEY'e geri sıçramaz).
  const addType = (): ConstraintType => stableAddType(choice(), pkAllowed());
  createEffect(() => {
    const next = addType();
    if (next !== choice()) setChoice(next);
  });

  const cardEls: Array<HTMLElement | undefined> = [];
  const nameEls: Array<HTMLInputElement | undefined> = [];
  let addEl: HTMLButtonElement | undefined;
  const focusLater = (el: () => HTMLElement | undefined) => queueMicrotask(() => el()?.focus());

  // Lookup'lar Suspense'e bağlanmaz (panel askıya alınmasın): şema listesi ilk odakta bir kez; hedef tablolar ve hedef
  // kolonlar odak/yazımda debounce'lu, tüm kartlar paylaşır (yalnızca odaktaki girdinin listesi görünür).
  const [schemas, setSchemas] = createSignal<LookupItem[]>([]);
  let schemasAsked = false;
  const tables = createDebouncedLookup<string>((schema) => props.designer.lookup(props.tabId, { kind: "tables", schema }), (s) => s);
  const refColumns = createDebouncedLookup<{ schema: string; table: string }>(
    (k) => props.designer.lookup(props.tabId, { kind: "table_columns", schema: k.schema, table: k.table }),
    (k) => JSON.stringify([k.schema, k.table]),
  );
  onCleanup(() => {
    tables.dispose();
    refColumns.dispose();
  });
  const lookups: Lookups = {
    schemaList: createUniqueId(),
    tableList: createUniqueId(),
    refColList: createUniqueId(),
    draftColList: createUniqueId(),
    loadSchemas: () => {
      if (schemasAsked) return;
      schemasAsked = true;
      props.designer.lookup(props.tabId, { kind: "schemas" }).then(setSchemas, () => (schemasAsked = false));
    },
    queryTables: (schema) => tables.query(schema.trim() === "" ? null : schema),
    queryRefColumns: (schema, table) => refColumns.query(table === "" ? null : { schema, table }),
  };
  const draftColumns = createMemo(() => columnChoices(d(), "").filter((n) => n !== ""));

  // Explorer odağı: istek başına bir kez; yalnızca bölüm etkinken (gizli öğe odaklanamaz/kaydırılamaz).
  let appliedSeq: number | null = null;
  createEffect(() => {
    const h = props.highlight;
    if (!props.active || !h || h.section !== "constraints" || h.index === null || h.seq === appliedSeq) return;
    appliedSeq = h.seq;
    const card = cardEls[h.index];
    if (!card) return;
    card.scrollIntoView?.({ block: "nearest" });
    const name = nameEls[h.index];
    if (editable() && name) name.focus();
    else card.focus();
  });

  const add = () => {
    const type = addType();
    props.editor.editSnap((x, snap) => addConstraintSnap(x, type, snap));
    focusLater(() => nameEls[len() - 1]);
  };

  const listErrors = () => [...exactIssues(props.issues, "constraints"), ...orphanRowIssues(props.issues, "constraints", len())];

  return (
    <form class="flex flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
      <Show when={locked()}>
        <div role="note" class="rounded border border-border px-3 py-2 text-[13px] text-warn-text">
          {ttbl.general.structureLocked}
        </div>
      </Show>
      <Show when={listErrors().length > 0}>
        <span class="text-xs text-danger">{listErrors().join(" ")}</span>
      </Show>
      <Show when={len() === 0}>
        <p class="text-[13px] text-muted">{t.empty}</p>
      </Show>
      <div aria-label={t.listLabel} role="group" class="flex flex-col gap-2">
        <For each={cards()}>
          {(i) => (
            <ConstraintCard
              {...props}
              index={i}
              editable={editable()}
              lookups={lookups}
              cardRef={(el) => (cardEls[i] = el)}
              nameRef={(el) => (nameEls[i] = el)}
              onRemove={() => {
                props.editor.editSnap((x, snap) => removeConstraintSnap(x, i, snap));
                focusLater(() => (len() === 0 ? addEl : nameEls[Math.min(i, len() - 1)]));
              }}
            />
          )}
        </For>
      </div>
      <datalist id={lookups.schemaList}>
        <For each={schemas()}>{(it) => <option value={it.value} />}</For>
      </datalist>
      <datalist id={lookups.tableList}>
        <For each={tables.items()}>{(it) => <option value={it.value} />}</For>
      </datalist>
      <datalist id={lookups.refColList}>
        <For each={refColumns.items()}>{(it) => <option value={it.value} label={it.detail ?? undefined} />}</For>
      </datalist>
      <datalist id={lookups.draftColList}>
        <For each={draftColumns()}>{(n) => <option value={n} />}</For>
      </datalist>
      <Show when={editable()}>
        <AddBar choice={addType()} pkAllowed={pkAllowed()} onChoice={setChoice} onAdd={add} addRef={(el) => (addEl = el)} />
      </Show>
      <Show when={removed().length > 0}>
        <div class="flex flex-col gap-1">
          <span class="text-xs font-medium">{t.removed}</span>
          <ul role="list" aria-label={t.removed} class="flex flex-col gap-1">
            <For each={removed()}>
              {(c) => {
                const label = constraintLabel(c);
                const oid = c.oid;
                const allowed = () => oid !== null && props.original !== null && canRestoreConstraint(d(), props.original, oid);
                return (
                  <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
                    <span class={badge}>{t.types[c.kind.type]}</span>
                    <span class="font-medium line-through">{label}</span>
                    <Show when={editable() && props.original}>
                      {(o) => (
                        <div class="ml-auto">
                          <Button
                            variant="ghost"
                            aria-label={t.restore(label)}
                            title={allowed() ? t.restore(label) : t.restoreBlocked}
                            disabled={!allowed()}
                            onClick={() => {
                              if (oid === null || !allowed()) return;
                              props.editor.editSnap((x, snap) => restoreConstraintWithPk(x, o(), oid, snap));
                              focusLater(() => nameEls[d().constraints.findIndex((k) => k.oid === oid)] ?? addEl);
                            }}
                          >
                            <Undo size={14} aria-hidden="true" />
                          </Button>
                        </div>
                      )}
                    </Show>
                  </li>
                );
              }}
            </For>
          </ul>
          <span class="text-xs text-warn-text">{t.removedNote}</span>
        </div>
      </Show>
      <Show when={d().unsupported.length > 0}>
        <section id={props.unsupportedId} aria-label={ttbl.unsupported.title} class="flex flex-col gap-1">
          <h3 class="text-xs font-medium">{ttbl.unsupported.title}</h3>
          <ul role="list" class="flex flex-col gap-1">
            <For each={d().unsupported}>
              {(entry) => (
                <li class="flex flex-col rounded border border-border px-2 py-1 text-[13px]">
                  <span class="font-mono">{entry}</span>
                  <span class="text-xs text-muted">{ttbl.unsupported[unsupportedKind(entry)]}</span>
                </li>
              )}
            </For>
          </ul>
          <span class="text-xs text-muted">{ttbl.unsupported.note}</span>
        </section>
      </Show>
    </form>
  );
}

function AddBar(props: {
  choice: ConstraintType;
  pkAllowed: boolean;
  onChoice: (t: ConstraintType) => void;
  onAdd: () => void;
  addRef: (el: HTMLButtonElement) => void;
}) {
  const t = ttbl.constraints;
  const id = createUniqueId();
  return (
    <div class="flex flex-wrap items-end gap-2">
      <div class="flex flex-col gap-1">
        <label for={id} class="text-xs text-muted">
          {t.addType}
        </label>
        <select id={id} class={inputClass} value={props.choice} onChange={(ev) => props.onChoice(ev.currentTarget.value as ConstraintType)}>
          <For each={TYPES}>
            {(ty) => (
              <option value={ty} disabled={ty === "primary_key" && !props.pkAllowed} title={ty === "primary_key" && !props.pkAllowed ? t.pkExists : undefined}>
                {t.types[ty]}
              </option>
            )}
          </For>
        </select>
      </div>
      <Button ref={props.addRef} onClick={props.onAdd}>
        {t.add}
      </Button>
    </div>
  );
}

type CardProps = TableSectionProps & {
  index: number;
  editable: boolean;
  lookups: Lookups;
  cardRef: (el: HTMLElement) => void;
  nameRef: (el: HTMLInputElement) => void;
  onRemove: () => void;
};

function ConstraintCard(props: CardProps) {
  const t = ttbl.constraints;
  const i = props.index;
  const p = (f: string) => `constraints[${i}].${f}`;
  // Kart silinirken kapanmadan önce bir kez daha okunabilir: boş kısıtla korunur.
  const c = (): ConstraintDef => props.draft.constraints[i] ?? newConstraint("check", props.draft);
  const orig = createMemo(() => originalConstraint(props.original, c()));
  const ro = () => !props.editable;
  const isNew = () => c().oid === null;
  const label = () => constraintLabel(c());
  const selected = () => props.highlight?.section === "constraints" && props.highlight.index === i;
  const nvMode = () => notValidMode(props.model.mode, c(), orig());
  const dangling = createMemo(() => danglingColumns(props.draft, c(), props.editor.snap().includes(i)));
  const restructured = () => nvMode() === "validate" && constraintStructureChanged(props.original, props.draft, c());

  /** Kısıt düzenlemesi (sayı değişmez); ardından PK üyelerinin NOT NULL'ı korunur. */
  const editC = (f: (k: ConstraintDef) => ConstraintDef) => props.editor.edit((x) => withPrimaryKeyNotNull(updateConstraint(x, i, f)));
  const setTarget = (patch: { refSchema?: string; refTable?: string }) =>
    props.editor.editSnap((x, snap) => withFkTargetSnap(x, i, patch, snap));

  const kindFixedId = createUniqueId();
  const flagHintId = createUniqueId();
  const scanId = createUniqueId();
  const dangId = createUniqueId();
  /** Kartta alanı görünmeyen yollar (kart, kimlik, bayraklar). */
  const cardErrors = () =>
    [`constraints[${i}]`, p("oid"), p("deferrable"), p("initiallyDeferred"), p("notValid"), p("nullsNotDistinct"), p("noInherit")].flatMap((x) =>
      exactIssues(props.issues, x),
    );

  return (
    <section
      ref={props.cardRef}
      aria-label={t.card(i, label())}
      aria-describedby={dangling().length > 0 ? dangId : undefined}
      tabIndex={-1}
      class="flex min-w-0 flex-col gap-2 rounded border border-border p-2 outline-none focus:border-accent"
      classList={{ "bg-accent/10": selected() }}
    >
      <div class="flex items-center gap-1.5">
        <span class="text-xs font-medium" aria-hidden="true">
          {i + 1}.
        </span>
        <span class={badge}>{t.types[c().kind.type]}</span>
        <Show when={isNew()}>
          <span class={badge}>{t.newBadge}</span>
        </Show>
        <Show when={orig()?.notValid}>
          <span class={badge}>{t.notValidBadge}</span>
        </Show>
        <Show when={selected()}>
          <span class={badge}>{t.selectedBadge}</span>
        </Show>
        <Show when={props.editable}>
          <div class="ml-auto">
            <Button variant="ghost" aria-label={t.remove(i, label())} title={t.remove(i, label())} onClick={props.onRemove}>
              <Trash2 size={14} aria-hidden="true" />
            </Button>
          </div>
        </Show>
      </div>
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <FieldRow label={t.name} path={p("name")} issues={props.issues} exact>
          {(a) => (
            <input
              id={a.id}
              ref={props.nameRef}
              class={inputClass + invalidClass(a.invalid)}
              autocomplete="off"
              spellcheck={false}
              placeholder={t.namePlaceholder}
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={ro()}
              value={c().name ?? ""}
              onInput={(ev) => {
                const v = normalizeEmpty(ev.currentTarget.value);
                editC((k) => ({ ...k, name: v }));
              }}
            />
          )}
        </FieldRow>
        <Show
          when={isNew()}
          fallback={
            <div class="flex flex-col gap-1">
              <span class="text-xs text-muted">{t.kind}</span>
              <span id={kindFixedId} class="text-xs text-muted">
                {t.kindFixed}
              </span>
            </div>
          }
        >
          <FieldRow label={t.kind} path={p("kind")} issues={props.issues} exact>
            {(a) => (
              <select
                id={a.id}
                class={inputClass + invalidClass(a.invalid)}
                aria-describedby={a.describedBy}
                disabled={ro()}
                value={c().kind.type}
                onChange={(ev) => {
                  const type = ev.currentTarget.value as ConstraintType;
                  props.editor.editSnap((x, snap) => switchConstraintTypeSnap(x, i, type, snap));
                }}
              >
                <For each={TYPES}>
                  {(ty) => {
                    const blocked = () => ty === "primary_key" && !canAddPrimaryKey(props.draft, i);
                    return (
                      <option value={ty} disabled={blocked()} title={blocked() ? t.pkExists : undefined}>
                        {t.types[ty]}
                      </option>
                    );
                  }}
                </For>
              </select>
            )}
          </FieldRow>
        </Show>
      </div>
      <Show when={c().kind.type === "primary_key" || c().kind.type === "unique"}>
        <ColumnList {...props} c={c()} list="columns" label={t.columns} onEdit={editC} />
        <ColumnList {...props} c={c()} list="include" label={t.include} onEdit={editC} />
        <Show when={c().kind.type === "unique"}>
          <label class="inline-flex items-center gap-1.5 text-[13px]">
            <input
              type="checkbox"
              disabled={ro()}
              checked={c().kind.type === "unique" && (c().kind as { nullsNotDistinct: boolean }).nullsNotDistinct}
              onChange={(ev) => {
                const v = ev.currentTarget.checked;
                editC((k) => (k.kind.type === "unique" ? { ...k, kind: { ...k.kind, nullsNotDistinct: v } } : k));
              }}
            />
            {t.nullsNotDistinct}
          </label>
        </Show>
      </Show>
      <Show when={c().kind.type === "check"}>
        <CheckBody {...props} c={c()} onEdit={editC} />
      </Show>
      <Show when={c().kind.type === "foreign_key"}>
        <FkBody {...props} c={c()} onEdit={editC} setTarget={setTarget} />
      </Show>
      <div class="flex flex-col gap-1">
        <Show when={supportsDeferrable(c().kind.type)}>
          <div class="flex flex-wrap gap-x-4 gap-y-1">
            <label class="inline-flex items-center gap-1.5 text-[13px]">
              <input
                type="checkbox"
                disabled={ro()}
                checked={c().deferrable}
                onChange={(ev) => {
                  const v = ev.currentTarget.checked;
                  editC((k) => withDeferrable(k, v));
                }}
              />
              {t.deferrable}
            </label>
            <label class="inline-flex items-center gap-1.5 text-[13px]">
              <input
                type="checkbox"
                disabled={ro() || !c().deferrable}
                checked={c().initiallyDeferred}
                onChange={(ev) => {
                  const v = ev.currentTarget.checked;
                  editC((k) => withInitiallyDeferred(k, v));
                }}
              />
              {t.initiallyDeferred}
            </label>
          </div>
        </Show>
        <Show when={nvMode() === "add"}>
          <label class="inline-flex items-center gap-1.5 text-[13px]">
            <input
              type="checkbox"
              aria-describedby={flagHintId}
              disabled={ro()}
              checked={c().notValid}
              onChange={(ev) => {
                const v = ev.currentTarget.checked;
                editC((k) => ({ ...k, notValid: v }));
              }}
            />
            {t.notValid}
          </label>
          <span id={flagHintId} class="text-xs text-muted">
            {t.notValidHint}
          </span>
        </Show>
        <Show when={nvMode() === "validate"}>
          <label class="inline-flex items-center gap-1.5 text-[13px]">
            <input
              type="checkbox"
              aria-describedby={flagHintId}
              disabled={ro()}
              checked={!c().notValid}
              onChange={(ev) => {
                const v = ev.currentTarget.checked;
                editC((k) => ({ ...k, notValid: !v }));
              }}
            />
            {t.validate}
          </label>
          <span id={flagHintId} class="text-xs text-muted">
            {restructured() ? t.revalidateHint : t.validateHint}
          </span>
        </Show>
        <Show when={scanNoteVisible(props.model.mode, c())}>
          <span id={scanId} class="text-xs text-muted">
            {t.scanNote}
          </span>
        </Show>
      </div>
      <Show when={dangling().length > 0}>
        <p id={dangId} class="text-xs text-warn-text">
          {t.dangling(dangling().join(", "))}
        </p>
      </Show>
      <Show when={cardErrors().length > 0}>
        <span class="text-xs text-danger">{cardErrors().join(" ")}</span>
      </Show>
    </section>
  );
}

/** PK/UNIQUE kolon ya da INCLUDE listesi: sıralı çipler (çıkar düğmesi) + seçilebilir kolon `select` + Kolon ekle. */
function ColumnList(props: CardProps & { c: ConstraintDef; list: ColumnListName; label: string; onEdit: (f: (k: ConstraintDef) => ConstraintDef) => void }) {
  const t = ttbl.constraints;
  const root = () => `constraints[${props.index}].${props.list}`;
  const names = () => constraintColumns(props.c, props.list);
  const avail = () => availableColumns(props.draft, props.c, props.list);
  const [pick, setPick] = createSignal("");
  const current = () => (avail().includes(pick()) ? pick() : "");
  const labelId = createUniqueId();
  const selectId = createUniqueId();
  const emptyId = createUniqueId();
  const listErrors = () => [...exactIssues(props.issues, root()), ...orphanRowIssues(props.issues, root(), names().length)];
  const ro = () => !props.editable;
  const add = () => {
    const name = current();
    if (name === "") return;
    const list = props.list;
    props.onEdit((k) => addConstraintColumn(k, list, name));
    setPick("");
  };

  return (
    <div role="group" aria-labelledby={labelId} aria-describedby={names().length === 0 && props.list === "columns" ? emptyId : undefined} class="flex flex-col gap-1">
      <span id={labelId} class="text-xs text-muted">
        {props.label}
      </span>
      <Show when={names().length > 0}>
        <ul role="list" aria-label={props.label} class="flex flex-wrap gap-1.5">
          <For each={[...names()]}>
            {(name, j) => {
              const errs = () => exactIssues(props.issues, `${root()}[${j()}]`);
              return (
                <li class="flex flex-col gap-0.5">
                  <span class="inline-flex h-6 items-center gap-1 rounded border border-border px-1.5 text-[12px]" classList={{ "!border-danger": errs().length > 0 }}>
                    <span class="font-mono">{name}</span>
                    <Show when={!ro()}>
                      <button
                        type="button"
                        class="rounded px-1 text-muted hover:bg-hover"
                        aria-label={t.removeColumn(name)}
                        title={t.removeColumn(name)}
                        onClick={() => {
                          const list = props.list;
                          const at = j();
                          props.onEdit((k) => removeConstraintColumn(k, list, at));
                        }}
                      >
                        <span aria-hidden="true">×</span>
                      </button>
                    </Show>
                  </span>
                  <Show when={errs().length > 0}>
                    <span class="text-xs text-danger">{errs().join(" ")}</span>
                  </Show>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
      <Show when={names().length === 0 && props.list === "columns"}>
        <span id={emptyId} class="text-xs text-muted">
          {t.noColumns}
        </span>
      </Show>
      <Show when={listErrors().length > 0}>
        <span class="text-xs text-danger">{listErrors().join(" ")}</span>
      </Show>
      <Show when={!ro()}>
        <div class="flex flex-wrap items-center gap-1.5">
          <select
            id={selectId}
            class={`${inputClass} !w-auto min-w-40`}
            aria-label={`${props.label}: ${t.chooseColumn}`}
            disabled={avail().length === 0}
            value={current()}
            onChange={(ev) => setPick(ev.currentTarget.value)}
          >
            <option value="">{t.chooseColumn}</option>
            <For each={avail()}>{(n) => <option value={n}>{n}</option>}</For>
          </select>
          <Button disabled={current() === ""} onClick={add} aria-label={`${props.label}: ${t.addColumn}`}>
            {t.addColumn}
          </Button>
        </div>
      </Show>
    </div>
  );
}

function CheckBody(props: CardProps & { c: ConstraintDef; onEdit: (f: (k: ConstraintDef) => ConstraintDef) => void }) {
  const t = ttbl.constraints;
  const i = props.index;
  const note = createUniqueId();
  const escape = createUniqueId();
  const k = () => (props.c.kind.type === "check" ? props.c.kind : { expr: "", noInherit: false });
  const ro = () => !props.editable;
  const blocked = () => noInheritBlocked(props.model.mode, props.draft, props.c);
  const noInheritHint = createUniqueId();
  return (
    <>
      <FieldRow label={t.expr} path={`constraints[${i}].expr`} issues={props.issues} exact>
        {(a) => (
          <>
            <textarea
              id={a.id}
              rows={2}
              class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(a.invalid)}`}
              spellcheck={false}
              placeholder={t.exprPlaceholder}
              aria-describedby={join(a.describedBy, note, escape)}
              aria-invalid={a.invalid}
              disabled={ro()}
              value={k().expr}
              onInput={(ev) => {
                const v = ev.currentTarget.value;
                props.onEdit((c) => (c.kind.type === "check" ? { ...c, kind: { ...c.kind, expr: v } } : c));
              }}
            />
            <FreeSqlNote id={note} />
            <p id={escape} class="text-xs text-muted">
              {ttbl.columns.escapeHint}
            </p>
          </>
        )}
      </FieldRow>
      <label class="inline-flex items-center gap-1.5 text-[13px]">
        <input
          type="checkbox"
          aria-describedby={blocked() ? noInheritHint : undefined}
          disabled={ro() || blocked()}
          checked={k().noInherit}
          onChange={(ev) => {
            const v = ev.currentTarget.checked;
            props.onEdit((c) => (c.kind.type === "check" ? { ...c, kind: { ...c.kind, noInherit: v } } : c));
          }}
        />
        {t.noInherit}
      </label>
      <Show when={blocked()}>
        <span id={noInheritHint} class="text-xs text-muted">
          {t.noInheritPartitioned}
        </span>
      </Show>
    </>
  );
}

function FkBody(
  props: CardProps & {
    c: ConstraintDef;
    onEdit: (f: (k: ConstraintDef) => ConstraintDef) => void;
    setTarget: (patch: { refSchema?: string; refTable?: string }) => void;
  },
) {
  const t = ttbl.constraints;
  const i = props.index;
  const p = (f: string) => `constraints[${i}].${f}`;
  const fk = () => (props.c.kind.type === "foreign_key" ? props.c.kind : null);
  const pairs = createMemo(() => (fk()?.columns ?? []).map((_, j) => j), undefined, { equals: (a, b) => a.length === b.length });
  // Kendine-başvuru kararı SelfSnap'ten (ad yayılımı ve sarkan referansla aynı kaynak); ad eşleşmesi yetmez.
  const selfRef = () => props.editor.snap().includes(i);
  const source = () => fkRefColumnSource(props.draft, props.c, selfRef());
  const nameOnly = () => fkSelfNameOnly(props.draft, props.c, selfRef());
  const ro = () => !props.editable;
  const refHintId = createUniqueId();
  const pairErrors = () => [
    ...exactIssues(props.issues, p("columns")),
    ...exactIssues(props.issues, p("refColumns")),
    ...orphanRowIssues(props.issues, p("columns"), pairs().length),
    ...orphanRowIssues(props.issues, p("refColumns"), pairs().length),
  ];
  const queryRefs = () => {
    const k = fk();
    if (k && source() === "lookup") props.lookups.queryRefColumns(k.refSchema, k.refTable);
  };

  return (
    <div class="flex flex-col gap-2">
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <FieldRow label={t.refSchema} path={p("refSchema")} issues={props.issues} exact>
          {(a) => (
            <input
              id={a.id}
              class={inputClass + invalidClass(a.invalid)}
              list={props.lookups.schemaList}
              autocomplete="off"
              spellcheck={false}
              required
              aria-required="true"
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={ro()}
              value={fk()?.refSchema ?? ""}
              onFocus={props.lookups.loadSchemas}
              onInput={(ev) => props.setTarget({ refSchema: ev.currentTarget.value })}
            />
          )}
        </FieldRow>
        <FieldRow label={t.refTable} path={p("refTable")} issues={props.issues} exact>
          {(a) => (
            <input
              id={a.id}
              class={inputClass + invalidClass(a.invalid)}
              list={props.lookups.tableList}
              autocomplete="off"
              spellcheck={false}
              required
              aria-required="true"
              placeholder={t.refTablePlaceholder}
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={ro()}
              value={fk()?.refTable ?? ""}
              onFocus={() => props.lookups.queryTables(fk()?.refSchema ?? "")}
              onInput={(ev) => {
                props.setTarget({ refTable: ev.currentTarget.value });
                props.lookups.queryTables(fk()?.refSchema ?? "");
              }}
            />
          )}
        </FieldRow>
      </div>
      <div role="group" aria-label={t.pairs} aria-describedby={source() !== "lookup" || nameOnly() ? refHintId : undefined} class="flex flex-col gap-1">
        <span class="text-xs text-muted" aria-hidden="true">
          {t.pairs}
        </span>
        <For each={pairs()}>
          {(j) => {
            const local = () => fk()?.columns[j] ?? "";
            const ref = () => fk()?.refColumns[j] ?? "";
            const localErr = () => exactIssues(props.issues, p(`columns[${j}]`));
            const refErr = () => exactIssues(props.issues, p(`refColumns[${j}]`));
            const localErrId = createUniqueId();
            const refErrId = createUniqueId();
            return (
              <fieldset class="flex min-w-0 flex-col gap-0.5">
                <legend class="sr-only">{t.pair(j)}</legend>
                <div class="flex flex-wrap items-center gap-1.5">
                  <select
                    class={`${inputClass} !w-auto min-w-36${invalidClass(localErr().length > 0)}`}
                    aria-label={`${t.pair(j)}: ${t.localColumn}`}
                    aria-describedby={localErr().length > 0 ? localErrId : undefined}
                    aria-invalid={localErr().length > 0}
                    disabled={ro()}
                    value={local()}
                    onChange={(ev) => {
                      const v = ev.currentTarget.value;
                      props.onEdit((k) => updateFkPair(k, j, { local: v }));
                    }}
                  >
                    <For each={columnChoices(props.draft, local())}>{(n) => <option value={n}>{n === "" ? t.chooseColumn : n}</option>}</For>
                  </select>
                  <span aria-hidden="true" class="text-muted">
                    →
                  </span>
                  <input
                    class={`${inputClass} !w-auto min-w-36 flex-1${invalidClass(refErr().length > 0)}`}
                    aria-label={`${t.pair(j)}: ${t.refColumn}`}
                    aria-describedby={join(refErr().length > 0 && refErrId, (source() !== "lookup" || nameOnly()) && refHintId)}
                    aria-invalid={refErr().length > 0}
                    list={source() === "draft" ? props.lookups.draftColList : source() === "lookup" ? props.lookups.refColList : undefined}
                    autocomplete="off"
                    spellcheck={false}
                    placeholder={t.refColumnPlaceholder}
                    disabled={ro()}
                    value={ref()}
                    onFocus={queryRefs}
                    onInput={(ev) => {
                      const v = ev.currentTarget.value;
                      props.onEdit((k) => updateFkPair(k, j, { ref: v }));
                      queryRefs();
                    }}
                  />
                  <Show when={props.editable && pairs().length > 1}>
                    <Button
                      variant="ghost"
                      aria-label={t.removePair(j)}
                      title={t.removePair(j)}
                      onClick={() => props.onEdit((k) => removeFkPair(k, j))}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                    </Button>
                  </Show>
                </div>
                <Show when={localErr().length > 0}>
                  <span id={localErrId} class="text-xs text-danger">
                    {localErr().join(" ")}
                  </span>
                </Show>
                <Show when={refErr().length > 0}>
                  <span id={refErrId} class="text-xs text-danger">
                    {refErr().join(" ")}
                  </span>
                </Show>
              </fieldset>
            );
          }}
        </For>
        <Show when={source() !== "lookup" || nameOnly()}>
          <span id={refHintId} class="text-xs text-muted">
            {source() === "draft" ? t.selfRefHint : source() === "none" ? t.noRefTable : t.selfRefNotRecorded}
          </span>
        </Show>
        <Show when={pairErrors().length > 0}>
          <span class="text-xs text-danger">{pairErrors().join(" ")}</span>
        </Show>
        <Show when={props.editable}>
          <div>
            <Button variant="ghost" class="!h-6 !px-1.5 text-xs" onClick={() => props.onEdit(addFkPair)}>
              {t.addPair}
            </Button>
          </div>
        </Show>
      </div>
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
        <FkSelect label={t.matchType} path={p("matchType")} issues={props.issues} editable={props.editable} value={fk()?.matchType ?? "simple"} options={MATCH} names={t.matchTypes}
          onChange={(v) => props.onEdit((k) => withFkActions(k, { matchType: v as FkMatch }))} />
        <FkSelect label={t.onUpdate} path={p("onUpdate")} issues={props.issues} editable={props.editable} value={fk()?.onUpdate ?? "no_action"} options={ACTIONS} names={t.actions}
          onChange={(v) => props.onEdit((k) => withFkActions(k, { onUpdate: v as FkAction }))} />
        <FkSelect label={t.onDelete} path={p("onDelete")} issues={props.issues} editable={props.editable} value={fk()?.onDelete ?? "no_action"} options={ACTIONS} names={t.actions}
          onChange={(v) => props.onEdit((k) => withFkActions(k, { onDelete: v as FkAction }))} />
      </div>
    </div>
  );
}

function FkSelect(props: {
  label: string;
  path: string;
  issues: TableSectionProps["issues"];
  editable: boolean;
  value: string;
  options: readonly string[];
  names: Record<string, string>;
  onChange: (v: string) => void;
}) {
  return (
    <FieldRow label={props.label} path={props.path} issues={props.issues} exact>
      {(a) => (
        <select
          id={a.id}
          class={inputClass + invalidClass(a.invalid)}
          aria-describedby={a.describedBy}
          aria-invalid={a.invalid}
          disabled={!props.editable}
          value={props.value}
          onChange={(ev) => props.onChange(ev.currentTarget.value)}
        >
          <For each={props.options}>{(o) => <option value={o}>{props.names[o]}</option>}</For>
        </select>
      )}
    </FieldRow>
  );
}
