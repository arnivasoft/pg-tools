// Kolon veri tipi girdisi (TableForm parçası): yazılabilir metin + `types` lookup önerileri (datalist; değer = tip, etiket
// = ayrıntı). Odaklanınca ve yazdıkça sorgulanır (createTypeSuggestions: 200 ms, son istek kazanır; denetleyici önbelleği).
// Yükleme sırasında datalist'e yer tutucu seçenek konmaz (datalist seçeneği değer sayılır); bunun yerine `aria-busy`.
import { For, createUniqueId, onCleanup } from "solid-js";
import { ttbl } from "../../app/strings-table";
import { inputClass } from "../../ui/Field";
import type { DesignerController } from "./designer-controller";
import { invalidClass } from "./FormParts";
import { createTypeSuggestions } from "./type-suggestions";

export function TypeInput(props: {
  designer: Pick<DesignerController, "lookup">;
  tabId: string;
  id: string;
  value: string;
  describedBy?: string;
  invalid: boolean;
  disabled: boolean;
  onInput: (v: string) => void;
}) {
  const listId = createUniqueId();
  const types = createTypeSuggestions((query) => props.designer.lookup(props.tabId, { kind: "types", query }));
  onCleanup(types.dispose);

  return (
    <>
      <input
        id={props.id}
        class={`${inputClass} font-mono${invalidClass(props.invalid)}`}
        list={listId}
        autocomplete="off"
        spellcheck={false}
        required
        aria-required="true"
        aria-busy={types.busy()}
        placeholder={ttbl.columns.dataTypePlaceholder}
        aria-describedby={props.describedBy}
        aria-invalid={props.invalid}
        disabled={props.disabled}
        value={props.value}
        onFocus={(ev) => types.query(ev.currentTarget.value)}
        onInput={(ev) => {
          const v = ev.currentTarget.value;
          props.onInput(v);
          types.query(v);
        }}
      />
      <datalist id={listId}>
        <For each={types.items()}>{(it) => <option value={it.value} label={it.detail ?? undefined} />}</For>
      </datalist>
    </>
  );
}
