export type ToastKind = "info" | "success" | "error";
export interface Toast { id: number; kind: ToastKind; text: string }
export const MAX_TOASTS = 4;

/** Result of pushing a toast: the new list, ids evicted by the MAX_TOASTS cap
 *  (their timers must be cleared) and, when the toast duplicated the newest one,
 *  the id of the existing toast that absorbed it (its timer must be restarted). */
export interface PushResult { list: Toast[]; evicted: number[]; mergedInto?: number }

export function pushToast(list: Toast[], toast: Toast): PushResult {
  const last = list[list.length - 1];
  if (last && last.kind === toast.kind && last.text === toast.text) {
    return { list, evicted: [], mergedInto: last.id };
  }
  const next = [...list, toast];
  if (next.length <= MAX_TOASTS) return { list: next, evicted: [] };
  const cut = next.length - MAX_TOASTS;
  return { list: next.slice(cut), evicted: next.slice(0, cut).map((t) => t.id) };
}
export function dismissToast(list: Toast[], id: number): Toast[] {
  return list.some((t) => t.id === id) ? list.filter((t) => t.id !== id) : list;
}
export function durationFor(kind: ToastKind): number {
  return kind === "error" ? 8000 : 3500;
}
