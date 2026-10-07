// Table formu Genel ve Bölümleme bölümleri (TableForm parçası). Genel: şema, ad (alter'da yeniden adlandırma), sahip,
// kalıcılık, tablespace, fillfactor, yorum; alter'da salt okunur bölüm bilgisi ve bölüm/kalıtım rozetleri. Bölümleme
// yalnızca create: strateji + anahtar listesi (kolon/ifade). Tablo adı/şeması düzenlemesi kendine-başvuran FK'lere
// anlık görüntüyle (editSnap) yayılır. İstemci SQL üretmez.
import { Trash2 } from "lucide-solid";
import { For, Show, createMemo, createResource, createUniqueId } from "solid-js";
import { ttbl } from "../../app/strings-table";
import type { PartitionKey } from "../../ipc/bindings/PartitionKey";
import type { PartitionStrategy } from "../../ipc/bindings/PartitionStrategy";
import type { Persistence } from "../../ipc/bindings/Persistence";
import type { TableDef } from "../../ipc/bindings/TableDef";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { CommentField, FieldRow, FreeSqlNote, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, fillfactorFromInput, normalizeEmpty, normalizeTablespaceEdit, orphanRowIssues, tablespaceChoices } from "./forms-model";
import type { TableSectionProps } from "./table-form-state";
import {
  addPartitionKey, partitionColumnChoices, partitionKeyLabel, removePartitionKey, renameTable, storageGate, structureLocked,
  switchPartitionKeyKind, updatePartitionKey, withPartitionStrategy, type TableDraft,
} from "./table-model";

/** Liste düzeyindeki hatalar (tam yol + artık var olmayan satırlar). */
function ListErrors(props: { messages: string[] }) {
  return (
    <Show when={props.messages.length > 0}>
      <span class="text-xs text-danger">{props.messages.join(" ")}</span>
    </Show>
  );
}

export function TableGeneral(props: TableSectionProps) {
  const g = ttbl.general;
  const alter = () => props.model.mode === "alter";
  const patch = (p: Partial<TableDef>) => props.editor.edit((d) => ({ ...d, ...p }));
  const renamePatch = (p: { name?: string; schema?: string }) => props.editor.editSnap((d, snap) => ({ d: renameTable(d, p, snap), snap }));
  const schemaList = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  // Gizli panelde odaklanılamaz: istek bölüm etkinleşince uygulanır (dağıtıcı Genel'i seçer).
  createFocusOnRequest(props, "name", () => (props.active ? nameEl : undefined));

  // Lookup hataları listeyi boş bırakır; serbest yazım sürer.
  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  const [tablespaces] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "tablespaces" }).catch(() => []),
  );
  const d = () => props.draft;
  const originalTablespace = () => props.original?.tablespace ?? null;
  const persistenceChanged = () => alter() && props.original !== null && props.original.persistence !== d().persistence;
  // Bölümlenmiş tablo: her zaman reddedilen depolama denetimleri kilitli, sürüme bağlı olanlar ipucu.
  const gate = createMemo(() => storageGate(props.model.mode, props.original, d()));

  return (
    <form class="flex max-w-2xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
      <Show when={structureLocked(props.original)}>
        <div role="note" class="rounded border border-border px-3 py-2 text-[13px] text-warn-text">
          {g.structureLocked}
        </div>
      </Show>
      <Show when={props.original && (props.original.isPartition || props.original.inherits)}>
        <div class="flex flex-wrap gap-1.5">
          <Show when={props.original?.isPartition}>
            <span class="inline-flex h-5 items-center rounded border border-border px-1.5 text-[11px]">{g.partitionBadge}</span>
          </Show>
          <Show when={props.original?.inherits}>
            <span class="inline-flex h-5 items-center rounded border border-border px-1.5 text-[11px]">{g.inheritsBadge}</span>
          </Show>
        </div>
      </Show>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <FieldRow label={g.schema} path="schema" issues={props.issues} hint={alter() ? g.schemaAlterHint : g.schemaCreateHint}>
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
                onInput={(ev) => renamePatch({ schema: ev.currentTarget.value })}
              />
              <datalist id={schemaList}>
                <For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>
              </datalist>
            </>
          )}
        </FieldRow>
        <FieldRow label={alter() ? g.rename : g.name} path="name" issues={props.issues}>
          {(a) => (
            <input
              id={a.id}
              ref={nameEl}
              class={inputClass + invalidClass(a.invalid)}
              autocomplete="off"
              spellcheck={false}
              required
              aria-required="true"
              placeholder={g.namePlaceholder}
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={props.readOnly}
              value={d().name}
              onInput={(ev) => renamePatch({ name: ev.currentTarget.value })}
            />
          )}
        </FieldRow>
      </div>
      <OwnerSelect
        designer={props.designer}
        tabId={props.tabId}
        value={d().owner}
        issues={props.issues}
        disabled={props.readOnly}
        onChange={(v) => patch({ owner: v })}
      />
      <FieldRow
        label={g.persistence}
        path="persistence"
        issues={props.issues}
        hint={gate().persistenceHint ?? undefined}
        warning={persistenceChanged() ? g.persistenceAlterNote : null}
      >
        {(a) => (
          <select
            id={a.id}
            class={inputClass + invalidClass(a.invalid)}
            aria-describedby={a.describedBy}
            aria-invalid={a.invalid}
            disabled={props.readOnly || gate().persistenceLocked}
            value={d().persistence}
            onChange={(ev) => patch({ persistence: ev.currentTarget.value as Persistence })}
          >
            <option value="permanent">{g.persistenceOptions.permanent}</option>
            <option value="unlogged">{g.persistenceOptions.unlogged}</option>
          </select>
        )}
      </FieldRow>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <FieldRow label={g.tablespace} path="tablespace" issues={props.issues} hint={gate().tablespaceHint ?? g.tablespaceHint}>
          {(a) => (
            <select
              id={a.id}
              class={inputClass + invalidClass(a.invalid)}
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={props.readOnly}
              value={d().tablespace ?? ""}
              onChange={(ev) => {
                const v = ev.currentTarget.value;
                patch({ tablespace: normalizeTablespaceEdit(originalTablespace(), v) });
              }}
            >
              {/* Temizleme yok: boş seçenek yalnızca yüklenen tanımda tablespace yoksa. */}
              <Show when={originalTablespace() === null}>
                <option value="">{g.tablespaceUnset}</option>
              </Show>
              <option value="pg_default" disabled={gate().pgDefaultBlocked && d().tablespace !== "pg_default"}>
                {g.tablespaceDefault}
              </option>
              <For each={tablespaceChoices(tablespaces() ?? [], d().tablespace)}>{(t) => <option value={t}>{t}</option>}</For>
            </select>
          )}
        </FieldRow>
        <FieldRow label={g.fillfactor} path="fillfactor" issues={props.issues} hint={gate().fillfactorHint ?? g.fillfactorHint}>
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
              disabled={props.readOnly || gate().fillfactorLocked}
              value={d().fillfactor ?? ""}
              onInput={(ev) => {
                // Çözümlenemeyen/uzun girdi yok sayılır: son geçerli değer kalır, yazılan metin silinmez.
                const v = fillfactorFromInput(ev.currentTarget.value, ev.currentTarget.validity.badInput);
                if (v !== undefined) patch({ fillfactor: v });
              }}
            />
          )}
        </FieldRow>
      </div>
      <CommentField
        value={d().comment}
        issues={props.issues}
        disabled={props.readOnly}
        onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
      />
      <Show when={alter() ? props.original?.partitionText : null}>
        {(text) => (
          <div class="flex flex-col gap-1">
            <span class="text-xs text-muted">{g.partitionText}</span>
            <code class="rounded border border-border px-2 py-1 font-mono text-[13px] break-all">PARTITION BY {text()}</code>
            <span class="text-xs text-muted">{g.partitionTextHint}</span>
          </div>
        )}
      </Show>
    </form>
  );
}

