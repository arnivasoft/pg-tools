// Table formu Kolonlar bölümü (TableForm parçası): kolon ızgarası (ad, tip seçici, NOT NULL, değer kaynağı + ifade,
// ayrıntılarda collation/yorum/USING), silinecek kolonlar listesi (geri al), kolon ekle, Explorer odağı (satır vurgusu +
// kaydırma). Satırlar sıra numarası listesiyle çizilir (düzenlemede odak kaybolmaz). Kolon yeniden adlandırma kısıt ve
// bölümleme anahtarlarına konum anlık görüntüsüyle yayılır (createColumnRenames). İstemci SQL üretmez.
import { Settings2, Trash2, Undo } from "lucide-solid";
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, on } from "solid-js";
import { ttbl } from "../../app/strings-table";
import type { ColumnDef } from "../../ipc/bindings/ColumnDef";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { CommentField, FieldRow, FreeSqlNote, invalidClass } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import { withPrimaryKeyNotNull } from "./table-constraints";
import { createColumnRenames, type TableSectionProps } from "./table-form-state";
import {
  addTableColumn, allowedValueSources, applySerialFix, columnDetailsHaveIssues, columnLockGate, columnTypeWarning, newColumn, originalColumn,
  primaryKeyMember, removedColumns, restoreColumn, serialFix, updateTableColumn,
  usingVisible, valueExpr, valueSourceOf, withCollation, withDataType, withValueExpr, withValueSource, type ValueSource,
} from "./table-model";
import { TypeInput } from "./TypeInput";

const badge = "inline-flex h-5 items-center rounded border border-border px-1.5 text-[11px]";

