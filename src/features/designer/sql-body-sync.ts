// Gövde düzenleyicisinde dış değer eşitlemesi (Plan 23): DOM'suz, yalnız `@codemirror/state` (node testlerinde çalışır).
import { ChangeSet, EditorState, type Extension } from "@codemirror/state";
import { displayDiff } from "./sql-body-model";

/** Belge metni (CodeMirror `Text` ya da dize): uzunluk ön denetimi `toString`'i çoğu durumda gereksiz kılar. */
interface DocLike {
  readonly length: number;
  toString(): string;
}

/**
 * Dış değer geldiğinde ne yapılacağı. Önce karşılaştırma: belge değerin görüntüsüyle aynıysa `noop` (editörün kendi
 * düzenlemesinin yankısı — bileşim sürse de; aksi halde IME ile yazılan metin ertelenip kaybolabilirdi). Gerçek dış
 * değişiklikte IME bileşimi sürüyorsa `defer` (bileşim bitince yeniden karar verilir), değilse `apply`.
 */
export function syncDecision(doc: DocLike, valueDisplay: string, composing: boolean): "noop" | "defer" | "apply" {
  if (doc.length === valueDisplay.length && doc.toString() === valueDisplay) return "noop";
  return composing ? "defer" : "apply";
}

/**
 * Dışarıdan gelen görüntü metni (`nextDisplay`; yeniden yükleme, form düzeyi geri al) için yeni durum: geri alma geçmişi
 * sıfırlanır (Ctrl+Z yeniden yükleme öncesi düzenlemeleri yeni metne uygulayamaz), seçim en küçük değişiklikle eşlenir.
 * `changes` çağıranın eski konumları (ör. kaydırma anlık görüntüsü) eşlemesi içindir. `extensions` çağıranın güncel
 * yapılandırmasıdır (compartment'lar şimdiki değerleriyle). Metin aynıysa `null` (geçmiş korunur).
 */
export function resyncState(
  state: EditorState,
  nextDisplay: string,
  extensions: Extension,
): { state: EditorState; changes: ChangeSet } | null {
  const d = displayDiff(state.doc.toString(), nextDisplay);
  if (!d) return null;
  const changes = ChangeSet.of(d, state.doc.length);
  return {
    state: EditorState.create({ doc: nextDisplay, selection: state.selection.map(changes), extensions }),
    changes,
  };
}
