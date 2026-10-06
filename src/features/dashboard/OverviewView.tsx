// Genel (Overview) bölümü. Özet kartları ve grafikler Task 4'te gelir; şimdilik yükleme durumu gösterilir.
import { td } from "../../app/strings-dashboard";
import type { SectionProps } from "./DashboardView";

export default function OverviewView(_props: SectionProps) {
  return <p class="p-3 text-muted">{td.loading}</p>;
}
