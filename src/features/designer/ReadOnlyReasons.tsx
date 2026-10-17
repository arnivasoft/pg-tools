// Salt okunur tanımın gerekçe şeridi (Plan 19 Karar 3; tembel parça): `editable = false` ise formun üstünde "Bu nesne
// buradan düzenlenemez" + arka ucun `unsupported` gerekçeleri (insan okunur sabit metin; olduğu gibi gösterilir).
// Görünüm mevcut "Düzenlenmeyenler" listeleriyle (TableConstraints/RoleForm) aynı kalıptadır.
import { For, Show, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";

export function ReadOnlyReasons(props: { def: { editable: boolean; unsupported: string[] }; id?: string }) {
  const titleId = createUniqueId();
  return (
    <Show when={!props.def.editable}>
      <section
        id={props.id}
        role="note"
        aria-labelledby={titleId}
        class="flex flex-col gap-1 rounded border border-[color:var(--srv-amber)] bg-panel px-3 py-2"
      >
        <h3 id={titleId} class="text-[13px] font-medium text-warn-text">
          {tdes.readOnly.title}
        </h3>
        <p class="text-xs text-muted">{tdes.readOnly.note}</p>
        <Show when={props.def.unsupported.length > 0}>
          <ul role="list" aria-label={tdes.readOnly.reasons} class="flex flex-col gap-1">
            <For each={props.def.unsupported}>
              {(u) => <li class="rounded border border-border px-2 py-1 text-[13px]">{u}</li>}
            </For>
          </ul>
        </Show>
      </section>
    </Show>
  );
}
