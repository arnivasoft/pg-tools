// Risk rozeti: SqlPreview, ApplyBar ve silme diyaloğu ortak kullanır. Ayrı modüldedir; silme diyaloğu SqlPreview'ı
// (CodeMirror) içe aktarmadan rozeti kullanabilsin.
import { tdes } from "../../app/strings-designer";
import type { Risk } from "../../ipc/bindings/Risk";

const riskClass: Record<Risk, string> = {
  safe: "border-border text-muted",
  destructive: "border-[color:var(--srv-amber)] text-warn-text",
  data_loss: "border-danger text-danger",
};

/** Risk rozeti (metin taşır; renk tek başına bilgi değildir). */
export function RiskBadge(props: { risk: Risk }) {
  return (
    <span class={`inline-flex h-[18px] shrink-0 items-center rounded-full border px-1.5 text-[11px] ${riskClass[props.risk]}`}>
      {tdes.risk[props.risk]}
    </span>
  );
}
