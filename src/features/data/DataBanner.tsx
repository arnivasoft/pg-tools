// Veri sekmesi şeritleri (tembel DataTabs parçası): salt okunur gerekçesi (`role="status"`), yükleme hatası, bağlantı
// kesildi (yeniden bağlan), nesne/sunucu yok ve bayat kaynak (yeniden yükle) (`role="alert"`). PG iletisi yalnızca
// burada görünür; günlüğe/bildirime gitmez.
import { Show } from "solid-js";
import { tdata } from "../../app/strings-data";
import type { DataModel } from "./data-controller";
import { readOnlyText } from "./data-grid-model";
import { BTN } from "./DataToolbar";

const ALERT = "flex shrink-0 flex-wrap items-center gap-2 border-b border-danger px-3 py-1.5 text-[13px] text-danger";

export function DataBanner(props: { m: DataModel; onReload: () => void; onReconnect: () => void }) {
  const ro = () => {
    const l = props.m.loaded;
    const s = l?.source ?? props.m.source;
    return s ? readOnlyText(s, l?.hasVersion ?? true, props.m.locked) : null;
  };
  return (
    <>
      <Show when={ro()}>
        {(text) => (
          <p role="status" class="shrink-0 border-b border-border bg-panel px-3 py-1.5 text-[13px] text-warn-text">
            {text()}
          </p>
        )}
      </Show>
      <div role="alert" class="contents">
        <Show when={props.m.status === "disconnected"}>
          <div class={ALERT}>
            <span class="min-w-0 flex-1">{tdata.tab.disconnected}</span>
            <button type="button" class={BTN} onClick={() => props.onReconnect()}>
              {tdata.tab.reconnect}
            </button>
          </div>
        </Show>
        <Show when={props.m.status === "gone"}>
          <div class={ALERT}>{tdata.tab.gone}</div>
        </Show>
        <Show when={props.m.needsReload && props.m.status !== "gone"}>
          <div class={ALERT}>
            <span class="min-w-0 flex-1">{tdata.banner.stale}</span>
            <button type="button" class={BTN} disabled={props.m.applying} onClick={() => props.onReload()}>
              {tdata.toolbar.reload}
            </button>
          </div>
        </Show>
        <Show when={props.m.loadError}>
          {(err) => (
            <div class={ALERT}>
              <span class="min-w-0 flex-1">{`${tdata.banner.loadError} ${err()}`}</span>
              <Show when={props.m.status !== "gone" && props.m.status !== "disconnected"}>
                <button type="button" class={BTN} disabled={props.m.applying || props.m.status === "loading"} onClick={() => props.onReload()}>
                  {tdata.toolbar.reload}
                </button>
              </Show>
            </div>
          )}
        </Show>
      </div>
      <Show when={props.m.status === "loading"}>
        <p role="status" class="shrink-0 px-3 py-1 text-[12px] text-muted">
          {tdata.status.loading}
        </p>
      </Show>
    </>
  );
}
