// Designer uygulama çubuğu (tembel parça): risk özeti, lock_timeout, kapıya göre onay denetimleri ve düğmeler.
// Kapı (`peekApplyGate`) yalnızca arayüz içindir; denetleyici `apply` anında kapıyı yeniden hesaplar (arayüze güvenmez).
// Enter yıkıcı eylemi onaylamaz: onay kutusu ve Uygula ayrı denetimlerdir, ad alanında Enter gönderim yapmaz.
// Gizlenmiş (parola) ifadeli planda "Query Tool'da aç" kapalıdır (`scriptActionsEnabled`; işleyici de erken döner).
import { Play, SquareTerminal, X } from "lucide-solid";
import { For, Show, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import type { DesignerController, DesignerModel } from "./designer-controller";
import { canSubmit, scriptActionsEnabled, scriptWithRanges, type ApplyGate } from "./designer-model";
import { RiskBadge } from "./RiskBadge";

/** lock_timeout seçenekleri (ms; 0 = sınırsız). */
const LOCK_TIMEOUTS = [1000, 5000, 30000, 0];

const selectClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:opacity-50";

export default function ApplyBar(props: {
  designer: DesignerController;
  model: DesignerModel;
  onOpenScript: (text: string) => void;
  onClose: () => void;
}) {
  const b = tdes.bar;
  const nameId = createUniqueId();
  const id = () => props.model.id;
  const gate = (): ApplyGate => props.designer.peekApplyGate(id());
  const locked = () => props.model.locked;
  const applying = () => props.model.applying;
  const done = () => props.model.mode === "create" && props.model.outcome?.outcome === "applied";
  const view = () => (props.model.view && props.model.viewFor === props.model.version ? props.model.view : null);
  const statements = () => view()?.statements ?? [];
  const needsConfirm = () => gate().kind === "needsConfirm" || gate().kind === "needsProductionName";
  const expectedName = () => {
    const g = gate();
    return g.kind === "needsProductionName" ? g.expected : null;
  };
  const submittable = () => !applying() && !done() && canSubmit(gate(), props.model.confirm);
  const dangerous = () => {
    const r = view()?.risk;
    return r === "destructive" || r === "data_loss";
  };
  /** Uygula kapalıyken gerekçe. */
  const applyTitle = () => {
    const g = gate();
    if (g.kind === "blocked") return g.reason === "password" ? tdes.gate.password : b.blocked[g.reason];
    if (submittable()) return undefined;
    return g.kind === "needsProductionName" ? b.needsName : g.kind === "needsConfirm" ? b.needsConfirm : undefined;
  };
  /** Query Tool'da aç: güncel görünüm var ve gizlenmiş ifade yok. */
  const canOpenScript = () => scriptActionsEnabled(view()).open;
  const failed = () => (props.model.outcome?.outcome === "failed" ? props.model.outcome : null);
  // Kısmen kalıcı hata/plan değişimi yeniden yükleme şeridinde (DesignerView) gösterilir; burada tekrarlanmaz.
  const gateError = () => (props.model.needsReload ? null : props.model.gateError);

  return (
    <div class="shrink-0 border-t border-border bg-panel">
      <Show when={gateError() || props.model.applyError}>
        <div role="alert" class="border-b border-border px-3 py-1.5 text-[12px] text-danger">
          <Show when={gateError()}>{(e) => <p>{e()}</p>}</Show>
          <Show when={props.model.applyError}>
            {(e) => (
              <p>
                {e()}
                <Show when={failed()?.position}>{(pos) => <span class="text-muted"> ({b.position(pos())})</span>}</Show>
              </p>
            )}
          </Show>
          <Show when={(failed()?.committed ?? 0) > 0}>
            <p class="font-medium">{tdes.apply.committed(failed()?.committed ?? 0)}</p>
          </Show>
        </div>
      </Show>
      <Show when={needsConfirm() && !locked()}>
        <div class="flex flex-col gap-1.5 border-b border-border px-3 py-2 text-[12px]">
          <label class="flex items-center gap-2">
            <input
              type="checkbox"
              checked={props.model.confirm.checked}
              disabled={applying()}
              onChange={(ev) => props.designer.setConfirm(id(), { checked: ev.currentTarget.checked })}
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
                    aria-label={b.typeNameLabel}
                    disabled={applying()}
                    value={props.model.confirm.typedName}
                    onInput={(ev) => props.designer.setConfirm(id(), { typedName: ev.currentTarget.value })}
                    // Ad alanında Enter formu/uygulamayı tetiklemez.
                    onKeyDown={(ev) => ev.key === "Enter" && ev.preventDefault()}
                  />
                </div>
              )}
            </Show>
          </Show>
        </div>
      </Show>
      <div class="flex flex-wrap items-center gap-2 px-2 py-1.5">
        <span class="inline-flex items-center gap-1 text-[12px] text-muted">
          {b.risk}
          <Show when={view()} fallback={<span>—</span>}>
            {(v) => <RiskBadge risk={v().risk} />}
          </Show>
        </span>
        <select
          class={selectClass}
          aria-label={b.lockTimeout}
          title={b.lockTimeoutTitle}
          disabled={applying() || locked()}
          value={String(props.model.lockTimeoutMs)}
          onChange={(ev) => props.designer.setLockTimeout(id(), Number(ev.currentTarget.value))}
        >
          <For each={LOCK_TIMEOUTS}>{(ms) => <option value={String(ms)}>{`${b.lockTimeout}: ${b.lockChoice(ms)}`}</option>}</For>
        </select>
        <div class="ml-auto flex items-center gap-1.5">
          <Button
            variant="ghost"
            disabled={statements().length === 0 || applying() || !canOpenScript()}
            title={view() && !canOpenScript() ? b.openScriptRedacted : b.openScriptTitle}
            onClick={() => {
              // Savunma: gizlenmiş plan düğme kapalı olsa da aktarılmaz.
              if (!canOpenScript()) return;
              props.onOpenScript(scriptWithRanges(statements()).text);
            }}
          >
            <SquareTerminal size={14} /> {b.openScript}
          </Button>
          <Show
            when={applying()}
            fallback={
              <Button onClick={() => props.onClose()}>
                <X size={14} /> {b.close}
              </Button>
            }
          >
            <Button title={b.cancelTitle} onClick={() => void props.designer.cancelApply(id())}>
              {b.cancel}
            </Button>
          </Show>
          <Button
            variant="primary"
            class={dangerous() ? "!bg-danger !text-danger-fg" : ""}
            disabled={!submittable() || locked()}
            title={locked() ? b.lockedTitle : applyTitle()}
            onClick={() => {
              // Çift tık/yeniden giriş: denetleyici uçuşta ikinci isteği zaten reddeder.
              if (submittable()) void props.designer.apply(id());
            }}
          >
            <Play size={14} /> {applying() ? b.applying : b.apply}
          </Button>
        </div>
      </div>
    </div>
  );
}
