export type ToastKind = "info" | "success" | "error";
export interface Toast { id: number; kind: ToastKind; text: string }
export const MAX_TOASTS = 4;

export function pushToast(list: Toast[], toast: Toast): Toast[] {
  const next = [...list, toast];
  return next.length > MAX_TOASTS ? next.slice(next.length - MAX_TOASTS) : next;
}
export function dismissToast(list: Toast[], id: number): Toast[] {
  return list.some((t) => t.id === id) ? list.filter((t) => t.id !== id) : list;
}
export function durationFor(kind: ToastKind): number {
  return kind === "error" ? 8000 : 3500;
}
