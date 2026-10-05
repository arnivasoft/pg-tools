import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";

export function createDebounced<T>(source: Accessor<T>, ms: number): Accessor<T> {
  const [value, setValue] = createSignal<T>(source());
  createEffect(
    on(
      source,
      (next) => {
        const handle = setTimeout(() => setValue(() => next), ms);
        onCleanup(() => clearTimeout(handle));
      },
      { defer: true },
    ),
  );
  return value;
}