export function TableColumns(props: TableSectionProps) {
  const t = ttbl.columns;
  const d = () => props.draft;
  const len = () => d().columns.length;
  const rows = createMemo(() => d().columns.map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length });
  const lock = createMemo(() => columnLockGate(props.readOnly, props.original));
  const editable = () => !lock().structure;
  // Ad değişimi ve silme başvuru kaydı (satırlar paylaşır; odak oturumları boyunca korunur).
  const renames = createColumnRenames(props.editor, () => props.original);
  const removed = createMemo(() => removedColumns(props.original, d()));

  /** Tek kolon düzenlemesi; ardından PK üyelerinin NOT NULL'ı korunur (değer kaynağı/ad değişimi onu düşürebilir). */
  const editColumn = (i: number, f: (c: ColumnDef) => ColumnDef) =>
    props.editor.edit((x) => withPrimaryKeyNotNull(updateTableColumn(x, i, f)));

  const rowEls: Array<HTMLElement | undefined> = [];
  const nameEls: Array<HTMLInputElement | undefined> = [];
  let addEl: HTMLButtonElement | undefined;
  const focusLater = (el: () => HTMLElement | undefined) => queueMicrotask(() => el()?.focus());

  // Collation listesi (oturum önbellekli lookup) ilk ihtiyaçta bir kez istenir; Suspense'e bağlanmaz (açılan ayrıntılar
  // paneli askıya almasın). Hata → boş liste, serbest yazım sürer; sonraki odakta yeniden denenir.
  const [collations, setCollations] = createSignal<LookupItem[]>([]);
  let collationsAsked = false;
  const loadCollations = () => {
    if (collationsAsked) return;
    collationsAsked = true;
    props.designer.lookup(props.tabId, { kind: "collations" }).then(setCollations, () => (collationsAsked = false));
  };
  const collationList = createUniqueId();

  // Explorer odağı: istek başına bir kez; yalnızca bölüm etkinken (gizli öğe odaklanamaz/kaydırılamaz).
  let appliedSeq: number | null = null;
  createEffect(() => {
    const h = props.highlight;
    if (!props.active || !h || h.section !== "columns" || h.index === null || h.seq === appliedSeq) return;
    appliedSeq = h.seq;
    const row = rowEls[h.index];
    if (!row) return;
    row.scrollIntoView?.({ block: "nearest" });
    const name = nameEls[h.index];
    if (editable() && name) name.focus();
    else row.focus();
  });

  const add = () => {
    props.editor.edit(addTableColumn);
    focusLater(() => nameEls[len() - 1]);
  };

  return (
    <form class="flex flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
      <p class="text-xs text-muted">
        {t.dataTypeHint} {ttbl.type.serverChecks}
      </p>
      {/* Üst düzey `columns` hatası genel şeritte; burada yalnızca artık var olmayan satırlarınkiler. */}
      <Show when={orphanRowIssues(props.issues, "columns", len()).length > 0}>
        <span class="text-xs text-danger">{orphanRowIssues(props.issues, "columns", len()).join(" ")}</span>
      </Show>
      <Show when={len() === 0}>
        <p class="text-[13px] text-muted">{t.empty}</p>
      </Show>
      <div role="list" aria-label={t.listLabel} class="flex flex-col gap-2">
        <For each={rows()}>
          {(i) => (
            <ColumnRow
              {...props}
              index={i}
              editable={editable()}
              commentLocked={lock().comment}
              rename={(v) => renames.rename(i, v)}
              editColumn={editColumn}
              collationList={collationList}
              loadCollations={loadCollations}
              rowRef={(el) => (rowEls[i] = el)}
              nameRef={(el) => (nameEls[i] = el)}
              onRemove={() => {
                renames.remove(i);
                focusLater(() => (len() === 0 ? addEl : nameEls[Math.min(i, len() - 1)]));
              }}
            />
          )}
        </For>
      </div>
      <datalist id={collationList}>
        <For each={collations()}>{(it) => <option value={it.value} />}</For>
      </datalist>
      <Show when={editable()}>
        <div class="flex flex-col gap-1">
          <div>
            <Button ref={(el: HTMLButtonElement) => (addEl = el)} onClick={add}>
              {t.add}
            </Button>
          </div>
          <span class="text-xs text-muted">{t.orderNote}</span>
        </div>
      </Show>
      <Show when={removed().length > 0}>
        <div class="flex flex-col gap-1">
          <span class="text-xs font-medium">{t.removed}</span>
          <ul role="list" aria-label={t.removed} class="flex flex-col gap-1">
            <For each={removed()}>
              {(c) => (
                <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
                  <span class="font-medium line-through">{c.name}</span>
                  <span class="font-mono text-muted">{c.dataType}</span>
                  <Show when={editable() && props.original}>
                    {(o) => (
                      <div class="ml-auto">
                        <Button
                          variant="ghost"
                          aria-label={t.restore(c.name)}
                          title={t.restore(c.name)}
                          onClick={() => {
                            const attnum = c.attnum;
                            if (attnum === null) return;
                            props.editor.edit((x) => withPrimaryKeyNotNull(restoreColumn(x, o(), attnum)));
                            focusLater(() => nameEls[d().columns.findIndex((k) => k.attnum === attnum)] ?? addEl);
                          }}
                        >
                          <Undo size={14} aria-hidden="true" />
                        </Button>
                      </div>
                    )}
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <span class="text-xs text-warn-text">{t.removedNote}</span>
        </div>
      </Show>
    </form>
  );
}

function ColumnRow(
  props: TableSectionProps & {
    index: number;
    editable: boolean;
    commentLocked: boolean;
    rename: (next: string) => void;
    editColumn: (i: number, f: (c: ColumnDef) => ColumnDef) => void;
    collationList: string;
    loadCollations: () => void;
    rowRef: (el: HTMLElement) => void;
    nameRef: (el: HTMLInputElement) => void;
    onRemove: () => void;
  },
) {
  const t = ttbl.columns;
  const i = props.index;
  const p = (f: string) => `columns[${i}].${f}`;
  // Satır silinirken kapanmadan önce bir kez daha okunabilir: boş kolonla korunur.
  const c = (): ColumnDef => props.draft.columns[i] ?? newColumn();
  const orig = createMemo(() => originalColumn(props.original, c()));
  const source = () => valueSourceOf(c());
  const ro = () => !props.editable;
  const selected = () => props.highlight?.section === "columns" && props.highlight.index === i;
  const showUsing = () => usingVisible(orig(), c());
  const pkMember = () => primaryKeyMember(props.draft, c().name);
  const detailsId = createUniqueId();
  const notNullHint = createUniqueId();
  const exprNote = createUniqueId();
  const exprEscape = createUniqueId();
  const usingNote = createUniqueId();
  const usingEscape = createUniqueId();
  const typeChangedId = createUniqueId();
  const rowErrId = createUniqueId();
  const notNullErr = () => exactIssues(props.issues, p("notNull")).length > 0;

  const [open, setOpen] = createSignal(false);
  // Hatası olan satırın ayrıntıları ve tipi yeni değişen yüklü kolonun USING alanı kendiliğinden açılır.
  createEffect(on(() => columnDetailsHaveIssues(props.issues, i), (v) => v && setOpen(true)));
  createEffect(on(showUsing, (v, prev) => v && !prev && setOpen(true), { defer: true }));

  const setSource = (s: ValueSource) => props.editColumn(i, (k) => withValueSource(k, s, orig()));
  const exprPath = () => (source() === "default" ? p("default") : p("generated"));
  const hasExpr = () => source() === "default" || source().startsWith("generated");
  /** Satırda alanı görünmeyen yollar (satır, kimlik, NOT NULL; ifade alanı yokken default/generated). */
  const rowErrors = () => {
    const paths = [`columns[${i}]`, p("attnum"), p("notNull")];
    if (!hasExpr()) paths.push(p("default"), p("generated"));
    return paths.flatMap((x) => exactIssues(props.issues, x));
  };

  return (
    <div
      role="listitem"
      ref={props.rowRef}
      tabIndex={-1}
      class="rounded border border-border p-2 outline-none focus:border-accent"
      classList={{ "bg-accent/10": selected() }}
    >
      <fieldset class="flex min-w-0 flex-col gap-2" aria-describedby={rowErrors().length > 0 ? rowErrId : undefined}>
        <legend class="sr-only">
          {t.row(i, c().name)}
        </legend>
        <div class="flex items-center gap-1.5">
          <span class="text-xs font-medium" aria-hidden="true">
            {i + 1}.
          </span>
          <Show when={c().attnum === null}>
            <span class={badge}>{t.newBadge}</span>
          </Show>
          <Show when={selected()}>
            <span class={badge}>{t.selectedBadge}</span>
          </Show>
          <div class="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              aria-label={t.detailsToggle(i, c().name)}
              title={t.details}
              aria-expanded={open()}
              aria-controls={detailsId}
              onClick={() => setOpen(!open())}
            >
              <Settings2 size={14} aria-hidden="true" />
            </Button>
            <Show when={props.editable}>
              <Button variant="ghost" aria-label={t.remove(i, c().name)} title={t.remove(i, c().name)} onClick={props.onRemove}>
                <Trash2 size={14} aria-hidden="true" />
              </Button>
            </Show>
          </div>
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
                required
                aria-required="true"
                placeholder={t.namePlaceholder}
                aria-describedby={a.describedBy}
                aria-invalid={a.invalid}
                disabled={ro()}
                value={c().name}
                onInput={(ev) => props.rename(ev.currentTarget.value)}
              />
            )}
          </FieldRow>
          <FieldRow label={t.dataType} path={p("dataType")} issues={props.issues} exact warning={columnTypeWarning(c(), orig())}>
            {(a) => (
              <>
                <TypeInput
                  designer={props.designer}
                  tabId={props.tabId}
                  id={a.id}
                  value={c().dataType}
                  describedBy={[a.describedBy, showUsing() ? typeChangedId : undefined].filter(Boolean).join(" ") || undefined}
                  invalid={a.invalid}
                  disabled={ro()}
                  onInput={(v) => props.editColumn(i, (k) => withDataType(k, orig(), v))}
                />
                <Show when={props.editable ? serialFix(c().dataType) : null}>
                  {(fix) => (
                    <div>
                      <Button variant="ghost" class="!h-6 !px-1.5 text-xs" onClick={() => props.editColumn(i, (k) => applySerialFix(k, orig()))}>
                        {ttbl.type.serialFix(fix().dataType)}
                      </Button>
                    </div>
                  )}
                </Show>
              </>
            )}
          </FieldRow>
        </div>
        <Show when={showUsing()}>
          <p id={typeChangedId} class="text-xs text-warn-text">
            {t.typeChanged}
          </p>
        </Show>
        <div class="grid grid-cols-1 items-start gap-2 sm:grid-cols-[auto_1fr]">
          <div class="flex flex-col gap-1 pt-5">
            <label class="inline-flex items-center gap-1.5 text-[13px]">
              <input
                type="checkbox"
                aria-describedby={[c().identity !== null || pkMember() ? notNullHint : "", notNullErr() ? rowErrId : ""].filter(Boolean).join(" ") || undefined}
                aria-invalid={notNullErr()}
                disabled={ro() || c().identity !== null || pkMember()}
                checked={c().notNull}
                onChange={(ev) => {
                  const v = ev.currentTarget.checked;
                  props.editColumn(i, (k) => ({ ...k, notNull: v }));
                }}
              />
              {t.notNull}
            </label>
            <Show when={c().identity !== null || pkMember()}>
              <span id={notNullHint} class="text-xs text-muted">
                {c().identity !== null ? t.notNullIdentity : t.notNullPrimaryKey}
              </span>
            </Show>
          </div>
          <FieldRow
            label={t.source}
            path={p("identity")}
            issues={props.issues}
            exact
            hint={
              orig()?.generated && source() === "none" ? t.sourceDropExpression
              : orig() && !orig()!.generated ? t.sourceNoGenerate
              : undefined
            }
          >
            {(a) => (
              <select
                id={a.id}
                class={inputClass + invalidClass(a.invalid)}
                aria-describedby={a.describedBy}
                aria-invalid={a.invalid}
                disabled={ro()}
                value={source()}
                onChange={(ev) => setSource(ev.currentTarget.value as ValueSource)}
              >
                <For each={allowedValueSources(orig())}>{(s) => <option value={s}>{t.sources[s]}</option>}</For>
              </select>
            )}
          </FieldRow>
        </div>
        <Show when={hasExpr()}>
          <FieldRow label={t.expr} path={exprPath()} issues={props.issues}>
            {(a) => (
              <>
                <input
                  id={a.id}
                  class={`${inputClass} font-mono${invalidClass(a.invalid)}`}
                  autocomplete="off"
                  spellcheck={false}
                  placeholder={source() === "default" ? t.exprDefaultPlaceholder : t.exprGeneratedPlaceholder}
                  aria-describedby={[a.describedBy, exprNote, exprEscape].filter(Boolean).join(" ")}
                  aria-invalid={a.invalid}
                  disabled={ro()}
                  value={valueExpr(c())}
                  onInput={(ev) => {
                    const v = ev.currentTarget.value;
                    props.editColumn(i, (k) => withValueExpr(k, v));
                  }}
                />
                <FreeSqlNote id={exprNote} />
                <p id={exprEscape} class="text-xs text-muted">
                  {t.escapeHint}
                </p>
              </>
            )}
          </FieldRow>
        </Show>
        <Show when={rowErrors().length > 0}>
          <span id={rowErrId} class="text-xs text-danger">
            {rowErrors().join(" ")}
          </span>
        </Show>
        <div id={detailsId} hidden={!open()} class="flex flex-col gap-2 border-t border-border pt-2">
          <FieldRow label={t.collation} path={p("collation")} issues={props.issues}>
            {(a) => (
              <input
                id={a.id}
                class={inputClass + invalidClass(a.invalid)}
                list={props.collationList}
                autocomplete="off"
                spellcheck={false}
                placeholder={t.collationPlaceholder}
                aria-describedby={a.describedBy}
                aria-invalid={a.invalid}
                disabled={ro()}
                value={c().collation ?? ""}
                onFocus={props.loadCollations}
                onInput={(ev) => {
                  const v = ev.currentTarget.value;
                  props.editColumn(i, (k) => withCollation(k, orig(), v));
                }}
              />
            )}
          </FieldRow>
          <CommentField
            value={c().comment}
            path={p("comment")}
            issues={props.issues}
            disabled={props.commentLocked}
            onChange={(v) => props.editColumn(i, (k) => ({ ...k, comment: v === null ? null : normalizeEmpty(v) }))}
          />
          <Show when={showUsing()}>
            <FieldRow label={t.using} path={p("using")} issues={props.issues} hint={t.usingHint}>
              {(a) => (
                <>
                  <input
                    id={a.id}
                    class={`${inputClass} font-mono${invalidClass(a.invalid)}`}
                    autocomplete="off"
                    spellcheck={false}
                    placeholder={t.usingPlaceholder}
                    aria-describedby={[a.describedBy, usingNote, usingEscape].filter(Boolean).join(" ")}
                    aria-invalid={a.invalid}
                    disabled={ro()}
                    value={c().using ?? ""}
                    onInput={(ev) => {
                      const v = ev.currentTarget.value;
                      props.editColumn(i, (k) => ({ ...k, using: normalizeEmpty(v) }));
                    }}
                  />
                  <FreeSqlNote id={usingNote} />
                  <p id={usingEscape} class="text-xs text-muted">
                    {t.escapeHint}
                  </p>
                </>
              )}
            </FieldRow>
          </Show>
        </div>
      </fieldset>
    </div>
  );
}
