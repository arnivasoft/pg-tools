// Schema formu (tembel parça): ad (alter'da yeniden adlandırma; `focusField = "name"` odaklar), sahip, yorum.
import { Show } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { inputClass } from "../../ui/Field";
import { CommentField, FieldRow, OwnerSelect, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { asSchema, normalizeEmpty, schemaEditPatch } from "./forms-model";

export default function SchemaForm(props: FormProps) {
  const f = tdes.schema;
  const draft = () => asSchema(props.model.draft);
  const alter = () => props.model.mode === "alter";
  const patch = (p: Parameters<typeof schemaEditPatch>[1]) => props.designer.edit(props.tabId, (d) => schemaEditPatch(d, p));
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);
  return (
    <Show when={draft()}>
      {(d) => (
        <form class="flex max-w-xl flex-col gap-3" onSubmit={(ev) => ev.preventDefault()}>
          <FieldRow
            label={alter() ? f.rename : f.name}
            path="name"
            issues={props.issues}
            hint={alter() ? f.renameHint : undefined}
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
          <CommentField
            value={d().comment}
            issues={props.issues}
            disabled={props.readOnly}
            onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
          />
        </form>
      )}
    </Show>
  );
}