/** Bölümleme (yalnızca create): strateji + anahtar listesi. Satırlar sıra numarası listesiyle çizilir (odak korunur). */
export function TablePartitioning(props: TableSectionProps) {
  const p = ttbl.partitioning;
  const d = () => props.draft;
  const keys = () => d().partitionBy?.keys ?? [];
  const keyRows = createMemo(() => keys().map((_, j) => j), undefined, { equals: (a, b) => a.length === b.length });
  const keyEls: Array<HTMLElement | undefined> = [];
  let addEl: HTMLButtonElement | undefined;
  const focusLater = (el: () => HTMLElement | undefined) => queueMicrotask(() => el()?.focus());
  const editKey = (j: number, key: PartitionKey) => props.editor.edit((x) => updatePartitionKey(x, j, key));

  return (
    <form class="flex max-w-2xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
      <ListErrors messages={exactIssues(props.issues, "partitionBy")} />
      <FieldRow label={p.strategy} path="partitionBy.strategy" issues={props.issues} hint={p.hint}>
        {(a) => (
          <select
            id={a.id}
            class={inputClass + invalidClass(a.invalid)}
            aria-describedby={a.describedBy}
            aria-invalid={a.invalid}
            disabled={props.readOnly}
            value={d().partitionBy?.strategy ?? ""}
            onChange={(ev) => {
              const v = ev.currentTarget.value;
              props.editor.edit((x) => withPartitionStrategy(x, v === "" ? null : (v as PartitionStrategy)));
            }}
          >
            <option value="">{p.none}</option>
            <option value="range">{p.strategies.range}</option>
            <option value="list">{p.strategies.list}</option>
            <option value="hash">{p.strategies.hash}</option>
          </select>
        )}
      </FieldRow>
      <Show when={d().partitionBy}>
        <div role="group" aria-label={p.keys} class="flex flex-col gap-2">
          <span class="text-xs text-muted">{p.keys}</span>
          <ListErrors messages={[...exactIssues(props.issues, "partitionBy.keys"), ...orphanRowIssues(props.issues, "partitionBy.keys", keys().length)]} />
          <Show when={keys().length === 0}>
            <p class="text-xs text-muted">{p.noKeys}</p>
          </Show>
          <For each={keyRows()}>
            {(j) => {
              // Satır silinirken kapanmadan önce bir kez daha okunabilir: boş anahtarla korunur.
              const k = (): PartitionKey => keys()[j] ?? { kind: "column", name: "" };
              const label = () => partitionKeyLabel(k());
              return (
                <div role="group" aria-label={p.key(j)} class="flex flex-col gap-2 rounded border border-border p-2">
                  <div class="flex items-center gap-1">
                    <span class="text-xs font-medium">{p.key(j)}</span>
                    <div class="ml-auto">
                      <Button
                        variant="ghost"
                        aria-label={p.removeKey(j, label())}
                        title={p.removeKey(j, label())}
                        disabled={props.readOnly}
                        onClick={() => {
                          props.editor.edit((x) => removePartitionKey(x, j));
                          focusLater(() => (keys().length === 0 ? addEl : keyEls[Math.min(j, keys().length - 1)]));
                        }}
                      >
                        <Trash2 size={14} aria-hidden="true" />
                      </Button>
                    </div>
                  </div>
                  <div class="grid grid-cols-1 gap-2 sm:grid-cols-[9rem_1fr]">
                    <FieldRow label={p.keyKind} path={`partitionBy.keys[${j}].kind`} issues={props.issues}>
                      {(a) => (
                        <select
                          id={a.id}
                          class={inputClass}
                          aria-describedby={a.describedBy}
                          disabled={props.readOnly}
                          value={k().kind}
                          onChange={(ev) => editKey(j, switchPartitionKeyKind(k(), ev.currentTarget.value as PartitionKey["kind"]))}
                        >
                          <option value="column">{p.keyColumn}</option>
                          <option value="expression">{p.keyExpression}</option>
                        </select>
                      )}
                    </FieldRow>
                    <PartitionKeyField
                      index={j}
                      keyValue={k()}
                      draft={d()}
                      issues={props.issues}
                      disabled={props.readOnly}
                      ref={(el) => (keyEls[j] = el)}
                      onInput={(key) => editKey(j, key)}
                    />
                  </div>
                </div>
              );
            }}
          </For>
          <div>
            <Button
              ref={(el: HTMLButtonElement) => (addEl = el)}
              disabled={props.readOnly}
              onClick={() => {
                props.editor.edit((x) => addPartitionKey(x, "column"));
                focusLater(() => keyEls[keys().length - 1]);
              }}
            >
              {p.addKey}
            </Button>
          </div>
        </div>
      </Show>
    </form>
  );
}

