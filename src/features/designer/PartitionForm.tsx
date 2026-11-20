// Partition formu (tembel parça): Üst tablo (salt okunur), kip seçimi (yeni bölüm tablosu / var olan tabloyu bağla),
// Genel (şema, ad / bağlanacak tablo), sınır düzenleyicisi (range / list / hash / DEFAULT), isteğe bağlı alt bölümleme,
// tablespace, sahip ve yorum. Alter ("Bölüm sınırı…") yalnız sınırı düzenler. İstemci SQL üretmez: sınır değerleri ve
// ifadeler olduğu gibi gider (kırpılmaz). `strategy`/`keyCount`/`detachPending`/`parent*` taslakta dokunulmadan kalır.
// Kobalte yok; metinler tembel `strings-partition`.
import { For, Show, createMemo, createResource, createUniqueId, type JSX } from "solid-js";
import { tpart } from "../../app/strings-partition";
import type { BoundValue } from "../../ipc/bindings/BoundValue";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { PartitionDef } from "../../ipc/bindings/PartitionDef";
import type { PartitionKey } from "../../ipc/bindings/PartitionKey";
import type { PartitionStrategy } from "../../ipc/bindings/PartitionStrategy";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CheckRow, CommentField, FieldRow, FreeSqlNote, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues, tablespaceChoices } from "./forms-model";
import {
  asPartition, attachPick, boundChanged, defaultPatch, hashPatch, listValuePatch, modePatch, partitionPatch, rangeKindPatch,
  rangeValuePatch, subKeyKind, subKeyPatch, subKeysPatch, subStrategyPatch, type BoundSide,
} from "./partition-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import { RowList } from "./RowList";

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

const STRATEGIES: PartitionStrategy[] = ["range", "list", "hash"];

