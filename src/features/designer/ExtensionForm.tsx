// Extension formu (tembel parça): ad (create: kurulabilir eklenti listesi + serbest yazım; alter: salt okunur), şema,
// sürüm (alter'da farklı sürüm → UPDATE TO notu) ve yalnız create'te CASCADE.
import { For, Show, createResource, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { inputClass } from "../../ui/Field";
import { FieldRow, invalidClass, type FormProps } from "./FormParts";
import { asExtension, extensionEditPatch, extensionOptionLabel, extensionVersionNote, findExtension, normalizeEmpty } from "./forms-model";

export default function ExtensionForm(props: FormProps) {
  const f = tdes.extension;
  const draft = () => asExtension(props.model.draft);
  const alter = () => props.model.mode === "alter";
  const patch = (p: Parameters<typeof extensionEditPatch>[1]) => props.designer.edit(props.tabId, (d) => extensionEditPatch(d, p));
  const nameList = createUniqueId();
  const schemaList = createUniqueId();
  const versionList = createUniqueId();

  // Lookup hataları listeyi boş bırakır; serbest yazım sürer.
  const [available] = createResource(
    () => (alter() ? null : props.tabId),
    (tabId) => props.designer.lookup(tabId, { kind: "available_extensions" }).catch(() => []),
  );
  const [schemas] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "schemas" }).catch(() => []),
  );
  // Sürüm listesi: alter'da yüklü eklenti adıyla; create'te yalnızca yazılan ad kurulabilir bir eklentiyse (boş sorgu yok).
  const versionName = (): string | null => {
    const d = draft();
    if (!d || d.name === "") return null;
    if (alter()) return d.name;
    return findExtension(available() ?? [], d.name) ? d.name : null;
  };
  const [versions] = createResource(versionName, (name) =>
    props.designer.lookup(props.tabId, { kind: "extension_versions", name }).catch(() => []),
  );
  const picked = () => {
    const d = draft();
    return d && !alter() ? findExtension(available() ?? [], d.name) : null;
  };
  const showNote = () => {
    const d = draft();
    return d !== null && extensionVersionNote(props.model.original, d);
  };

  return (
    <Show when={draft()}>
      {(d) => (
        <form class="flex max-w-xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
          <FieldRow
            label={f.name}
            path="name"
            issues={props.issues}
            hint={alter() ? f.nameFixed : picked()?.detail ?? f.nameCreateHint}
          >
            {(a) => (
              <>
                <input
                  id={a.id}
                  class={inputClass + invalidClass(a.invalid)}
                  list={alter() ? undefined : nameList}
                  autocomplete="off"
                  spellcheck={false}
                  required
                  aria-required="true"
                  placeholder={f.namePlaceholder}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  readOnly={alter()}
                  disabled={props.readOnly}
                  value={d().name}
                  onInput={(ev) => patch({ name: ev.currentTarget.value })}
                />
                <Show when={!alter()}>
                  <datalist id={nameList}>
                    <For each={available() ?? []}>{(i) => <option value={i.value} label={extensionOptionLabel(i)} />}</For>
                  </datalist>
                </Show>
              </>
            )}
          </FieldRow>
          <FieldRow label={f.schema} path="schema" issues={props.issues} hint={alter() ? f.schemaAlterHint : f.schemaHint}>
            {(a) => (
              <>
                <input
                  id={a.id}
                  class={inputClass + invalidClass(a.invalid)}
                  list={schemaList}
                  autocomplete="off"
                  spellcheck={false}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
                  value={d().schema ?? ""}
                  onInput={(ev) => patch({ schema: normalizeEmpty(ev.currentTarget.value) })}
                />
                <datalist id={schemaList}>
                  <For each={schemas() ?? []}>{(i) => <option value={i.value} />}</For>
                </datalist>
              </>
            )}
          </FieldRow>
          <FieldRow
            label={f.version}
            path="version"
            issues={props.issues}
            hint={showNote() ? f.versionNote : alter() ? f.versionAlterHint : f.versionCreateHint}
          >
            {(a) => (
              <>
                <input
                  id={a.id}
                  class={inputClass + invalidClass(a.invalid)}
                  list={versionList}
                  autocomplete="off"
                  spellcheck={false}
                  aria-describedby={a.describedBy}
                  aria-invalid={a.invalid}
                  disabled={props.readOnly}
                  value={d().version ?? ""}
                  onInput={(ev) => patch({ version: normalizeEmpty(ev.currentTarget.value) })}
                />
                <datalist id={versionList}>
                  <For each={versionName() === null ? [] : (versions() ?? [])}>{(i) => <option value={i.value} />}</For>
                </datalist>
              </>
            )}
          </FieldRow>
          <Show when={!alter()}>
            <label class="flex items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                disabled={props.readOnly}
                checked={d().cascade}
                onChange={(ev) => patch({ cascade: ev.currentTarget.checked })}
              />
              {f.cascade}
            </label>
          </Show>
        </form>
      )}
    </Show>
  );
}
