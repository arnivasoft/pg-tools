// Index formu (tembel parça): şema + tablo (salt okunur), ad (alter'da yeniden adlandırma), yöntem, UNIQUE, anahtar listesi
// (sütun/ifade, opclass, collation, DESC, NULLS — son ikisi yalnız btree), INCLUDE, WHERE, tablespace, fillfactor, yorum;
// create'te CONCURRENTLY. Kısıta ait indekste form tümüyle salt okunur; geçersiz indekste uyarı şeridi. Satırlar sıra
// numarası listesiyle çizilir (her düzenleme taslağı kopyalar; nesneye göre anahtarlı `For` satırları yeniden kurup odağı
// kaybettirirdi).
import { ArrowDown, ArrowUp, Trash2 } from "lucide-solid";
import { For, Show, createMemo, createResource, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { IndexKey } from "../../ipc/bindings/IndexKey";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { addColumn, addInclude, moveColumn, removeColumn, removeInclude, updateColumn, type FieldIssues } from "./designer-model";
import { CommentField, FieldRow, FreeSqlNote, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import {
  asIndex, btreeOnlyFields, columnKeyLabel, exactIssues, fillfactorEnabled, fillfactorFromInput, indexEditPatch,
  indexMethodPatch, indexReadOnlyReason, newIndexColumn, normalizeEmpty, normalizeTablespaceEdit, nullsFirstToSelect,
  orphanRowIssues, selectToNullsFirst, structuralChange, switchKeyKind, updateInclude,
} from "./forms-model";

/** Lookup başarısızsa yöntem seçicisi yine dolu olsun (arka ucun sabit kümesiyle aynı sıra). */
const FALLBACK_METHODS = ["btree", "hash", "gin", "gist", "brin", "spgist"];

/** Onay kutusu + (varsa) alan hatası; hata metni `aria-describedby` ile bağlı. */
function CheckField(props: {
  label: string;
  path: string;
  issues: FieldIssues;
  hint?: string;
  checked: boolean;
  disabled: boolean;
  onChange: (v: boolean) => void;
}) {
  const id = createUniqueId();
  const errors = () => exactIssues(props.issues, props.path);
  const describedBy = () =>
    [errors().length > 0 ? `${id}-err` : null, props.hint ? `${id}-hint` : null].filter((x) => x !== null).join(" ") || undefined;
  return (
    <div class="flex flex-col gap-1">
      <label class="flex items-center gap-2 text-[13px]">
        <input
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
      <Show when={props.hint}>
        <span id={`${id}-hint`} class="text-xs text-muted">
          {props.hint}
        </span>
      </Show>
    </div>
  );
}

/** Liste düzeyindeki hatalar (tam yol + artık var olmayan satırlar). */
function ListErrors(props: { messages: string[] }) {
  return (
    <Show when={props.messages.length > 0}>
      <span class="text-xs text-danger">{props.messages.join(" ")}</span>
    </Show>
  );
}

export default function IndexForm(props: FormProps) {
  const f = tdes.index;
  const draft = () => asIndex(props.model.draft);
  const original = () => asIndex(props.model.original);
  const alter = () => props.model.mode === "alter";
  const reason = createMemo(() => indexReadOnlyReason(original() ?? draft()));
  // Kısıta ait indeks: arka uç yalnız tablespace/fillfactor/yorumu kabul eder; plan gereği form tümüyle salt okunur.
  const ro = () => props.readOnly || reason().constraint !== null;
  const edit = (fn: (d: IndexDef) => IndexDef) =>
    props.designer.edit(props.tabId, (d) => (d.kind === "index" ? { ...fn(d), kind: "index" } : d));
  const patch = (p: Partial<IndexDef>) => props.designer.edit(props.tabId, (d) => indexEditPatch(d, p));
  const columnList = createUniqueId();
  const whereNote = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  // Satır denetimlerine odak taşıma (ekle/taşı/sil sonrası klavye kullanıcısı yerini kaybetmesin).
  const keyEls: Array<HTMLElement | undefined> = [];
  const upEls: Array<HTMLButtonElement | undefined> = [];
  const downEls: Array<HTMLButtonElement | undefined> = [];
  const includeEls: Array<HTMLInputElement | undefined> = [];
  let addColumnEl: HTMLButtonElement | undefined;
  let addIncludeEl: HTMLButtonElement | undefined;
  const focusLater = (el: () => HTMLElement | undefined) => queueMicrotask(() => el()?.focus());

  // Lookup hataları listeyi boş bırakır; serbest yazım sürer.
  const [methods] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "index_methods" }).catch(() => []),
  );
  const [tablespaces] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "tablespaces" }).catch(() => []),
  );
  const tableKey = createMemo(() => {
    const d = draft();
    return d && d.schema !== "" && d.table !== "" ? `${d.schema}\u0000${d.table}` : null;
  });
  const [tableColumns] = createResource(tableKey, (key) => {
    const [schema = "", table = ""] = key.split("\u0000");
    return props.designer.lookup(props.tabId, { kind: "table_columns", schema, table }).catch(() => []);
  });

  const methodOptions = (current: string) => {
    const list = (methods() ?? []).map((m) => m.value);
    const base = list.length > 0 ? list : FALLBACK_METHODS;
    return base.includes(current) ? base : [current, ...base];
  };
  const tablespaceOptions = (current: string | null) => {
    const list = (tablespaces() ?? []).map((t) => t.value).filter((v) => v !== "pg_default");
    if (current !== null && current !== "pg_default" && !list.includes(current)) list.unshift(current);
    return list;
  };

  return (
    <Show when={draft()}>
      {(d) => {
        const btree = () => btreeOnlyFields(d().method);
        // Sıra numaraları (sayı anahtarı: satır bileşeni düzenlemede korunur, yalnızca uzunluk değişince eklenir/kalkar).
        const columnRows = createMemo(() => d().columns.map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length });
        const includeRows = createMemo(() => d().include.map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length });
        const rebuild = () => {
          const o = original();
          return alter() && o !== null && structuralChange(o, d());
        };
        const editCol = (i: number, p: Partial<IndexColumn>) => edit((x) => updateColumn(x, i, p));
        const move = (from: number, to: number, which: "up" | "down") => {
          edit((x) => moveColumn(x, from, to));
          // Taşınan satırın aynı düğmesine odak; sınırda diğer yön düğmesi.
          const last = d().columns.length - 1;
          focusLater(() =>
            which === "up" ? (to > 0 ? upEls[to] : downEls[to]) : to < last ? downEls[to] : upEls[to],
          );
        };
        const remove = (i: number) => {
          edit((x) => removeColumn(x, i));
          focusLater(() => (d().columns.length === 0 ? addColumnEl : keyEls[Math.min(i, d().columns.length - 1)]));
        };
        return (
          <form class="flex max-w-2xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
            <Show when={reason().constraint}>
              {(text) => (
                <div class="rounded border border-border px-3 py-2 text-[13px]">
                  <p class="font-medium text-warn-text">{text()}</p>
                  <ListErrors messages={exactIssues(props.issues, "constraint")} />
                </div>
              )}
            </Show>
            <Show when={!reason().constraint && exactIssues(props.issues, "constraint").length > 0}>
              <ListErrors messages={exactIssues(props.issues, "constraint")} />
            </Show>
            <Show when={reason().invalid}>
              {(text) => <div class="rounded border border-danger px-3 py-2 text-[13px] text-danger">{text()}</div>}
            </Show>
            <Show when={rebuild()}>
              <div role="note" class="rounded border border-border px-3 py-2 text-[13px] text-warn-text">
                {f.structural}
              </div>
            </Show>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <FieldRow label={f.schema} path="schema" issues={props.issues}>
                {(a) => (
                  <input id={a.id} class={inputClass} aria-describedby={a.describedBy} readOnly value={d().schema} />
                )}
              </FieldRow>
              <FieldRow label={f.table} path="table" issues={props.issues} hint={f.tableHint}>
                {(a) => (
                  <input id={a.id} class={inputClass} aria-describedby={a.describedBy} readOnly value={d().table} />
                )}
              </FieldRow>
            </div>
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
                  disabled={ro()}
                  value={d().name}
                  onInput={(ev) => patch({ name: ev.currentTarget.value })}
                />
              )}
            </FieldRow>
            <FieldRow label={f.method} path="method" issues={props.issues} hint={f.methodHint}>
              {(a) => (
                <select
                  id={a.id}
                  class={inputClass + invalidClass(a.invalid)}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={ro()}
                  value={d().method}
                  onChange={(ev) => {
                    const m = ev.currentTarget.value;
                    edit((x) => indexMethodPatch(x, m));
                  }}
                >
                  <For each={methodOptions(d().method)}>{(m) => <option value={m}>{m}</option>}</For>
                </select>
              )}
            </FieldRow>
            <CheckField
              label={f.unique}
              path="unique"
              issues={props.issues}
              hint={f.uniqueHint}
              checked={d().unique}
              disabled={ro()}
              onChange={(v) => patch({ unique: v })}
            />

            <div role="group" aria-label={f.columns} class="flex flex-col gap-2">
              <span class="text-xs text-muted">{f.columns}</span>
              <span class="text-xs text-muted">{f.columnsHint}</span>
              <Show when={d().columns.length === 0}>
                <p class="text-xs text-muted">{f.noColumns}</p>
              </Show>
              <For each={columnRows()}>
                {(i) => {
                  // Satır silinirken kapanmadan önce bir kez daha okunabilir: boş satırla korunur.
                  const c = () => d().columns[i] ?? newIndexColumn();
                  const label = () => columnKeyLabel(c().key);
                  return (
                    <div role="group" aria-label={f.row(i)} class="flex flex-col gap-2 rounded border border-border p-2">
                      <div class="flex items-center gap-1">
                        <span class="text-xs font-medium">{f.row(i)}</span>
                        <div class="ml-auto flex items-center gap-1">
                          <Button
                            variant="ghost"
                            ref={(el: HTMLButtonElement) => (upEls[i] = el)}
                            aria-label={f.moveUp(i, label())}
                            title={f.moveUp(i, label())}
                            disabled={ro() || i === 0}
                            onClick={() => move(i, i - 1, "up")}
                          >
                            <ArrowUp size={14} aria-hidden="true" />
                          </Button>
                          <Button
                            variant="ghost"
                            ref={(el: HTMLButtonElement) => (downEls[i] = el)}
                            aria-label={f.moveDown(i, label())}
                            title={f.moveDown(i, label())}
                            disabled={ro() || i === d().columns.length - 1}
                            onClick={() => move(i, i + 1, "down")}
                          >
                            <ArrowDown size={14} aria-hidden="true" />
                          </Button>
                          <Button
                            variant="ghost"
                            aria-label={f.removeColumn(i, label())}
                            title={f.removeColumn(i, label())}
                            disabled={ro()}
                            onClick={() => remove(i)}
                          >
                            <Trash2 size={14} aria-hidden="true" />
                          </Button>
                        </div>
                      </div>
                      <ListErrors messages={exactIssues(props.issues, `columns[${i}]`)} />
                      <div class="grid grid-cols-1 gap-2 sm:grid-cols-[9rem_1fr]">
                        <FieldRow label={f.keyKind} path={`columns[${i}].kind`} issues={props.issues}>
                          {(a) => (
                            <select
                              id={a.id}
                              class={inputClass}
                              aria-describedby={a.describedBy}
                              disabled={ro()}
                              value={c().key.kind}
                              onChange={(ev) => {
                                const kind = ev.currentTarget.value as IndexKey["kind"];
                                editCol(i, { key: switchKeyKind(c().key, kind) });
                              }}
                            >
                              <option value="column">{f.keyColumn}</option>
                              <option value="expression">{f.keyExpression}</option>
                            </select>
                          )}
                        </FieldRow>
                        <KeyField
                          index={i}
                          keyValue={c().key}
                          issues={props.issues}
                          listId={columnList}
                          disabled={ro()}
                          ref={(el) => (keyEls[i] = el)}
                          onInput={(key) => editCol(i, { key })}
                        />
                      </div>
                      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                        <FieldRow label={f.opclass} path={`columns[${i}].opclass`} issues={props.issues}>
                          {(a) => (
                            <input
                              id={a.id}
                              class={inputClass + invalidClass(a.invalid)}
                              autocomplete="off"
                              spellcheck={false}
                              aria-describedby={a.describedBy}
                              aria-invalid={a.invalid}
                              disabled={ro()}
                              value={c().opclass ?? ""}
                              onInput={(ev) => editCol(i, { opclass: normalizeEmpty(ev.currentTarget.value) })}
                            />
                          )}
                        </FieldRow>
                        <FieldRow label={f.collation} path={`columns[${i}].collation`} issues={props.issues}>
                          {(a) => (
                            <input
                              id={a.id}
                              class={inputClass + invalidClass(a.invalid)}
                              autocomplete="off"
                              spellcheck={false}
                              aria-describedby={a.describedBy}
                              aria-invalid={a.invalid}
                              disabled={ro()}
                              value={c().collation ?? ""}
                              onInput={(ev) => editCol(i, { collation: normalizeEmpty(ev.currentTarget.value) })}
                            />
                          )}
                        </FieldRow>
                      </div>
                      <div class="grid grid-cols-1 items-end gap-2 sm:grid-cols-2">
                        <label class="flex h-7 items-center gap-2 text-[13px]" title={btree() ? undefined : f.btreeOnly}>
                          <input
                            type="checkbox"
                            disabled={ro() || !btree()}
                            checked={c().descending}
                            onChange={(ev) => editCol(i, { descending: ev.currentTarget.checked })}
                          />
                          {f.descending}
                        </label>
                        <FieldRow label={f.nullsOrder} path={`columns[${i}].nulls`} issues={props.issues} hint={btree() ? undefined : f.btreeOnly}>
                          {(a) => (
                            <select
                              id={a.id}
                              class={inputClass}
                              aria-describedby={a.describedBy}
                              disabled={ro() || !btree()}
                              value={nullsFirstToSelect(c().nullsFirst)}
                              onChange={(ev) => editCol(i, { nullsFirst: selectToNullsFirst(ev.currentTarget.value) })}
                            >
                              <option value="default">{f.nulls.default}</option>
                              <option value="first">{f.nulls.first}</option>
                              <option value="last">{f.nulls.last}</option>
                            </select>
                          )}
                        </FieldRow>
                      </div>
                    </div>
                  );
                }}
              </For>
              <ListErrors messages={orphanRowIssues(props.issues, "columns", d().columns.length)} />
              <div>
                <Button
                  ref={(el: HTMLButtonElement) => (addColumnEl = el)}
                  disabled={ro()}
                  onClick={() => {
                    edit((x) => addColumn(x, newIndexColumn()));
                    focusLater(() => keyEls[d().columns.length - 1]);
                  }}
                >
                  {f.addColumn}
                </Button>
              </div>
            </div>

            <div role="group" aria-label={f.include} class="flex flex-col gap-2">
              <span class="text-xs text-muted">{f.include}</span>
              <ListErrors messages={[...exactIssues(props.issues, "include"), ...orphanRowIssues(props.issues, "include", d().include.length)]} />
              <For each={includeRows()}>
                {(i) => {
                  const name = () => d().include[i] ?? "";
                  return (
                    <FieldRow label={f.includeItem(i)} path={`include[${i}]`} issues={props.issues}>
                      {(a) => (
                        <div class="flex items-center gap-1">
                          <input
                            id={a.id}
                            ref={(el) => (includeEls[i] = el)}
                            class={inputClass + invalidClass(a.invalid)}
                            list={columnList}
                            autocomplete="off"
                            spellcheck={false}
                            aria-describedby={a.describedBy}
                            aria-invalid={a.invalid}
                            disabled={ro()}
                            value={name()}
                            onInput={(ev) => {
                              const v = ev.currentTarget.value;
                              edit((x) => updateInclude(x, i, v));
                            }}
                          />
                          <Button
                            variant="ghost"
                            aria-label={f.removeInclude(i, name() || f.empty)}
                            title={f.removeInclude(i, name() || f.empty)}
                            disabled={ro()}
                            onClick={() => {
                              edit((x) => removeInclude(x, i));
                              focusLater(() => (d().include.length === 0 ? addIncludeEl : includeEls[Math.min(i, d().include.length - 1)]));
                            }}
                          >
                            <Trash2 size={14} aria-hidden="true" />
                          </Button>
                        </div>
                      )}
                    </FieldRow>
                  );
                }}
              </For>
              <span class="text-xs text-muted">{f.includeHint}</span>
              <div>
                <Button
                  ref={(el: HTMLButtonElement) => (addIncludeEl = el)}
                  disabled={ro()}
                  onClick={() => {
                    edit((x) => addInclude(x, ""));
                    focusLater(() => includeEls[d().include.length - 1]);
                  }}
                >
                  {f.addInclude}
                </Button>
              </div>
            </div>
            <datalist id={columnList}>
              <For each={tableColumns.loading ? [] : (tableColumns() ?? [])}>
                {(i) => <option value={i.value} label={i.detail ?? undefined} />}
              </For>
            </datalist>

            <FieldRow label={f.where} path="whereClause" issues={props.issues} hint={f.whereHint}>
              {(a) => (
                <>
                  <textarea
                    id={a.id}
                    rows={2}
                    class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(a.invalid)}`}
                    spellcheck={false}
                    placeholder={f.wherePlaceholder}
                    aria-describedby={[a.describedBy, whereNote].filter(Boolean).join(" ")}
                    aria-invalid={a.invalid}
                    disabled={ro()}
                    value={d().whereClause ?? ""}
                    onInput={(ev) => patch({ whereClause: normalizeEmpty(ev.currentTarget.value) })}
                  />
                  <FreeSqlNote id={whereNote} />
                </>
              )}
            </FieldRow>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <FieldRow label={f.tablespace} path="tablespace" issues={props.issues} hint={f.tablespaceHint}>
                {(a) => (
                  <select
                    id={a.id}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={ro()}
                    value={d().tablespace ?? ""}
                    onChange={(ev) => {
                      const v = ev.currentTarget.value;
                      patch({ tablespace: normalizeTablespaceEdit(original()?.tablespace ?? null, v) });
                    }}
                  >
                    {/* Temizleme yok: boş seçenek yalnızca yüklenen tanımda tablespace yoksa. */}
                    <Show when={(original()?.tablespace ?? null) === null}>
                      <option value="">{f.tablespaceUnset}</option>
                    </Show>
                    <option value="pg_default">{f.tablespaceDefault}</option>
                    <For each={tablespaceOptions(d().tablespace)}>{(t) => <option value={t}>{t}</option>}</For>
                  </select>
                )}
              </FieldRow>
              <FieldRow
                label={f.fillfactor}
                path="fillfactor"
                issues={props.issues}
                hint={fillfactorEnabled(d().method) ? f.fillfactorHint : f.fillfactorDisabled(d().method)}
              >
                {(a) => (
                  <input
                    id={a.id}
                    type="number"
                    min={10}
                    max={100}
                    step={1}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={ro() || !fillfactorEnabled(d().method)}
                    value={d().fillfactor ?? ""}
                    onInput={(ev) => patch({ fillfactor: fillfactorFromInput(ev.currentTarget.value) })}
                  />
                )}
              </FieldRow>
            </div>
            <CommentField
              value={d().comment}
              issues={props.issues}
              disabled={ro()}
              onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
            />
            <Show when={!alter()}>
              <CheckField
                label={f.concurrently}
                path="concurrently"
                issues={props.issues}
                hint={f.concurrentlyNote}
                checked={props.model.concurrently}
                disabled={ro()}
                onChange={(v) => props.designer.setConcurrently(props.tabId, v)}
              />
            </Show>
          </form>
        );
      }}
    </Show>
  );
}

/** Anahtar: sütun adı (tablo kolonları önerisi) ya da serbest ifade (+ serbest SQL notu). */
function KeyField(props: {
  index: number;
  keyValue: IndexKey;
  issues: FieldIssues;
  listId: string;
  disabled: boolean;
  ref: (el: HTMLElement) => void;
  onInput: (key: IndexKey) => void;
}) {
  const f = tdes.index;
  const noteId = createUniqueId();
  return (
    <FieldRow
      label={props.keyValue.kind === "column" ? f.columnName : f.expression}
      path={`columns[${props.index}].key`}
      issues={props.issues}
    >
      {(a) => (
        <Show
          when={props.keyValue.kind === "expression" ? props.keyValue : null}
          fallback={
            <input
              id={a.id}
              ref={props.ref}
              class={inputClass + invalidClass(a.invalid)}
              list={props.listId}
              autocomplete="off"
              spellcheck={false}
              required
              aria-required="true"
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={props.disabled}
              value={props.keyValue.kind === "column" ? props.keyValue.name : ""}
              onInput={(ev) => props.onInput({ kind: "column", name: ev.currentTarget.value })}
            />
          }
        >
          {(e) => (
            <>
              <textarea
                id={a.id}
                ref={props.ref}
                rows={2}
                class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(a.invalid)}`}
                spellcheck={false}
                required
                aria-required="true"
                placeholder={f.expressionPlaceholder}
                aria-describedby={[a.describedBy, noteId].filter(Boolean).join(" ")}
                aria-invalid={a.invalid}
                disabled={props.disabled}
                value={e().sql}
                onInput={(ev) => props.onInput({ kind: "expression", sql: ev.currentTarget.value })}
              />
              <FreeSqlNote id={noteId} />
            </>
          )}
        </Show>
      )}
    </FieldRow>
  );
}
