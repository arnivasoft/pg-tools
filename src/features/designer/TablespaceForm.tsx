// Tablespace formu (tembel parça): ad (alter'da yeniden adlandırma), sahip, konum (yalnız create'te düzenlenir; alter'da
// salt okunur metin), dört seçenek (metin; boş = ayarsız; yalnız biçim ön uyarısı, asıl doğrulama arka uçta) ve yorum.
// İstemci SQL üretmez; `location` olduğu gibi gider (kırpılmaz). `editable = false` (sistem tablespace'i) ya da `readOnly`
// sekme tamamen salt okunurdur. Kobalte yok; metinler tembel `strings-tablespace`.
import { For, Show, createMemo, createUniqueId, type JSX } from "solid-js";
import { ttsp } from "../../app/strings-tablespace";
import type { TablespaceDef } from "../../ipc/bindings/TablespaceDef";
import { inputClass } from "../../ui/Field";
import { CommentField, FieldRow, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { issuesFor } from "./designer-model";
import { normalizeEmpty } from "./forms-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";
import {
  asTablespace, tablespaceOptionPatch, tablespaceOptionWarnings, tablespacePatch, type TablespaceOptionKey,
} from "./tablespace-model";

const OPTIONS: { key: TablespaceOptionKey; mode: "decimal" | "numeric"; hint?: string }[] = [
  { key: "seqPageCost", mode: "decimal" },
  { key: "randomPageCost", mode: "decimal" },
  { key: "effectiveIoConcurrency", mode: "numeric" },
  { key: "maintenanceIoConcurrency", mode: "numeric", hint: ttsp.options.maintenanceHint },
];

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

export default function TablespaceForm(props: FormProps) {
  const draft = () => asTablespace(props.model.draft);
  const alter = () => props.model.mode === "alter";
  const locked = () => props.readOnly || !(draft()?.editable ?? true);
  const patch = (p: Partial<TablespaceDef>) => props.designer.edit(props.tabId, (x) => tablespacePatch(x, p));
  const setOption = (key: TablespaceOptionKey, text: string) =>
    props.designer.edit(props.tabId, (x) => {
      const prev = asTablespace(x);
      return prev ? tablespacePatch(x, tablespaceOptionPatch(prev, key, text)) : x;
    });
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  return (
    <Show when={draft()}>
      {(d) => {
        const warnings = createMemo(() => tablespaceOptionWarnings(d().options));
        const warningText = (k: TablespaceOptionKey) => {
          const w = warnings()[k];
          return w ? ttsp.warnings[w] : null;
        };
        return (
          <form class="flex max-w-xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <Show when={issuesFor(props.issues, "object").length > 0}>
              <p role="alert" class="text-xs text-danger">
                {issuesFor(props.issues, "object").join(" ")}
              </p>
            </Show>
            <Show when={!alter()}>
              <p role="note" class="text-xs text-muted">
                {ttsp.createNote}
              </p>
            </Show>

            <Section title={ttsp.sections.general}>
              <FieldRow
                label={alter() ? ttsp.general.rename : ttsp.general.name}
                path="name"
                issues={props.issues}
                hint={ttsp.general.nameHint}
              >
                {(f) => (
                  <input
                    id={f.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(f.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={ttsp.general.namePlaceholder}
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
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
              <FieldRow
                label={ttsp.location.label}
                path="location"
                issues={props.issues}
                hint={alter() ? ttsp.location.alterHint : ttsp.location.createHint}
              >
                {(f) =>
                  alter() ? (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono text-muted${invalidClass(f.invalid)}`}
                      readOnly
                      aria-readonly="true"
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      value={d().location === "" ? ttsp.location.empty : d().location}
                    />
                  ) : (
                    <input
                      id={f.id}
                      class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                      autocomplete="off"
                      spellcheck={false}
                      required
                      aria-required="true"
                      placeholder={ttsp.location.placeholder}
                      aria-describedby={f.describedBy}
                      aria-invalid={f.invalid}
                      disabled={locked()}
                      value={d().location}
                      onInput={(ev) => patch({ location: ev.currentTarget.value })}
                    />
                  )
                }
              </FieldRow>
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={locked()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
            </Section>

            <Section title={ttsp.sections.options}>
              <For each={OPTIONS}>
                {(o) => (
                  <FieldRow
                    label={ttsp.options[o.key]}
                    path={`options.${o.key}`}
                    issues={props.issues}
                    hint={o.hint}
                    warning={warningText(o.key)}
                  >
                    {(f) => (
                      <input
                        id={f.id}
                        class={`${inputClass} font-mono${invalidClass(f.invalid)}`}
                        inputmode={o.mode}
                        autocomplete="off"
                        spellcheck={false}
                        placeholder={ttsp.options.unset}
                        aria-describedby={f.describedBy}
                        aria-invalid={f.invalid}
                        disabled={locked()}
                        value={d().options[o.key] ?? ""}
                        onInput={(ev) => setOption(o.key, ev.currentTarget.value)}
                      />
                    )}
                  </FieldRow>
                )}
              </For>
            </Section>
          </form>
        );
      }}
    </Show>
  );
}
