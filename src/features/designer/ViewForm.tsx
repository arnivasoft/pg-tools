// View / Materialized View formu (tembel parça; DesignerView'ın tek `lazy` girdisi): Genel, Sorgu, Kolonlar (salt okunur),
// Seçenekler (view) ya da Depolama (MV). İstemci SQL üretmez: sorgu olduğu gibi gider; yeniden oluşturma yalnızca açık seçimdir.
// Kobalte/ikon yok (yalnızca yerel denetimler); metinler tembel `strings-view` modülünde.
import { For, Show, createResource, createUniqueId, type JSX } from "solid-js";
import { tview } from "../../app/strings-view";
import type { MatViewDef } from "../../ipc/bindings/MatViewDef";
import type { MatViewRefresh } from "../../ipc/bindings/MatViewRefresh";
import type { ViewColumn } from "../../ipc/bindings/ViewColumn";
import type { ViewDef } from "../../ipc/bindings/ViewDef";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CheckRow, CommentField, FieldRow, FreeSqlNote, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { fillfactorFromInput, normalizeEmpty, normalizeTablespaceEdit, tablespaceChoices } from "./forms-model";
import {
  asMatView, asView, checkOptionFromSelect, lossList, matviewEditPatch, matviewRebuild, recreateVisible, refreshChoices,
  viewEditPatch, withDataPatch, withQuery,
} from "./view-model";

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

function ColumnList(props: { columns: readonly ViewColumn[] }) {
  return (
    <Show when={props.columns.length > 0}>
      <Section title={tview.sections.columns}>
        <ul class="flex flex-col gap-0.5 rounded border border-border px-3 py-2 text-[13px]">
          <For each={props.columns}>
            {(c) => (
              <li class="flex gap-2">
                <span class="font-mono">{c.name}</span>
                <span class="text-muted">{c.dataType}</span>
              </li>
            )}
          </For>
        </ul>
      </Section>
    </Show>
  );
}

function LossNote(props: { items: string[] }) {
  return (
    <div role="note" class="rounded border border-border px-3 py-2 text-xs text-warn-text">
      <p class="font-medium">{tview.lossTitle}</p>
      <ul class="list-disc pl-4">
        <For each={props.items}>{(i) => <li>{i}</li>}</For>
      </ul>
    </div>
  );
}

