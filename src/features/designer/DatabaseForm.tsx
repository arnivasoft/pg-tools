// Database formu (tembel parça): Genel, Oluşturma seçenekleri (create'te düzenlenebilir; alter'da salt okunur), Bağlantı ve
// depolama, Ayarlar (ortak SettingsEditor). İstemci SQL üretmez; değerler olduğu gibi gider. Kobalte/ikon yok.
import { For, Show, createResource, createUniqueId, type JSX } from "solid-js";
import { tdb } from "../../app/strings-database";
import type { LocaleProvider } from "../../ipc/bindings/LocaleProvider";
import { inputClass } from "../../ui/Field";
import {
  asDatabase, connectionLimitFromInput, createOnlyLocked, unlimitedToggle, databaseEditPatch, localeFields, needsNoSessions, template0Hint, withProvider,
} from "./database-model";
import { issuesFor, type FieldIssues } from "./designer-model";
import { CheckRow, CommentField, FieldRow, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { normalizeEmpty, normalizeTablespaceEdit, tablespaceChoices } from "./forms-model";
import { SettingsEditor } from "./SettingsEditor";
import type { DatabaseDef } from "../../ipc/bindings/DatabaseDef";

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

// Sınırsız kutusunun hatası yok: connectionLimit hataları sayı alanında gösterilir.
const NO_ISSUES: FieldIssues = { byPath: new Map(), general: [], generalPaths: [] };
const providerOf = (v: string): LocaleProvider | null => (v === "libc" || v === "icu" || v === "builtin" ? v : null);

export default function DatabaseForm(props: FormProps) {
  const draft = () => asDatabase(props.model.draft);
  const original = (): DatabaseDef | null => asDatabase(props.model.original);
  const locked = () => createOnlyLocked(props.model.mode);
  const alter = () => props.model.mode === "alter";
  const patch = (p: Partial<DatabaseDef>) => props.designer.edit(props.tabId, (d) => databaseEditPatch(d, p));
  // Salt okunur alanlar zaten kilitli; kaydın kendisi readOnly ise tüm alanlar kapalıdır.
  const off = () => props.readOnly;

  const templateList = createUniqueId();
  const lcCollateList = createUniqueId();
  const lcCtypeList = createUniqueId();
  const providerLocaleList = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const [templates] = createResource(
    () => (locked() ? null : props.tabId),
    (tabId) => props.designer.lookup(tabId, { kind: "template_databases" }).catch(() => []),
  );
  const [encodings] = createResource(
    () => (locked() ? null : props.tabId),
    (tabId) => props.designer.lookup(tabId, { kind: "encodings" }).catch(() => []),
  );
  const [libcLocales] = createResource(
    () => (locked() ? null : props.tabId),
    (tabId) => props.designer.lookup(tabId, { kind: "locales", provider: "libc" }).catch(() => []),
  );
  const [providerLocales] = createResource(
    () => {
      const p = draft()?.localeProvider ?? null;
      return !locked() && (p === "icu" || p === "builtin") ? { tabId: props.tabId, p } : null;
    },
    (k) => props.designer.lookup(k.tabId, { kind: "locales", provider: k.p }).catch(() => []),
  );
  const [tablespaces] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "tablespaces" }).catch(() => []),
  );

  return (
    <Show when={draft()}>
      {(d) => {
        const lf = () => localeFields(d().localeProvider);
        const unlimited = () => d().connectionLimit < 0;
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <Show when={needsNoSessions(original(), d())}>
              <p role="note" class="rounded border border-border px-3 py-2 text-xs text-warn-text">
                {tdb.noSessions}
              </p>
            </Show>

            <Section title={tdb.sections.general}>
              <FieldRow
                label={alter() ? tdb.general.rename : tdb.general.name}
                path="name"
                issues={props.issues}
                hint={alter() ? tdb.general.renameNote : undefined}
              >
                {(a) => (
                  <input
                    id={a.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(a.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={tdb.general.namePlaceholder}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={off()}
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
                disabled={off()}
                onChange={(v) => patch({ owner: v })}
              />
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={off()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
            </Section>

            <Section title={tdb.sections.createOptions}>
              <p class="text-xs text-muted">{tdb.createOptions.note}</p>
              <FieldRow label={tdb.createOptions.template} path="template" issues={props.issues} hint={locked() ? undefined : tdb.createOptions.templateHint}>
                {(a) => (
                  <>
                    <input
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      list={locked() ? undefined : templateList}
                      autocomplete="off"
                      spellcheck={false}
                      readOnly={locked()}
                      placeholder={locked() ? tdb.createOptions.unset : "template1"}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={off()}
                      value={d().template ?? ""}
                      onInput={(ev) => patch({ template: normalizeEmpty(ev.currentTarget.value) })}
                    />
                    <datalist id={templateList}>
                      <For each={templates() ?? []}>{(i) => <option value={i.value} />}</For>
                    </datalist>
                  </>
                )}
              </FieldRow>
              <FieldRow label={tdb.createOptions.encoding} path="encoding" issues={props.issues}>
                {(a) => (
                  <Show
                    when={!locked()}
                    fallback={
                      <input
                        id={a.id}
                        class={inputClass + invalidClass(a.invalid)}
                        readOnly
                        placeholder={tdb.createOptions.unset}
                        aria-describedby={a.describedBy}
                        aria-invalid={a.invalid}
                        disabled={off()}
                        value={d().encoding ?? ""}
                      />
                    }
                  >
                    <select
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={off()}
                      value={d().encoding ?? ""}
                      onChange={(ev) => patch({ encoding: normalizeEmpty(ev.currentTarget.value) })}
                    >
                      <option value="">{tdb.createOptions.encodingDefault}</option>
                      <For each={encodings() ?? []}>{(e) => <option value={e.value}>{e.value}</option>}</For>
                    </select>
                  </Show>
                )}
              </FieldRow>
              <FieldRow label={tdb.createOptions.provider} path="localeProvider" issues={props.issues}>
                {(a) => (
                  <Show
                    when={!locked()}
                    fallback={
                      <input
                        id={a.id}
                        class={inputClass + invalidClass(a.invalid)}
                        readOnly
                        placeholder={tdb.createOptions.unset}
                        aria-describedby={a.describedBy}
                        aria-invalid={a.invalid}
                        disabled={off()}
                        value={d().localeProvider ?? ""}
                      />
                    }
                  >
                    <select
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={off()}
                      value={d().localeProvider ?? ""}
                      onChange={(ev) => props.designer.edit(props.tabId, (x) => (x.kind === "database" ? { ...withProvider(x, providerOf(ev.currentTarget.value)), kind: "database" } : x))}
                    >
                      <option value="">{tdb.createOptions.providerDefault}</option>
                      <option value="libc">{tdb.createOptions.providerLibc}</option>
                      <option value="icu">{tdb.createOptions.providerIcu}</option>
                      <option value="builtin">{tdb.createOptions.providerBuiltin}</option>
                    </select>
                  </Show>
                )}
              </FieldRow>
              <FieldRow label={tdb.createOptions.lcCollate} path="lcCollate" issues={props.issues}>
                {(a) => (
                  <>
                    <input
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      list={locked() ? undefined : lcCollateList}
                      autocomplete="off"
                      spellcheck={false}
                      readOnly={locked()}
                      placeholder={locked() ? tdb.createOptions.unset : undefined}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={off()}
                      value={d().lcCollate ?? ""}
                      onInput={(ev) => patch({ lcCollate: normalizeEmpty(ev.currentTarget.value) })}
                    />
                    <datalist id={lcCollateList}>
                      <For each={libcLocales() ?? []}>{(i) => <option value={i.value} />}</For>
                    </datalist>
                  </>
                )}
              </FieldRow>
              <FieldRow label={tdb.createOptions.lcCtype} path="lcCtype" issues={props.issues}>
                {(a) => (
                  <>
                    <input
                      id={a.id}
                      class={inputClass + invalidClass(a.invalid)}
                      list={locked() ? undefined : lcCtypeList}
                      autocomplete="off"
                      spellcheck={false}
                      readOnly={locked()}
                      placeholder={locked() ? tdb.createOptions.unset : undefined}
                      aria-describedby={a.describedBy}
                      aria-invalid={a.invalid}
                      disabled={off()}
                      value={d().lcCtype ?? ""}
                      onInput={(ev) => patch({ lcCtype: normalizeEmpty(ev.currentTarget.value) })}
                    />
                    <datalist id={lcCtypeList}>
                      <For each={libcLocales() ?? []}>{(i) => <option value={i.value} />}</For>
                    </datalist>
                  </>
                )}
              </FieldRow>
              <Show when={locked() ? d().localeProvider === "icu" || d().localeProvider === "builtin" || d().providerLocale !== null : lf().providerLocale !== null || d().providerLocale !== null || issuesFor(props.issues, "providerLocale").length > 0}>
                <FieldRow
                  label={(locked() ? d().localeProvider : lf().providerLocale) === "builtin" ? tdb.createOptions.builtinLocale : tdb.createOptions.icuLocale}
                  path="providerLocale"
                  issues={props.issues}
                >
                  {(a) => (
                    <>
                      <input
                        id={a.id}
                        class={inputClass + invalidClass(a.invalid)}
                        list={locked() ? undefined : providerLocaleList}
                        autocomplete="off"
                        spellcheck={false}
                        readOnly={locked()}
                        placeholder={locked() ? tdb.createOptions.unset : undefined}
                        aria-describedby={a.describedBy}
                        aria-invalid={a.invalid}
                        disabled={off()}
                        value={d().providerLocale ?? ""}
                        onInput={(ev) => patch({ providerLocale: normalizeEmpty(ev.currentTarget.value) })}
                      />
                      <datalist id={providerLocaleList}>
                        <For each={providerLocales() ?? []}>{(i) => <option value={i.value}>{i.detail ?? ""}</option>}</For>
                      </datalist>
                    </>
                  )}
                </FieldRow>
              </Show>
              <Show when={!locked() && template0Hint(d())}>
                <p role="note" class="text-xs text-warn-text">
                  {tdb.createOptions.template0Hint}
                </p>
              </Show>
              <Show when={locked()}>
                <p class="text-xs text-muted">{tdb.localeNormalized}</p>
              </Show>
            </Section>

            <Section title={tdb.sections.storage}>
              <FieldRow
                label={tdb.storage.tablespace}
                path="tablespace"
                issues={props.issues}
                hint={tdb.storage.tablespaceHint}
                warning={alter() && original() && original()!.tablespace !== d().tablespace ? tdb.storage.tablespaceMoveNote : null}
              >
                {(a) => (
                  <select
                    id={a.id}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={off()}
                    value={d().tablespace ?? ""}
                    onChange={(ev) => patch({ tablespace: normalizeTablespaceEdit(original()?.tablespace ?? null, ev.currentTarget.value) })}
                  >
                    <Show when={(original()?.tablespace ?? null) === null}>
                      <option value="">{tdb.storage.tablespaceUnset}</option>
                    </Show>
                    <option value="pg_default">{tdb.storage.tablespaceDefault}</option>
                    <For each={tablespaceChoices(tablespaces() ?? [], d().tablespace)}>{(t) => <option value={t}>{t}</option>}</For>
                  </select>
                )}
              </FieldRow>
              <FieldRow label={tdb.storage.connectionLimit} path="connectionLimit" issues={props.issues} hint={tdb.storage.unlimitedHint}>
                {(a) => (
                  <input
                    id={a.id}
                    type="number"
                    min={0}
                    step={1}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={off() || unlimited()}
                    value={unlimited() ? "" : d().connectionLimit}
                    onInput={(ev) => {
                      const v = connectionLimitFromInput(ev.currentTarget.value, ev.currentTarget.validity.badInput);
                      if (v !== undefined) patch({ connectionLimit: v });
                    }}
                  />
                )}
              </FieldRow>
              <CheckRow
                label={tdb.storage.unlimited}
                path="connectionLimit"
                issues={NO_ISSUES}
                disabled={off()}
                checked={unlimited()}
                onChange={(c) => patch({ connectionLimit: unlimitedToggle(original(), c) })}
              />
              <CheckRow
                label={tdb.storage.allowConnections}
                path="allowConnections"
                issues={props.issues}
                warning={d().allowConnections ? null : tdb.storage.allowConnectionsWarn}
                disabled={off()}
                checked={d().allowConnections}
                onChange={(c) => patch({ allowConnections: c })}
              />
              <CheckRow
                label={tdb.storage.isTemplate}
                path="isTemplate"
                issues={props.issues}
                warning={d().isTemplate ? tdb.storage.isTemplateNote : null}
                disabled={off()}
                checked={d().isTemplate}
                onChange={(c) => patch({ isTemplate: c })}
              />
            </Section>

            <SettingsEditor
              {...props}
              path="settings"
              list={d().settings}
              original={original()?.settings ?? null}
              onChange={(l) => patch({ settings: l })}
            />
          </form>
        );
      }}
    </Show>
  );
}