export default function PartitionForm(props: FormProps) {
  const draft = () => asPartition(props.model.draft);
  const original = () => asPartition(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez: her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);

  const patch = (p: Partial<PartitionDef>) => props.designer.edit(props.tabId, (x) => partitionPatch(x, p));
  /** Taslağın güncel hâli üzerinde işlem yapan düzenleme. */
  const update = (f: (d: PartitionDef) => PartitionDef) =>
    props.designer.edit(props.tabId, (x) => {
      const prev = asPartition(x);
      if (!prev) return x;
      return partitionPatch(x, f(prev));
    });

  const modeName = createUniqueId();
  const hintId = createUniqueId();
  const subHintId = createUniqueId();
  const noteId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const schemaList = createUniqueId();
  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  const [tablespaces] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "tablespaces" }).catch(() => []),
  );
  // Bağlanabilecek tablolar: yalnız create + attach kipinde ve üst tablo oid'i bilinirken istenir.
  const parentOid = () => props.model.createContext?.tableOid ?? null;
  const [attachable] = createResource(
    () => (!alter() && draft()?.mode === "attach" && parentOid() !== null ? { tabId: props.tabId, oid: parentOid() as number } : false),
    (k) => props.designer.lookup(k.tabId, { kind: "attachable_tables", parent_oid: k.oid }).catch((): LookupItem[] => []),
  );

  return (
    <Show when={draft()}>
      {(d) => {
        const attachMode = () => !alter() && d().mode === "attach";
        const createMode = () => !alter() && d().mode === "create";
        const boundLocked = () => locked() || (alter() && d().detachPending);
        const changed = createMemo(() => alter() && boundChanged(original(), d()));
        const topErrors = () => [...issuesFor(props.issues, "parentSchema"), ...issuesFor(props.issues, "parentTable"), ...issuesFor(props.issues, "object")];
        const items = () => attachable() ?? [];
        // Seçili tablo (schema + ad eşleşmesi); yoksa boş seçenek.
        const pickedIndex = () => items().findIndex((i) => i.detail === d().schema && i.value === d().name);
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />
            <Show when={alter() && d().detachPending}>
              <p role="alert" class="rounded border border-[color:var(--srv-amber)] px-3 py-2 text-xs text-warn-text">
                {tpart.notes.detachPending}
              </p>
            </Show>

            <Section title={tpart.sections.parent}>
              <FieldRow label={tpart.parent.table} path="parentTable" issues={props.issues} hint={tpart.parent.tableHint} exact>
                {(f) => (
                  <input
                    id={f.id}
                    class={`${inputClass} font-mono text-muted${invalidClass(f.invalid)}`}
                    readOnly
                    aria-readonly="true"
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    value={`${d().parentSchema}.${d().parentTable}`}
                  />
                )}
              </FieldRow>
              <p class="text-[13px] text-muted">
                <span class="font-mono">{d().strategy.toUpperCase()}</span> · {tpart.parent.keyCount(d().keyCount)}
              </p>
            </Section>

            <Show when={!alter()}>
              <fieldset class="flex flex-col gap-1">
                <legend class="text-[13px] font-medium">{tpart.mode.legend}</legend>
                <label class="flex items-center gap-2 text-[13px]" classList={{ "text-muted": locked() }}>
                  <input
                    type="radio"
                    name={modeName}
                    disabled={locked()}
                    checked={d().mode === "create"}
                    onChange={(ev) => ev.currentTarget.checked && update((x) => modePatch(x, "create"))}
                  />
                  {tpart.mode.create}
                </label>
                <label class="flex items-center gap-2 text-[13px]" classList={{ "text-muted": locked() }}>
                  <input
                    type="radio"
                    name={modeName}
                    disabled={locked()}
                    checked={d().mode === "attach"}
                    onChange={(ev) => ev.currentTarget.checked && update((x) => modePatch(x, "attach"))}
                  />
                  {tpart.mode.attach}
                </label>
              </fieldset>
            </Show>

            <Section title={tpart.sections.general}>
              <Show
                when={!alter()}
                fallback={
                  <FieldRow label={tpart.general.partitionName} path="name" issues={props.issues} hint={tpart.general.partitionNameHint}>
                    {(f) => (
                      <input
                        id={f.id}
                        class={`${inputClass} font-mono text-muted${invalidClass(f.invalid)}`}
                        readOnly
                        aria-readonly="true"
                        aria-describedby={f.describedBy}
                        aria-invalid={f.invalid}
                        value={`${d().schema}.${d().name}`}
                      />
                    )}
                  </FieldRow>
                }
              >
                <Show
                  when={createMode()}
                  fallback={
                    <>
                      <FieldRow label={tpart.attach.label} path="name" issues={props.issues}>
                        {(f) => (
                          <select
                            id={f.id}
                            class={inputClass + invalidClass(f.invalid)}
                            required
                            aria-required="true"
                            aria-describedby={[f.describedBy, noteId].filter(Boolean).join(" ")}
                            aria-invalid={f.invalid}
                            disabled={locked() || items().length === 0}
                            onChange={(ev) => {
                              const it = items()[Number(ev.currentTarget.value)];
                              if (it) update((x) => attachPick(x, it.value, it.detail ?? ""));
                            }}
                          >
                            <option value="-1" selected={pickedIndex() < 0}>
                              {attachable.loading ? tpart.attach.loading : items().length === 0 ? tpart.attach.none : tpart.attach.pick}
                            </option>
                            <For each={items()}>
                              {(it, i) => (
                                <option value={i()} selected={i() === pickedIndex()}>
                                  {it.detail ? `${it.detail}.${it.value}` : it.value}
                                </option>
                              )}
                            </For>
                          </select>
                        )}
                      </FieldRow>
                      <Show when={!attachable.loading && items().length === 0}>
                        <p id={noteId} class="text-xs text-muted">
                          {tpart.attach.noneNote}
                        </p>
                      </Show>
                      <p role="note" class="text-xs text-warn-text">
                        {tpart.attach.destructive}
                      </p>
                    </>
                  }
                >
                  <FieldRow label={tpart.general.schema} path="schema" issues={props.issues}>
                    {(f) => (
                      <>
                        <input
                          id={f.id}
                          class={inputClass + invalidClass(f.invalid)}
                          list={schemaList}
                          autocomplete="off"
                          spellcheck={false}
                          required
                          aria-required="true"
                          aria-describedby={f.describedBy}
                          aria-invalid={f.invalid}
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
                  <FieldRow label={tpart.general.name} path="name" issues={props.issues}>
                    {(f) => (
                      <input
                        id={f.id}
                        ref={nameEl}
                        class={inputClass + invalidClass(f.invalid)}
                        autocomplete="off"
                        spellcheck={false}
                        required
                        aria-required="true"
                        placeholder={tpart.general.namePlaceholder}
                        aria-describedby={f.describedBy}
                        aria-invalid={f.invalid}
                        disabled={locked()}
                        value={d().name}
                        onInput={(ev) => patch({ name: ev.currentTarget.value })}
                      />
                    )}
                  </FieldRow>
                </Show>
              </Show>
            </Section>

            <Section title={tpart.sections.bound}>
              <ListErrors
                messages={[
                  ...exactIssues(props.issues, "bound"), ...exactIssues(props.issues, "bound.from"),
                  ...exactIssues(props.issues, "bound.to"), ...exactIssues(props.issues, "bound.values"),
                ]}
              />
              <Show when={d().strategy !== "hash"}>
                <CheckRow
                  label={tpart.bound.default}
                  hint={tpart.bound.defaultHint}
                  path="bound.default"
                  issues={props.issues}
                  checked={d().bound.kind === "default"}
                  disabled={boundLocked()}
                  onChange={(on) => update((x) => defaultPatch(x, on))}
                />
              </Show>
              <Show when={d().bound.kind !== "default"}>
                <RangeEditor d={d()} props={props} disabled={boundLocked()} update={update} />
                <ListEditor d={d()} props={props} disabled={boundLocked()} update={update} />
                <HashEditor d={d()} props={props} disabled={boundLocked()} update={update} />
                <p id={hintId} class="text-xs text-muted">
                  {tpart.bound.hint}
                </p>
                <FreeSqlNote />
              </Show>
              <Show when={changed()}>
                <p role="note" class="text-xs text-warn-text">
                  {tpart.notes.rebind}
                </p>
              </Show>
            </Section>

            <Show when={createMode()}>
              <Section title={tpart.sections.sub}>
                <ListErrors messages={[...exactIssues(props.issues, "partitionBy"), ...exactIssues(props.issues, "partitionBy.keys")]} />
                <FieldRow label={tpart.sub.strategy} path="partitionBy.strategy" issues={props.issues} hint={tpart.sub.hint}>
                  {(f) => (
                    <select
                      id={f.id}
                      class={inputClass + invalidClass(f.invalid)}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().partitionBy?.strategy ?? ""}
                      onChange={(ev) => {
                        const v = ev.currentTarget.value;
                        update((x) => subStrategyPatch(x, v === "" ? null : (v as PartitionStrategy)));
                      }}
                    >
                      <option value="" selected={d().partitionBy === null}>
                        {tpart.sub.none}
                      </option>
                      <For each={STRATEGIES}>
                        {(s) => (
                          <option value={s} selected={d().partitionBy?.strategy === s}>
                            {tpart.sub[s]}
                          </option>
                        )}
                      </For>
                    </select>
                  )}
                </FieldRow>
                <Show when={d().partitionBy}>
                  {(p) => (
                    <div class="flex flex-col gap-1" aria-describedby={subHintId}>
                      <RowList
                        label={tpart.sub.keys}
                        items={p().keys}
                        onChange={(keys) => update((x) => subKeysPatch(x, keys))}
                        name={(k) => (k.kind === "column" ? k.name : k.sql)}
                        newItem={(): PartitionKey => ({ kind: "column", name: "" })}
                        addLabel={tpart.sub.add}
                        emptyText={tpart.sub.empty}
                        readOnly={locked()}
                      >
                        {(k, j) => (
                          <div class="grid grid-cols-1 gap-2 sm:grid-cols-[9rem_1fr]">
                            <FieldRow label={tpart.sub.keyKind} path={`partitionBy.keys[${j}].kind`} issues={props.issues} exact>
                              {(f) => (
                                <select
                                  id={f.id}
                                  class={inputClass}
                                  aria-describedby={f.describedBy}
                                  disabled={locked()}
                                  value={k().kind}
                                  onChange={(ev) => {
                                    const kind = ev.currentTarget.value as PartitionKey["kind"];
                                    update((x) => subKeyPatch(x, j, subKeyKind(k(), kind)));
                                  }}
                                >
                                  <option value="column" selected={k().kind === "column"}>{tpart.sub.column}</option>
                                  <option value="expression" selected={k().kind === "expression"}>{tpart.sub.expression}</option>
                                </select>
                              )}
                            </FieldRow>
                            <FieldRow
                              label={k().kind === "column" ? tpart.sub.column : tpart.sub.expression}
                              path={`partitionBy.keys[${j}]`}
                              issues={props.issues}
                            >
                              {(f) => (
                                <input
                                  id={f.id}
                                  class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                                  autocomplete="off"
                                  spellcheck={false}
                                  required
                                  aria-required="true"
                                  placeholder={k().kind === "column" ? tpart.sub.columnPlaceholder : tpart.sub.expressionPlaceholder}
                                  aria-describedby={f.describedBy}
                                  aria-invalid={f.invalid}
                                  disabled={locked()}
                                  value={k().kind === "column" ? (k() as { name: string }).name : (k() as { sql: string }).sql}
                                  onInput={(ev) => {
                                    const v = ev.currentTarget.value;
                                    update((x) => subKeyPatch(x, j, k().kind === "column" ? { kind: "column", name: v } : { kind: "expression", sql: v }));
                                  }}
                                />
                              )}
                            </FieldRow>
                          </div>
                        )}
                      </RowList>
                      <ListErrors messages={orphanRowIssues(props.issues, "partitionBy.keys", p().keys.length)} />
                      <FreeSqlNote id={subHintId} />
                    </div>
                  )}
                </Show>
              </Section>

              <Section title={tpart.sections.storage}>
                <FieldRow label={tpart.storage.tablespace} path="tablespace" issues={props.issues}>
                  {(f) => (
                    <select
                      id={f.id}
                      class={inputClass + invalidClass(f.invalid)}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().tablespace ?? ""}
                      onChange={(ev) => patch({ tablespace: normalizeEmpty(ev.currentTarget.value) })}
                    >
                      <option value="" selected={d().tablespace === null}>
                        {tpart.storage.tablespaceDefault}
                      </option>
                      <For each={tablespaceChoices(tablespaces() ?? [], d().tablespace)}>
                        {(t) => (
                          <option value={t} selected={t === d().tablespace}>
                            {t}
                          </option>
                        )}
                      </For>
                    </select>
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
            </Show>
          </form>
        );
      }}
    </Show>
  );
}

