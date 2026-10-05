import { createRoot, createSignal } from "solid-js";
import { dismissToast, durationFor, pushToast, type Toast, type ToastKind } from "./toast-model";

export const toast = createRoot(() => {
  const [list, setList] = createSignal<Toast[]>([]);
  let nextId = 1;
  const dismiss = (id: number) => setList((l) => dismissToast(l, id));
  const show = (text: string, kind: ToastKind = "info") => {
    const id = nextId++;
    setList((l) => pushToast(l, { id, kind, text }));
    setTimeout(() => dismiss(id), durationFor(kind));
  };
  return { list, show, dismiss };
});
