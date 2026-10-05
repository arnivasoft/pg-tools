import { For } from "solid-js";
import type { PropertySection } from "../../ipc/bindings/PropertySection";

export function PropertiesView(props: { sections: PropertySection[] }) {
  return (
    <div class="flex flex-col gap-4 p-3">
      <For each={props.sections}>
        {(section) => (
          <section>
            <h3 class="mb-1 text-xs font-semibold uppercase tracking-wide text-muted">{section.title}</h3>
            <dl class="grid grid-cols-[minmax(120px,max-content)_1fr] gap-x-4 gap-y-1">
              <For each={section.rows}>
                {(row) => (
                  <>
                    <dt class="text-muted">{row.label}</dt>
                    <dd class="m-0 break-words select-text">{row.value}</dd>
                  </>
                )}
              </For>
            </dl>
          </section>
        )}
      </For>
    </div>
  );
}
