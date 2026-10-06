// Oturumlar ve Kilitler görünümlerinin ortak küçük parçaları: durum rozeti, süre metni, İptal/Sonlandır düğmeleri, bant.
import type { JSX } from "solid-js";
import { td } from "../../app/strings-dashboard";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { SignalKind } from "../../ipc/bindings/SignalKind";
import { Button } from "../../ui/Button";
import type { DashController } from "./dash-controller";
import { formatDuration } from "./dash-model";
import { durationTone, stateTone, type DurationTone, type StateTone } from "./table-model";

const toneClass: Record<StateTone, string> = {
  active: "border-accent text-accent",
  idle: "border-border text-muted",
  warn: "border-[color:var(--srv-amber)] text-warn-text",
  danger: "border-danger text-danger",
  muted: "border-border text-muted",
};

/** Durum rozeti: ham PostgreSQL durum metni (null → "—"), ton renkle; renk tek başına bilgi taşımaz (metin var). */
export function StateBadge(props: { state: string | null }) {
  return (
    <span class={`inline-flex h-[18px] items-center rounded-full border px-1.5 text-[11px] ${toneClass[stateTone(props.state)]}`}>
      {props.state ?? td.sessions.unknown}
    </span>
  );
}

const durClass: Record<DurationTone, string> = {
  none: "text-muted",
  normal: "",
  warn: "text-warn-text",
  danger: "text-danger font-medium",
};

export function Duration(props: { secs: number | null }) {
  return <span class={durClass[durationTone(props.secs)]}>{formatDuration(props.secs)}</span>;
}

/** İptal + Sonlandır: kendi satırında ve istek uçuşta iken kapalı (`title` nedenini söyler). Diyalog açar, doğrudan göndermez. */
export function SignalButtons(props: { dash: DashController; tabId: string; row: ActivityRow; signaling: boolean }) {
  const s = td.sessions;
  const disabled = () => props.row.isSelf || props.signaling;
  const why = (normal: string) => (props.row.isSelf ? s.selfTitle : props.signaling ? s.busyTitle : normal);
  const go = (kind: SignalKind) => void props.dash.confirmSignal(props.tabId, props.row, kind);
  return (
    <span class="inline-flex gap-1">
      <Button class="!h-5 !px-1.5 text-[11px]" disabled={disabled()} title={why(s.cancelTitle)} onClick={() => go("cancel")}>
        {td.signal.cancel}
      </Button>
      <Button class="!h-5 !px-1.5 text-[11px]" disabled={disabled()} title={why(s.terminateTitle)} onClick={() => go("terminate")}>
        {td.signal.terminate}
      </Button>
    </span>
  );
}

/** Bölüm hatası/kesilme bandı: eski veri altında görünür kalır. */
export function Banner(props: { kind: "error" | "info"; children: JSX.Element }) {
  return (
    <div
      role={props.kind === "error" ? "alert" : "status"}
      class="shrink-0 border-b px-3 py-1.5 text-[12px]"
      classList={{ "border-danger text-danger": props.kind === "error", "border-border bg-panel text-muted": props.kind === "info" }}
    >
      {props.children}
    </div>
  );
}

