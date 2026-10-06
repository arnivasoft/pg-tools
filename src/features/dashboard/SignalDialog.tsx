// İptal/sonlandırma onay diyaloğu: `dash.signalGate.request()`'i çizer (QueryTabs'taki `CloseTxDialog` kalıbı). Odak
// "Vazgeç"te başlar (Enter yıkıcı eylemi onaylamaz); onayda gerçek iş denetleyicidedir (`confirmSignal` → canlı doğrulama →
// `dash_signal`). Onaydan sonra kapı kapanır; uçuştaki istek `model.signaling` ile düğmeleri kapalı tutar.
import { For, Show, createEffect, on } from "solid-js";
import { t } from "../../app/strings";
import { td } from "../../app/strings-dashboard";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import type { DashController } from "./dash-controller";
import { firstLines } from "./table-model";

/** Gövdede gösterilen sorgu satırı sayısı. */
const QUERY_LINES = 3;

export default function SignalDialog(props: { dash: DashController; tabId: string; restoreFocus?: () => void }) {
  const g = props.dash.signalGate;
  /** Yalnızca bu sekmenin isteği (her dashboard sekmesi kendi görünümünde bir diyalog taşır). */
  const req = () => {
    const r = g.request();
    return r && r.tabId === props.tabId ? r : null;
  };
  const busy = () => props.dash.model(props.tabId)?.signaling ?? false;
  // Diyalog kapanınca odak gövdeye düştüyse (tetikleyen düğme yenilemede yeniden oluşmuş olabilir) ızgaraya dön.
  createEffect(
    on(
      () => req() !== null,
      (open, was) => {
        if (was && !open) setTimeout(() => document.activeElement === document.body && props.restoreFocus?.(), 0);
      },
    ),
  );
  let cancelEl: HTMLButtonElement | undefined;
  const s = td.signal;
  const lines = () => firstLines(req()?.row.query ?? null, QUERY_LINES);
  const terminate = () => req()?.kind === "terminate";
  return (
    <Dialog
      open={req() !== null}
      onOpenChange={(open) => !open && g.answer(false)}
      title={terminate() ? s.terminateTitle : s.cancelTitle}
      width="460px"
      // Yıkıcı eylem yanlışlıkla Enter ile onaylanmasın: odak "Vazgeç"te başlar.
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => g.answer(false)}>
            {t.server.cancel}
          </Button>
          <Button
            variant="primary"
            class={terminate() ? "!bg-danger !text-white" : ""}
            disabled={busy()}
            onClick={() => g.answer(true)}
          >
            {terminate() ? s.terminate : s.confirmCancel}
          </Button>
        </>
      }
    >
      <Show when={req()}>
        {(r) => (
          <div class="flex flex-col gap-2 text-[13px]">
            <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
              <dt class="text-muted">{s.pid}</dt>
              <dd class="font-mono">{r().row.pid}</dd>
              <dt class="text-muted">{s.user}</dt>
              <dd class="truncate">{r().row.user ?? td.sessions.unknown}</dd>
              <dt class="text-muted">{s.app}</dt>
              <dd class="truncate">{r().row.application || td.sessions.unknown}</dd>
              <dt class="text-muted">{s.state}</dt>
              <dd>{r().row.state ?? td.sessions.unknown}</dd>
            </dl>
            <div>
              <div class="text-muted">{s.query}</div>
              <Show
                when={r().row.query != null}
                fallback={<p class="italic text-muted">{td.sessions.noQuery}</p>}
              >
                <pre class="max-h-24 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-panel px-2 py-1 font-mono text-[12px]">
                  {lines().text || td.sessions.emptyQuery}
                  <Show when={lines().truncated}>
                    {"\n"}
                    {s.queryTruncated}
                  </Show>
                </pre>
              </Show>
            </div>
            <Show when={r().warnings.length > 0}>
              <ul class="flex flex-col gap-1">
                <For each={r().warnings}>
                  {(w) => (
                    <li class="rounded border border-danger px-2 py-1 text-danger">{s.warn[w.kind]}</li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        )}
      </Show>
    </Dialog>
  );
}