interface EditorProps {
  d: PartitionDef;
  props: FormProps;
  disabled: boolean;
  update: (f: (d: PartitionDef) => PartitionDef) => void;
}

/** Range: `keyCount` satır (ekle/sil yok); her satırda FROM ve TO için tür seçimi + değer metni. */
function RangeEditor(p: EditorProps) {
  const cells = (side: BoundSide): BoundValue[] => (p.d.bound.kind === "range" ? p.d.bound[side] : []);
  const rows = createMemo(() => cells("from").map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length });
  return (
    <Show when={p.d.bound.kind === "range"}>
      <div role="group" aria-label={tpart.bound.fromTo} class="flex flex-col gap-2">
        <For each={rows()}>
          {(i) => (
            <div role="group" aria-label={tpart.bound.key(i)} class="flex flex-col gap-2 rounded border border-border p-2">
              <span class="text-xs font-medium">{tpart.bound.key(i)}</span>
              <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                <For each={["from", "to"] as BoundSide[]}>
                  {(side) => {
                    const cell = (): BoundValue | undefined => cells(side)[i];
                    const aria = () => (side === "from" ? tpart.bound.fromAria(i) : tpart.bound.toAria(i));
                    return (
                      <FieldRow label={side === "from" ? tpart.bound.from : tpart.bound.to} path={`bound.${side}[${i}]`} issues={p.props.issues}>
                        {(f) => (
                          <div class="flex gap-2">
                            <select
                              id={f.id}
                              class={`${inputClass} w-32 shrink-0`}
                              aria-label={side === "from" ? tpart.bound.fromKindAria(i) : tpart.bound.toKindAria(i)}
                              aria-describedby={f.describedBy}
                              disabled={p.disabled}
                              value={cell()?.kind ?? "value"}
                              onChange={(ev) => p.update((x) => rangeKindPatch(x, side, i, ev.currentTarget.value as BoundValue["kind"]))}
                            >
                              <option value="value" selected={cell()?.kind === "value"}>{tpart.bound.kindValue}</option>
                              <option value="minvalue" selected={cell()?.kind === "minvalue"}>{tpart.bound.kindMin}</option>
                              <option value="maxvalue" selected={cell()?.kind === "maxvalue"}>{tpart.bound.kindMax}</option>
                            </select>
                            <Show when={cell()?.kind === "value"}>
                              <input
                                class={`${inputClass} min-w-0 flex-1 font-mono${invalidClass(f.invalid)}`}
                                autocomplete="off"
                                spellcheck={false}
                                aria-label={aria()}
                                aria-describedby={f.describedBy}
                                aria-invalid={f.invalid}
                                placeholder={tpart.bound.valuePlaceholder}
                                disabled={p.disabled}
                                value={(cell() as { sql?: string } | undefined)?.sql ?? ""}
                                onInput={(ev) => p.update((x) => rangeValuePatch(x, side, i, ev.currentTarget.value))}
                              />
                            </Show>
                          </div>
                        )}
                      </FieldRow>
                    );
                  }}
                </For>
              </div>
            </div>
          )}
        </For>
        <ListErrors
          messages={[...orphanRowIssues(p.props.issues, "bound.from", cells("from").length), ...orphanRowIssues(p.props.issues, "bound.to", cells("to").length)]}
        />
      </div>
    </Show>
  );
}