/** Anahtar: kolon (taslak kolon adlarından seçim) ya da serbest ifade (+ serbest SQL notu ve E'…' ipucu). */
function PartitionKeyField(props: {
  index: number;
  keyValue: PartitionKey;
  draft: TableDraft;
  issues: FormProps["issues"];
  disabled: boolean;
  ref: (el: HTMLElement) => void;
  onInput: (key: PartitionKey) => void;
}) {
  const p = ttbl.partitioning;
  const noteId = createUniqueId();
  const escapeId = createUniqueId();
  const columnName = () => (props.keyValue.kind === "column" ? props.keyValue.name : "");
  const choices = () => partitionColumnChoices(props.draft, columnName());
  const hasColumns = () => choices().some((c) => c !== "");
  return (
    <FieldRow
      label={props.keyValue.kind === "column" ? p.keyColumn : p.keyExpression}
      path={`partitionBy.keys[${props.index}]`}
      issues={props.issues}
      hint={props.keyValue.kind === "column" && !hasColumns() ? p.noColumns : undefined}
    >
      {(a) => (
        <Show
          when={props.keyValue.kind === "expression" ? props.keyValue : null}
          fallback={
            <select
              id={a.id}
              ref={props.ref}
              class={inputClass + invalidClass(a.invalid)}
              required
              aria-required="true"
              aria-describedby={a.describedBy}
              aria-invalid={a.invalid}
              disabled={props.disabled}
              value={columnName()}
              onChange={(ev) => props.onInput({ kind: "column", name: ev.currentTarget.value })}
            >
              <For each={choices()}>{(c) => <option value={c}>{c === "" ? p.chooseColumn : c}</option>}</For>
            </select>
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
                placeholder={p.expressionPlaceholder}
                aria-describedby={[a.describedBy, noteId, escapeId].filter(Boolean).join(" ")}
                aria-invalid={a.invalid}
                disabled={props.disabled}
                value={e().sql}
                onInput={(ev) => props.onInput({ kind: "expression", sql: ev.currentTarget.value })}
              />
              <FreeSqlNote id={noteId} />
              <p id={escapeId} class="text-xs text-muted">
                {ttbl.columns.escapeHint}
              </p>
            </>
          )}
        </Show>
      )}
    </FieldRow>
  );
}
