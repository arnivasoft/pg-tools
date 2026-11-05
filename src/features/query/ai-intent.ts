// Asistana bekleyen istek: başka bir ekran (hata, plan, şema farkı, dashboard) bir sorgu sekmesinin Asistan panelini
// açtırır ve isteği bırakır; panel (tembel AiPane) isteği alıp gönderir. İstek sekme modelinde (`TabModel.aiIntent`)
// durur: ana pakete çalışma zamanı kodu eklemez, kalıcı değildir (taslak yalnız belirli alanları yazar).
//
// Bu modülü şema farkı ve dashboard yalnız tıklanınca dinamik yükler; QueryTab yalnız türünü kullanır. Statik ortak
// içe aktarma ortak parça yaratıp ana paketin ön yükleme listesini büyütürdü (bütçe).
import type { AiMode } from "../../ipc/bindings/AiMode";
import type { QueryController } from "./query-controller";
import { isQueryTabId } from "./tabs-store";

/**
 * Dışarıdan bırakılan istek. `sql` verilirse editör seçimi yerine o gönderilir; `extra`: plan ya da istatistik.
 * `label`: sohbette kullanıcı turunun başlığı (ör. "Planı açıkla").
 */
export interface AiIntent {
  mode: AiMode;
  label: string;
  prompt?: string;
  sql?: string;
  extra?: string;
}

export type OpenForAssistant = (
  input: { serverId: string; databaseOid: number | null; database?: string | null; text: string },
  intent: AiIntent,
) => Promise<void>;

/** Metni yeni sorgu sekmesinde açar (çalıştırmaz) ve asistana isteği bırakır; sekme açılmadıysa (vazgeçildi) bir şey yapmaz. */
export async function openForAssistant(
  query: Pick<QueryController, "openScript" | "tabs" | "patch">,
  input: Parameters<OpenForAssistant>[0],
  intent: AiIntent,
): Promise<void> {
  const before = query.tabs.state.activeId;
  await query.openScript(input);
  const id = query.tabs.state.activeId;
  if (id && id !== before && isQueryTabId(query.tabs.state, id)) query.patch(id, { aiIntent: intent });
}
