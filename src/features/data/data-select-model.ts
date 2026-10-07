// Sıralama/filtre durumu (saf). İstemci SQL üretmez: istek `data_select`'e gider, filtre metni olduğu gibi gönderilir.
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { OrderBy } from "../../ipc/bindings/OrderBy";
import type { SelectRequest } from "../../ipc/bindings/SelectRequest";

/** Arka ucun kabul ettiği en çok sıralama anahtarı. */
export const MAX_ORDER = 8;

export interface SelectState {
  order: OrderBy[];
  filter: string;
}

export const EMPTY_SELECT: SelectState = { order: [], filter: "" };

/** Kolonun bir sonraki hâli: yok → artan → azalan → yok. */
function next(cur: OrderBy | undefined, column: string): OrderBy | null {
  if (!cur) return { column, desc: false };
  return cur.desc ? null : { column, desc: true };
}

/** Başlık tıklaması: `additive` (Shift) mevcut sıralamada döndürür/ekler; değilse yalnızca bu kolon kalır. */
export function cycleOrder(order: readonly OrderBy[], column: string, additive: boolean): OrderBy[] {
  const cur = order.find((o) => o.column === column);
  const n = next(cur, column);
  if (!additive) return n ? [n] : [];
  if (!cur) return n && order.length < MAX_ORDER ? [...order, n] : [...order];
  return order.flatMap((o) => (o.column !== column ? [o] : n ? [n] : []));
}

export function toRequest(s: SelectState): SelectRequest {
  return { order: [...s.order], filter: s.filter.trim() === "" ? null : s.filter };
}

const ORDER_PATH = /^order(\[\d+\](\.column)?)?$/;

/** `data_select` alan hataları: `filter` | `order`, `order[i]`, `order[i].column` | gerisi genel. */
export function selectErrors(errors: readonly FieldError[]): { filter: string[]; order: string[]; general: string[] } {
  const r = { filter: [] as string[], order: [] as string[], general: [] as string[] };
  for (const e of errors) {
    if (e.path === "filter") r.filter.push(e.message);
    else if (ORDER_PATH.test(e.path)) r.order.push(e.message);
    else r.general.push(e.message);
  }
  return r;
}
