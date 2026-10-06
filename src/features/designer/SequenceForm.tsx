// Sequence formu (tembel parça): şema (alter'da SET SCHEMA), ad (alter'da yeniden adlandırma), sahip, veri türü, sayı
// parametreleri (metin; yalnızca biçim ön uyarısı, asıl doğrulama arka uçta), CYCLE, OWNED BY ve yorum.
import { For, Show, createMemo, createResource, createUniqueId, type JSX } from "solid-js";
import { tdes } from "../../app/strings-designer";
import type { SeqType } from "../../ipc/bindings/SeqType";
import type { SequenceDef } from "../../ipc/bindings/SequenceDef";
import { inputClass } from "../../ui/Field";
import type { FieldIssues } from "./designer-model";
import { CommentField, FieldRow, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import {
  asSequence, incrementEditPatch, incrementSign, normalizeEmpty, ownedByHint, seqTypeBoundsNote, sequenceDirectionChanged,
  sequenceEditPatch, sequenceFormatWarnings, sequenceStartChanged, sequenceTypeChanged, type SequenceNumberField,
} from "./forms-model";

const SEQ_TYPES: SeqType[] = ["smallint", "integer", "bigint"];

/** Sayı parametresi: metin girdisi (`inputmode="numeric"`); `nullable` alanda boş = `null` (NO MINVALUE/MAXVALUE). */
function NumberField(props: {
  label: string;
  path: SequenceNumberField;
  value: string | null;
  issues: FieldIssues;
  warning?: string;
  hint?: string;
  nullable?: boolean;
  disabled: boolean;
  onInput: (v: string | null) => void;
}) {
  return (
    <FieldRow label={props.label} path={props.path} issues={props.issues} warning={props.warning} hint={props.hint}>
      {(a) => (
        <input
          id={a.id}
          class={inputClass + invalidClass(a.invalid) + " font-mono"}
          inputmode="numeric"
          autocomplete="off"
          spellcheck={false}
          required={!props.nullable}
          aria-required={props.nullable ? undefined : "true"}
          aria-describedby={a.describedBy}
          aria-invalid={a.invalid}
          disabled={props.disabled}
          value={props.value ?? ""}
          onInput={(ev) => {
            const v = ev.currentTarget.value;
            props.onInput(props.nullable ? (v.trim() === "" ? null : v) : v);
          }}
        />
      )}
    </FieldRow>
  );
}

export default function SequenceForm(props: FormProps) {
  const f = tdes.sequence;
  const draft = () => asSequence(props.model.draft);
  const alter = () => props.model.mode === "alter";
  const patch = (p: Partial<SequenceDef>) => props.designer.edit(props.tabId, (d) => sequenceEditPatch(d, p));
  const schemaList = createUniqueId();
  const ownedSchemaList = createUniqueId();
  const tableList = createUniqueId();
  const columnList = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  // Yön değişimi: yazım ara durumunda ("-") son geçerli yön hatırlanır (yalnızca bu form örneğinde).
  let lastSign: 1 | -1 = 1;
  const editIncrement = (next: string) =>
    props.designer.edit(props.tabId, (d) => {
      if (d.kind !== "sequence") return d;
      const out = incrementEditPatch(d, next, lastSign);
      lastSign = incrementSign(next) ?? incrementSign(d.increment) ?? lastSign;
      return { ...out, kind: "sequence" };
    });

  // Lookup hataları listeyi boş bırakır; serbest yazım sürer.
  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  const ownedSchema = createMemo(() => draft()?.ownedBy?.schema ?? "");
  const [tables] = createResource(
    () => (ownedSchema() === "" ? null : ownedSchema()),
    (schema) => props.designer.lookup(props.tabId, { kind: "tables", schema }).catch(() => []),
  );
  // Kolon listesi yalnızca yazılan tablo listede varsa (yarım adlarla sorgu yok); anahtar "şema\0tablo".
  const columnsKey = createMemo(() => {
    const o = draft()?.ownedBy;
    if (!o || o.table === "" || !(tables() ?? []).some((t) => t.value === o.table)) return null;
    return `${o.schema}\u0000${o.table}`;
  });
  const [columns] = createResource(columnsKey, (key) => {
    const [schema = "", table = ""] = key.split("\u0000");
    return props.designer.lookup(props.tabId, { kind: "table_columns", schema, table }).catch(() => []);
  });

  const warnings = createMemo(() => {
    const d = draft();
    return d ? sequenceFormatWarnings(d) : {};
  });

  return (
    <Show when={draft()}>
      {(d) => {
        const notes = () => {
          const out: string[] = [];
          if (sequenceDirectionChanged(props.model.original, d())) out.push(f.directionNote);
          if (sequenceStartChanged(props.model.original, d())) out.push(f.startAlterNote);
          return out;
        };
        const editOwned = (p: Partial<NonNullable<SequenceDef["ownedBy"]>>) => {
          const o = d().ownedBy;
          if (o) patch({ ownedBy: { ...o, ...p } });
        };
        return (
          <form class="flex max-w-xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
            <FieldRow label={f.schema} path="schema" issues={props.issues} hint={alter() ? f.schemaAlterHint : f.schemaCreateHint}>
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
                    disabled={props.readOnly}
                    value={d().schema}
                    onInput={(ev) => patch({ schema: ev.currentTarget.value })}
                  />
                  <datalist id={schemaList}>
                    <For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>
                  </datalist>
                </>
              )}
            </FieldRow>
            <FieldRow label={alter() ? f.rename : f.name} path="name" issues={props.issues}>
              {(a) => (
                <input
                  id={a.id}
                  ref={nameEl}
                  class={inputClass + invalidClass(a.invalid)}
                  autocomplete="off"
                  spellcheck={false}
                  required
                  aria-required="true"
                  placeholder={f.namePlaceholder}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
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
              disabled={props.readOnly}
              onChange={(v) => patch({ owner: v })}
            />
            <FieldRow
              label={f.dataType}
              path="dataType"
              issues={props.issues}
              warning={sequenceTypeChanged(props.model.original, d()) ? f.dataTypeChanged : null}
              hint={seqTypeBoundsNote(d().dataType)}
            >
              {(a) => (
                <select
                  id={a.id}
                  class={inputClass + invalidClass(a.invalid)}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
                  value={d().dataType}
                  onChange={(ev) => patch({ dataType: ev.currentTarget.value as SeqType })}
                >
                  <For each={SEQ_TYPES}>{(t) => <option value={t}>{t}</option>}</For>
                </select>
              )}
            </FieldRow>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <NumberField
                label={f.increment}
                path="increment"
                value={d().increment}
                issues={props.issues}
                warning={warnings().increment}
                hint={f.incrementHint}
                disabled={props.readOnly}
                onInput={(v) => editIncrement(v ?? "")}
              />
              <NumberField
                label={f.start}
                path="start"
                value={d().start}
                issues={props.issues}
                warning={warnings().start}
                disabled={props.readOnly}
                onInput={(v) => patch({ start: v ?? "" })}
              />
              <NumberField
                label={f.minValue}
                path="minValue"
                value={d().minValue}
                issues={props.issues}
                warning={warnings().minValue}
                hint={f.boundHint}
                nullable
                disabled={props.readOnly}
                onInput={(v) => patch({ minValue: v })}
              />
              <NumberField
                label={f.maxValue}
                path="maxValue"
                value={d().maxValue}
                issues={props.issues}
                warning={warnings().maxValue}
                hint={f.boundHint}
                nullable
                disabled={props.readOnly}
                onInput={(v) => patch({ maxValue: v })}
              />
              <NumberField
                label={f.cache}
                path="cache"
                value={d().cache}
                issues={props.issues}
                warning={warnings().cache}
                disabled={props.readOnly}
                onInput={(v) => patch({ cache: v ?? "" })}
              />
            </div>
            <Show when={notes().length > 0}>
              <div role="note" class="rounded border border-border px-3 py-2 text-xs text-warn-text">
                <For each={notes()}>{(n) => <p>{n}</p>}</For>
              </div>
            </Show>
            <label class="flex items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                disabled={props.readOnly}
                checked={d().cycle}
                onChange={(ev) => patch({ cycle: ev.currentTarget.checked })}
              />
              {f.cycle}
            </label>
            <div role="group" aria-label={f.ownedBy} class="flex flex-col gap-3 rounded border border-border p-3">
              <FieldRow label={f.ownedBy} path="ownedBy" exact issues={props.issues} hint={f.ownedByHint}>
                {(a) => (
                  <select
                    id={a.id}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={props.readOnly}
                    value={d().ownedBy ? "column" : "none"}
                    onChange={(ev) =>
                      patch({ ownedBy: ev.currentTarget.value === "column" ? { schema: d().schema, table: "", column: "" } : null })
                    }
                  >
                    <option value="none">{f.ownedByNone}</option>
                    <option value="column">{f.ownedByColumn}</option>
                  </select>
                )}
              </FieldRow>
              <Show when={d().ownedBy}>
                {(o) => (
                  <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
                    <OwnedPart
                      label={f.ownedBySchema}
                      path="ownedBy.schema"
                      issues={props.issues}
                      warning={ownedByHint(d())}
                      listId={ownedSchemaList}
                      value={o().schema}
                      disabled={props.readOnly}
                      onInput={(v) => editOwned({ schema: v })}
                      options={<For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>}
                    />
                    <OwnedPart
                      label={f.ownedByTable}
                      path="ownedBy.table"
                      issues={props.issues}
                      listId={tableList}
                      value={o().table}
                      disabled={props.readOnly}
                      onInput={(v) => editOwned({ table: v })}
                      options={<For each={tables.loading ? [] : (tables() ?? [])}>{(i) => <option value={i.value} />}</For>}
                    />
                    <OwnedPart
                      label={f.ownedByColumnName}
                      path="ownedBy.column"
                      issues={props.issues}
                      listId={columnList}
                      value={o().column}
                      disabled={props.readOnly}
                      onInput={(v) => editOwned({ column: v })}
                      options={
                        <For each={columnsKey() === null || columns.loading ? [] : (columns() ?? [])}>
                          {(i) => <option value={i.value} label={i.detail ?? undefined} />}
                        </For>
                      }
                    />
                  </div>
                )}
              </Show>
            </div>
            <CommentField
              value={d().comment}
              issues={props.issues}
              disabled={props.readOnly}
              onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
            />
          </form>
        );
      }}
    </Show>
  );
}

/** OWNED BY parçası: yazılabilir metin + öneri listesi (`datalist`). */
function OwnedPart(props: {
  label: string;
  path: string;
  issues: FieldIssues;
  warning?: string | null;
  listId: string;
  value: string;
  disabled: boolean;
  onInput: (v: string) => void;
  options: JSX.Element;
}) {
  return (
    <FieldRow label={props.label} path={props.path} issues={props.issues} warning={props.warning}>
      {(a) => (
        <>
          <input
            id={a.id}
            class={inputClass + invalidClass(a.invalid)}
            list={props.listId}
            autocomplete="off"
            spellcheck={false}
            required
            aria-required="true"
            aria-describedby={a.describedBy}
            aria-invalid={a.invalid}
            disabled={props.disabled}
            value={props.value}
            onInput={(ev) => props.onInput(ev.currentTarget.value)}
          />
          <datalist id={props.listId}>{props.options}</datalist>
        </>
      )}
    </FieldRow>
  );
}