/** List: değer satırları (ekle/sil); `NULL` serbest. */
function ListEditor(p: EditorProps) {
  const values = (): string[] => (p.d.bound.kind === "list" ? p.d.bound.values : []);
  return (
    <Show when={p.d.bound.kind === "list"}>
      <div class="flex flex-col gap-1">
        <RowList
          label={tpart.bound.values}
          items={values()}
          onChange={(vals) => p.update((x) => (x.bound.kind === "list" ? { ...x, bound: { ...x.bound, values: vals } } : x))}
          name={(v) => v}
          newItem={() => ""}
          reorderable={false}
          addLabel={tpart.bound.add}
          emptyText={tpart.bound.empty}
          readOnly={p.disabled}
        >
          {(v, i) => (
            <FieldRow label={tpart.bound.value} path={`bound.values[${i}]`} issues={p.props.issues} exact>
              {(f) => (
                <input
                  id={f.id}
                  class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                  autocomplete="off"
                  spellcheck={false}
                  placeholder={tpart.bound.valuePlaceholderList}
                  aria-describedby={f.describedBy}
                  aria-invalid={f.invalid}
                  disabled={p.disabled}
                  value={v()}
                  onInput={(ev) => p.update((x) => listValuePatch(x, i, ev.currentTarget.value))}
                />
              )}
            </FieldRow>
          )}
        </RowList>
        <ListErrors messages={orphanRowIssues(p.props.issues, "bound.values", values().length)} />
      </div>
    </Show>
  );
}

