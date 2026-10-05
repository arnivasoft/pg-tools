export interface ForId<T> {
  id: string;
  value: T;
}

/** Bir değeri yalnızca onu üreten düğüm hâlâ seçiliyse geçerli sayar. */
export function forNode<T>(entry: ForId<T> | undefined, id: string | undefined): T | undefined {
  return entry !== undefined && id !== undefined && entry.id === id ? entry.value : undefined;
}
