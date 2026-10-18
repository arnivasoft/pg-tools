// Gövde düzenleyicisinde dış değer eşitlemesi (Plan 23): DOM'suz, yalnız `@codemirror/state` (node testlerinde çalışır).
import { ChangeSet, EditorState, type Extension } from "@codemirror/state";
import { displayDiff } from "./sql-body-model";

/**
 * Dışarıdan gelen görüntü metni (`nextDisplay`; yeniden yükleme, form düzeyi geri al) için yeni durum: geri alma geçmişi
 * sıfırlanır (Ctrl+Z yeniden yükleme öncesi düzenlemeleri yeni metne uygulayamaz), seçim en küçük değişiklikle eşlenir.
 * `extensions` çağıranın güncel yapılandırmasıdır (compartment'lar şimdiki değerleriyle). Metin aynıysa `null`.
 */
export function resyncState(state: EditorState, nextDisplay: string, extensions: Extension): EditorState | null {
  const d = displayDiff(state.doc.toString(), nextDisplay);
  if (!d) return null;
  const changes = ChangeSet.of(d, state.doc.length);
  return EditorState.create({ doc: nextDisplay, selection: state.selection.map(changes), extensions });
}
