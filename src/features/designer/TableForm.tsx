// Table formu (tembel parça; Genel/Kolonlar/Kısıtlar/Bölümleme bölümleri bu parçaya statik içe aktarılır): bölüm
// sekmeleri (hata rozeti), odak dağıtıcı (createTableFormState) ve bölüm gövdeleri.
//
// Sekme çubuğu burada elle kurulur (WAI-ARIA tabs: ok tuşları/Home/End ile otomatik etkinleştirme, gezici tabindex):
// `@kobalte/core/tabs`'ı bu parçadan doğrudan içe aktarmak paketleyicinin ortak kodu ana paketten ayrı, açılışta yüklenen
// bir parçaya bölmesine yol açıyor; `src/ui/Tabs` ise etikete rozet koyamıyor ve ana pakette olduğu için büyütülemez.
//
// Bölüm paneli kalıbı: TÜM bölümler bağlı kalır, seçili olmayanlar `hidden` (kaydırma ve iç durum korunur; sekme
// değişimi bileşen kurmaz). Her panelin kendi `<Suspense>`'i vardır: paneldeki lookup kaynakları DesignerView'ın form
// `<Suspense>`'ini (ve sekme çubuğunu) asla yeniden askıya almaz. Yeni bölüm = `PANELS`'e bir `Component<TableSectionProps>`.
import { For, Show, Suspense, createUniqueId, mergeProps, type Component } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { ttbl } from "../../app/strings-table";
import type { FormProps } from "./FormParts";
import { TableColumns } from "./TableColumns";
import { TableGeneral, TablePartitioning } from "./TableGeneral";
import { createTableFormState, type TableSectionProps } from "./table-form-state";
import { asTable, sectionHasIssues, tableSections, type TableSection } from "./table-model";

/** Task 7 gövdesi gelene dek yer tutucu (başlık). */
const pending = (s: "constraints"): Component<TableSectionProps> => () => (
  <h2 class="text-[13px] font-medium">{ttbl.sections[s]}</h2>
);

/** Bölüm → gövde. Bölümün görünürlüğü `tableSections(mode)`'dan gelir (partitioning yalnızca create). */
const PANELS: Record<TableSection, Component<TableSectionProps>> = {
  general: TableGeneral,
  columns: TableColumns,
  constraints: pending("constraints"),
  partitioning: TablePartitioning,
};

export default function TableForm(props: FormProps) {
  const draft = () => asTable(props.model.draft);
  const original = () => asTable(props.model.original);
  const state = createTableFormState(props);
  const base = createUniqueId();
  const tabId = (s: TableSection) => `${base}-tab-${s}`;
  const panelId = (s: TableSection) => `${base}-panel-${s}`;
  const tabEls = new Map<TableSection, HTMLButtonElement>();
  const sections = () => tableSections(props.model.mode);
  // Bölüm listesi değişirse (create → alter sonrası partitioning kalkar) seçili bölüm listede kalmalı.
  const current = (): TableSection => (sections().includes(state.section()) ? state.section() : "general");

  const onKeyDown = (ev: KeyboardEvent) => {
    const list = sections();
    const at = list.indexOf(current());
    const next =
      ev.key === "ArrowRight" ? list[(at + 1) % list.length]
      : ev.key === "ArrowLeft" ? list[(at - 1 + list.length) % list.length]
      : ev.key === "Home" ? list[0]
      : ev.key === "End" ? list[list.length - 1]
      : undefined;
    if (!next) return;
    ev.preventDefault();
    state.setSection(next);
    tabEls.get(next)?.focus();
  };

  /** "Satır bulunamadı" şeridi (kolon/kısıt odağı; vurgu yeniden yüklemede temizlenir). */
  const missing = (s: TableSection) => {
    const h = state.highlight();
    if (!h || h.section !== s || h.index !== null) return null;
    return s === "columns" ? ttbl.focus.columnMissing : ttbl.focus.constraintMissing;
  };

  return (
    <Show when={draft()}>
      {(d) => {
        const sectionProps = mergeProps(props, {
          get draft() {
            return d();
          },
          get original() {
            return original();
          },
          editor: state.editor,
          get highlight() {
            return state.highlight();
          },
        });
        return (
          <div class="flex min-h-0 flex-col gap-3">
            <div role="tablist" aria-label={ttbl.sections.ariaLabel} class="flex shrink-0 gap-1 border-b border-border" onKeyDown={onKeyDown}>
              <For each={sections()}>
                {(s) => {
                  const selected = () => current() === s;
                  return (
                    <button
                      type="button"
                      role="tab"
                      id={tabId(s)}
                      ref={(el) => tabEls.set(s, el)}
                      aria-selected={selected()}
                      aria-controls={panelId(s)}
                      tabIndex={selected() ? 0 : -1}
                      class="-mb-px inline-flex items-center gap-1.5 border-b-2 px-2 py-1.5 text-[13px]"
                      classList={{ "border-accent text-fg": selected(), "border-transparent text-muted": !selected() }}
                      onClick={() => state.setSection(s)}
                    >
                      {ttbl.sections[s]}
                      <Show when={sectionHasIssues(props.issues, s)}>
                        <span aria-hidden="true" class="h-1.5 w-1.5 rounded-full bg-danger" />
                        <span class="sr-only">{` (${ttbl.sections.hasIssues})`}</span>
                      </Show>
                    </button>
                  );
                }}
              </For>
            </div>
            <For each={sections()}>
              {(s) => {
                const Body = PANELS[s];
                return (
                  <div role="tabpanel" id={panelId(s)} aria-labelledby={tabId(s)} tabIndex={0} hidden={current() !== s}>
                    <div class="flex flex-col gap-3">
                      <Show when={missing(s)}>
                        {(text) => (
                          <div role="status" class="rounded border border-border px-3 py-2 text-[13px] text-warn-text">
                            {text()}
                          </div>
                        )}
                      </Show>
                      <Suspense fallback={<p class="text-muted">{tdes.loading}</p>}>
                        <Body {...sectionProps} active={current() === s} />
                      </Suspense>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        );
      }}
    </Show>
  );
}
