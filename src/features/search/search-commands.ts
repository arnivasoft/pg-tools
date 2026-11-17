// Nesne arama komut sarmalayıcısı (tembel parça; Plan 32). commands.ts'e eklenmez: ana paket büyümez.
// Tauri parametre adları src-tauri tarafıyla birebir (camelCase).
import { invoke } from "@tauri-apps/api/core";
import type { SearchKind } from "../../ipc/bindings/SearchKind";
import type { SearchResult } from "../../ipc/bindings/SearchResult";
import { toPayload } from "../../ipc/errors";

export async function searchObjects(serverId: string, databaseOid: number, query: string, kinds: SearchKind[]): Promise<SearchResult> {
  try {
    return await invoke<SearchResult>("search_objects", { serverId, databaseOid, query, kinds });
  } catch (e) {
    const p = toPayload(e);
    throw Object.assign(new Error(p.message), p);
  }
}