export default function ViewForm(props: FormProps) {
  const isMv = () => props.model.draft?.kind === "materialized_view";
  const view = () => asView(props.model.draft);
  const mv = () => asMatView(props.model.draft);
  // Ortak alanlar için tür bağımsız taslak (kind'e göre daraltılmış).
  const common = (): ViewDef | MatViewDef | null => view() ?? mv();
  const alter = () => props.model.mode === "alter";
  const originalView = (): ViewDef | null => asView(props.model.original);
  const originalMv = (): MatViewDef | null => asMatView(props.model.original);
  const original = (): ViewDef | MatViewDef | null => originalView() ?? originalMv();

  const patchCommon = (p: Partial<ViewDef & MatViewDef>) =>
    props.designer.edit(props.tabId, (d) => (d.kind === "view" ? viewEditPatch(d, p) : matviewEditPatch(d, p)));
  const patchView = (p: Partial<ViewDef>) => props.designer.edit(props.tabId, (d) => viewEditPatch(d, p));
  const patchMv = (p: Partial<MatViewDef>) => props.designer.edit(props.tabId, (d) => matviewEditPatch(d, p));

  const schemaList = createUniqueId();
  const noteId = createUniqueId();
  const escId = createUniqueId();
  const corId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  const [tablespaces] = createResource(
    () => (isMv() ? props.tabId : null),
    (tabId) => props.designer.lookup(tabId, { kind: "tablespaces" }).catch(() => []),
  );

  const editQuery = (q: string) =>
    props.designer.edit(props.tabId, (d) => {
      if (d.kind === "view") return { ...withQuery(d, originalView(), q), kind: "view" };
      if (d.kind === "materialized_view") {
        const next = withQuery(d, originalMv(), q);
        return { ...next, refresh: refreshChoices(originalMv(), next).includes(next.refresh) ? next.refresh : null, kind: "materialized_view" };
      }
      return d;
    });

  const refreshLabel = (r: MatViewRefresh | null) =>
    r === null ? tview.mv.refreshNone : r === "normal" ? tview.mv.refreshNormal : tview.mv.refreshConcurrently;

  return (
    <Show when={common()}>
      {(d) => (
        <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
          <Section title={tview.sections.general}>
            <FieldRow
              label={tview.general.schema}
              path="schema"
              issues={props.issues}
              hint={alter() ? tview.general.schemaAlterHint : tview.general.schemaCreateHint}
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
                    disabled={props.readOnly}
                    value={d().schema}
                    onInput={(ev) => patchCommon({ schema: ev.currentTarget.value })}
                  />
                  <datalist id={schemaList}>
                    <For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>
                  </datalist>
                </>
              )}
            </FieldRow>
            <FieldRow label={alter() ? tview.general.rename : tview.general.name} path="name" issues={props.issues}>
              {(a) => (
                <input
                  id={a.id}
                  ref={nameEl}
                  class={inputClass + invalidClass(a.invalid)}
                  autocomplete="off"
                  spellcheck={false}
                  required
                  aria-required="true"
                  placeholder={tview.general.namePlaceholder}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
                  value={d().name}
                  onInput={(ev) => patchCommon({ name: ev.currentTarget.value })}
                />
              )}
            </FieldRow>
            <OwnerSelect
              designer={props.designer}
              tabId={props.tabId}
              value={d().owner}
              issues={props.issues}
              disabled={props.readOnly}
              onChange={(v) => patchCommon({ owner: v })}
            />
            <CommentField
              value={d().comment}
              issues={props.issues}
              disabled={props.readOnly}
              onChange={(v) => patchCommon({ comment: v === null ? null : normalizeEmpty(v) })}
            />
          </Section>

          <Section title={tview.sections.query}>
            <FieldRow label={tview.query.label} path="query" issues={props.issues} hint={tview.query.hint}>
              {(a) => (
                <textarea
                  id={a.id}
                  rows={12}
                  wrap="off"
                  spellcheck={false}
                  autocomplete="off"
                  required
                  aria-required="true"
                  class={`${inputClass} !h-auto resize-y overflow-x-auto py-1 font-mono leading-snug${invalidClass(a.invalid)}`}
                  aria-describedby={[a.describedBy, noteId, escId, alter() && !isMv() && originalView() && d().query !== originalView()!.query ? corId : null]
                    .filter(Boolean)
                    .join(" ")}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
                  value={d().query}
                  onInput={(ev) => editQuery(ev.currentTarget.value)}
                />
              )}
            </FieldRow>
            <FreeSqlNote id={noteId} />
            <p id={escId} class="text-xs text-muted">
              {tview.query.escapeHint}
            </p>
            <Show when={alter() && !isMv() && view() && originalView() && d().query !== originalView()!.query}>
              <p id={corId} role="note" class="text-xs text-warn-text">
                {tview.query.corNote}
              </p>
            </Show>
            <Show when={isMv() && mv() && matviewRebuild(originalMv(), mv()!)}>
              <p role="note" class="text-xs text-warn-text">
                {tview.query.rebuildWarning}
              </p>
              <LossNote items={lossList(original())} />
            </Show>
          </Section>

          <Show when={alter()}>
            <ColumnList columns={original()?.columns ?? []} />
          </Show>

          <Show when={view()}>
            {(v) => (
              <Section title={tview.sections.options}>
                <FieldRow label={tview.view.checkOption} path="checkOption" issues={props.issues}>
                  {(a) => (
                    <select
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={props.readOnly}
                      value={v().checkOption ?? ""}
                      onChange={(ev) => patchView({ checkOption: checkOptionFromSelect(ev.currentTarget.value) })}
                    >
                      <option value="">{tview.view.checkNone}</option>
                      <option value="local">{tview.view.checkLocal}</option>
                      <option value="cascaded">{tview.view.checkCascaded}</option>
                    </select>
                  )}
                </FieldRow>
                <CheckRow
                  label={tview.view.securityBarrier}
                  path="securityBarrier"
                  issues={props.issues}
                  disabled={props.readOnly}
                  checked={v().securityBarrier}
                  onChange={(c) => patchView({ securityBarrier: c })}
                />
                <CheckRow
                  label={tview.view.securityInvoker}
                  path="securityInvoker"
                  issues={props.issues}
                  disabled={props.readOnly}
                  checked={v().securityInvoker}
                  onChange={(c) => patchView({ securityInvoker: c })}
                />
                <Show when={recreateVisible(props.model.mode, originalView(), v())}>
                  <CheckRow
                    label={tview.view.recreate}
                    path="recreate"
                    issues={props.issues}
                    hint={tview.view.recreateNote}
                    disabled={props.readOnly}
                    checked={v().recreate}
                    onChange={(c) => patchView({ recreate: c })}
                  />
                  <Show when={v().recreate}>
                    <LossNote items={lossList(originalView())} />
                  </Show>
                </Show>
              </Section>
            )}
          </Show>

          <Show when={mv()}>
            {(m) => {
              const choices = () => refreshChoices(originalMv(), m());
              return (
                <Section title={tview.sections.storage}>
                  <FieldRow label={tview.mv.tablespace} path="tablespace" issues={props.issues} hint={tview.mv.tablespaceHint}>
                    {(a) => (
                      <select
                        id={a.id}
                        class={inputClass + invalidClass(a.invalid)}
                        aria-describedby={a.describedBy}
                        aria-invalid={a.invalid}
                        disabled={props.readOnly}
                        value={m().tablespace ?? ""}
                        onChange={(ev) =>
                          patchMv({ tablespace: normalizeTablespaceEdit(originalMv()?.tablespace ?? null, ev.currentTarget.value) })
                        }
                      >
                        <Show when={(originalMv()?.tablespace ?? null) === null}>
                          <option value="">{tview.mv.tablespaceUnset}</option>
                        </Show>
                        <option value="pg_default">{tview.mv.tablespaceDefault}</option>
                        <For each={tablespaceChoices(tablespaces() ?? [], m().tablespace)}>{(t) => <option value={t}>{t}</option>}</For>
                      </select>
                    )}
                  </FieldRow>
                  <FieldRow label={tview.mv.fillfactor} path="fillfactor" issues={props.issues} hint={tview.mv.fillfactorHint}>
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
                        disabled={props.readOnly}
                        value={m().fillfactor ?? ""}
                        onInput={(ev) => {
                          const v = fillfactorFromInput(ev.currentTarget.value, ev.currentTarget.validity.badInput);
                          if (v !== undefined) patchMv({ fillfactor: v });
                        }}
                      />
                    )}
                  </FieldRow>
                  <CheckRow
                    label={tview.mv.withData}
                    path="withData"
                    issues={props.issues}
                    hint={m().withData ? undefined : tview.mv.withNoDataNote}
                    disabled={props.readOnly}
                    checked={m().withData}
                    onChange={(c) =>
                      props.designer.edit(props.tabId, (x) =>
                        x.kind === "materialized_view" ? { ...withDataPatch(originalMv(), x, c), kind: "materialized_view" } : x,
                      )
                    }
                  />
                  <Show when={choices().length > 1}>
                    <FieldRow label={tview.mv.refresh} path="refresh" issues={props.issues}>
                      {(a) => (
                        <select
                          id={a.id}
                          class={inputClass + invalidClass(a.invalid)}
                          aria-describedby={a.describedBy}
                          aria-invalid={a.invalid}
                          disabled={props.readOnly}
                          value={m().refresh ?? ""}
                          onChange={(ev) => patchMv({ refresh: (ev.currentTarget.value || null) as MatViewRefresh | null })}
                        >
                          <For each={choices()}>{(r) => <option value={r ?? ""}>{refreshLabel(r)}</option>}</For>
                        </select>
                      )}
                    </FieldRow>
                  </Show>
                </Section>
              );
            }}
          </Show>
        </form>
      )}
    </Show>
  );
}
