// Silme diyaloğu (tembel parça; sekme açmaz): yüklenen tanımın `Drop` planı, CASCADE seçimi, risk onayı ve Sil.
// Önizleme metni yalnızca `PlanView.statements[].sql`'dir (salt okunur `<pre>`; CodeMirror yok). Kapı (`drop.gate()`)
// yalnızca arayüz içindir; denetleyici `apply` anında yeniden hesaplar. Odak Vazgeç'te başlar; Enter silmeyi onaylamaz
// (onay kutusu ve Sil ayrı denetimlerdir, ad alanında Enter yutulur).
import { For, Show, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { inputClass } from "../../ui/Field";
import { canSubmit, kindLabel } from "./designer-model";
import type { DropController } from "./drop-controller";
import { RiskBadge } from "./RiskBadge";

export default function DropDialog(props: { drop: DropController }) {
  const d = tdes.drop;
  const b = tdes.bar;
  const nameId = createUniqueId();
  const cascadeHintId = createUniqueId();
  let cancelEl: HTMLButtonElement | undefined;
  const s = () => props.drop.state();
  const gate = () => props.drop.gate();
  const applying = () => s()?.phase === "applying";
  const locked = () => s()?.locked ?? false;
  const view = () => s()?.view ?? null;
  const needsConfirm = () => gate().kind === "needsConfirm" || gate().kind === "needsProductionName";
  const expectedName = () => {
    const g = gate();
    return g.kind === "needsProductionName" ? g.expected : null;
  };
  const submittable = () => {
    const cur = s();
    return cur !== null && cur.phase === "ready" && !cur.locked && canSubmit(gate(), cur.confirm);
  };
  /** Sil kapalıyken gerekçe. */
  const submitTitle = () => {
    if (locked()) return b.lockedTitle;
    const g = gate();
    if (g.kind === "blocked") return b.blocked[g.reason];
    if (submittable()) return undefined;
    return g.kind === "needsProductionName" ? b.needsName : b.needsConfirm;
  };

  return (
    <Dialog
      open={s() !== null}
      // Uygulama uçuştayken Escape/kapat yok sayılır (İptal düğmesi `ddl_cancel` ister).
      onOpenChange={(open) => !open && !applying() && props.drop.close()}
      title={s() ? d.title(kindLabel(s()!.input.object.kind), s()!.input.name) : ""}
      width="560px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Show
            when={applying()}
            fallback={
              <Button ref={cancelEl} onClick={() => props.drop.close()}>
                {s()?.phase === "error" ? d.close : d.cancel}
              </Button>
            }
          >
            <Button title={b.cancelTitle} onClick={() => props.drop.cancelApply()}>
              {d.cancelApply}
            </Button>
          </Show>
          <Button
            variant="primary"
            class="!bg-danger !text-danger-fg"
            disabled={!submittable()}
            title={submitTitle()}
            onClick={() => {
              // Çift tık: denetleyici uçuşta ikinci isteği zaten reddeder.
              if (submittable()) void props.drop.apply();
            }}
          >
            {applying() ? d.submitting : d.submit}
          </Button>
        </>
      }
    >
      <Show when={s()}>
        {(cur) => (
          <div class="flex flex-col gap-3 text-[13px]">
            <Show when={cur().locked}>
              <p class="text-muted">{tdes.gate.locked}</p>
            </Show>
            <Show when={cur().phase === "loading"}>
              <p class="text-muted" role="status">
                {d.loading}
              </p>
            </Show>
            <Show when={cur().status}>
              {(st) => (
                <div role="alert" class="flex flex-col gap-1 rounded border border-danger px-3 py-2">
                  <p class="font-medium text-danger">{st().text}</p>
                  <Show when={st().detail}>
                    {(detail) => (
                      <>
                        <Show when={st().kind === "dependents"}>
                          <p class="text-[12px] text-muted">{d.dependentsDetail}</p>
                        </Show>
                        <pre class="max-h-32 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px]">{detail()}</pre>
                      </>
                    )}
                  </Show>
                  <Show when={cur().phase === "error" && cur().canReload}>
                    <div>
                      <Button onClick={() => props.drop.reload()}>{d.reload}</Button>
                    </div>
                  </Show>
                </div>
              )}
            </Show>
            <Show when={cur().previewError}>
              {(e) => (
                <div role="alert" class="rounded border border-danger px-3 py-2">
                  <p class="font-medium text-danger">{d.previewError}</p>
                  <p class="text-muted">{e()}</p>
                </div>
              )}
            </Show>
            <Show when={cur().phase !== "loading" && cur().phase !== "error"}>
              <section aria-label={d.statements} classList={{ "opacity-70": cur().pending && view() !== null }}>
                <h3 class="mb-1 text-[12px] text-muted">{d.statements}</h3>
                <Show
                  when={view()}
                  fallback={
                    <p class="text-muted" role="status">
                      {cur().pending ? d.pending : "—"}
                    </p>
                  }
                >
                  {(v) => (
                    <ol class="flex flex-col gap-1">
                      <For each={v().statements}>
                        {(st) => (
                          <li class="flex items-start gap-2 rounded border border-border px-2 py-1">
                            <RiskBadge risk={st.risk} />
                            <div class="min-w-0 flex-1">
                              <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px]">{st.sql}</pre>
                              <Show when={st.note}>
                                <p class="text-[12px] text-muted">{st.note}</p>
                              </Show>
                            </div>
                          </li>
                        )}
                      </For>
                    </ol>
                  )}
                </Show>
                <Show when={(view()?.warnings.length ?? 0) > 0}>
                  <h3 class="mb-1 mt-2 text-[12px] text-muted">{d.warnings}</h3>
                  <ul class="list-disc pl-5 text-[12px] text-warn-text">
                    <For each={view()?.warnings ?? []}>{(w) => <li>{w}</li>}</For>
                  </ul>
                </Show>
              </section>
              <div class="flex flex-col gap-1">
                <label class="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={cur().cascade}
                    disabled={cur().phase !== "ready"}
                    aria-describedby={cascadeHintId}
                    onChange={(ev) => props.drop.setCascade(ev.currentTarget.checked)}
                  />
                  {d.cascade}
                </label>
                <p id={cascadeHintId} class="pl-6 text-[12px] text-muted">
                  {d.cascadeHint}
                </p>
              </div>
              <Show when={needsConfirm() && !cur().locked}>
                <div class="flex flex-col gap-1.5 border-t border-border pt-2">
                  <label class="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={cur().confirm.checked}
                      disabled={applying()}
                      onChange={(ev) => props.drop.setConfirm({ checked: ev.currentTarget.checked })}
                    />
                    {b.confirm}
                  </label>
                  <Show when={gate().kind === "needsProductionName"}>
                    <Show when={expectedName()} fallback={<p class="text-danger">{b.noServerName}</p>}>
                      {(name) => (
                        <div class="flex flex-col gap-1">
                          <label for={nameId}>{b.typeName(name())}</label>
                          <input
                            id={nameId}
                            class={inputClass}
                            autocomplete="off"
                            spellcheck={false}
                            disabled={applying()}
                            value={cur().confirm.typedName}
                            onInput={(ev) => props.drop.setConfirm({ typedName: ev.currentTarget.value })}
                            // Ad alanında Enter silmeyi tetiklemez.
                            onKeyDown={(ev) => ev.key === "Enter" && ev.preventDefault()}
                          />
                        </div>
                      )}
                    </Show>
                  </Show>
                </div>
              </Show>
            </Show>
          </div>
        )}
      </Show>
    </Dialog>
  );
}
