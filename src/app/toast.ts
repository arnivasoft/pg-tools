import { createRoot, createSignal } from "solid-js";
import { dismissToast, durationFor, pushToast, type Toast, type ToastKind } from "./toast-model";

export const toast = createRoot(() => {
  const [list, setList] = createSignal<Toast[]>([]);
  const timers = new Map<number, ReturnType<typeof setTimeout>>();
  let nextId = 1;
  const clearTimer = (id: number) => {
    const handle = timers.get(id);
    if (handle !== undefined) {
      clearTimeout(handle);
      timers.delete(id);
    }
  };
  const dismiss = (id: number) => {
    clearTimer(id);
    setList((l) => dismissToast(l, id));
  };
  const schedule = (id: number, kind: ToastKind) => {
    clearTimer(id);
    timers.set(id, setTimeout(() => dismiss(id), durationFor(kind)));
  };
  const show = (text: string, kind: ToastKind = "info") => {
    const result = pushToast(list(), { id: nextId, kind, text });
    const id = result.mergedInto ?? nextId++;
    for (const evicted of result.evicted) clearTimer(evicted);
    setList(result.list);
    schedule(id, kind);
  };
  return { list, show, dismiss };
});