/** Hash: MODULUS / REMAINDER sayı alanları; geçersiz metin modeli değiştirmez, odak çıkınca son geçerli sayı döner. */
function HashEditor(p: EditorProps) {
  const field = (which: "modulus" | "remainder", label: string): JSX.Element => (
    <FieldRow label={label} path={`bound.${which}`} issues={p.props.issues} exact>
      {(f) => (
        <input
          id={f.id}
          class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
          inputmode="numeric"
          autocomplete="off"
          aria-describedby={f.describedBy}
          aria-invalid={f.invalid}
          disabled={p.disabled}
          value={p.d.bound.kind === "hash" ? String(p.d.bound[which]) : ""}
          onInput={(ev) => {
            const el = ev.currentTarget;
            p.update((x) => hashPatch(x, which, el.value));
            // Geçersiz (sayı olmayan/taşan) metin yazılamaz; boş alan yeni sayı yazılana kadar boş kalabilir.
            if (el.value !== "" && !/^\d+$/.test(el.value)) el.value = p.d.bound.kind === "hash" ? String(p.d.bound[which]) : "";
          }}
          onBlur={(ev) => {
            ev.currentTarget.value = p.d.bound.kind === "hash" ? String(p.d.bound[which]) : "";
          }}
        />
      )}
    </FieldRow>
  );
  return (
    <Show when={p.d.bound.kind === "hash"}>
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {field("modulus", tpart.bound.modulus)}
        {field("remainder", tpart.bound.remainder)}
      </div>
    </Show>
  );
}
