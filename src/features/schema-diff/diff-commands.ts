// Şema karşılaştırma komut sarmalayıcıları (tembel parça). commands.ts'e eklenmez: ana paket büyümez (Plan 30).
// Tauri parametre adları src-tauri tarafıyla birebir (camelCase).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { DiffDatabase } from "../../ipc/bindings/DiffDatabase";
import type { DiffDetail } from "../../ipc/bindings/DiffDetail";
import type { DiffEvent } from "../../ipc/bindings/DiffEvent";
import type { DiffOptions } from "../../ipc/bindings/DiffOptions";
import type { DiffResult } from "../../ipc/bindings/DiffResult";
import type { DiffScript } from "../../ipc/bindings/DiffScript";
import type { DiffSide } from "../../ipc/bindings/DiffSide";
import { toPayload } from "../../ipc/errors";

/** data-commands.ts'teki yerel `call` ile aynı gerekçe: `IpcError` içe aktarılmaz; hata yükün alanlarını taşır. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const p = toPayload(e);
    throw Object.assign(new Error(p.message), p);
  }
}

function channel<T>(onMessage: (m: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = onMessage;
  return ch;
}

export const schemaDiffDatabases = (serverId: string) => call<DiffDatabase[]>("schema_diff_databases", { serverId });
export const schemaDiffStart = (
  diffId: string,
  source: DiffSide,
  target: DiffSide,
  options: DiffOptions,
  onEvent: (e: DiffEvent) => void,
) => call<DiffResult>("schema_diff_start", { diffId, source, target, options, onEvent: channel(onEvent) });
export const schemaDiffCancel = (diffId: string) => call<null>("schema_diff_cancel", { diffId });
export const schemaDiffDetail = (diffId: string, itemKey: string) => call<DiffDetail>("schema_diff_detail", { diffId, itemKey });
export const schemaDiffScript = (diffId: string, itemKeys: string[]) => call<DiffScript>("schema_diff_script", { diffId, itemKeys });
export const schemaDiffSave = (diffId: string, path: string) => call<null>("schema_diff_save", { diffId, path });
export const schemaDiffClose = (diffId: string) => call<null>("schema_diff_close", { diffId });

export type DiffApi = {
  schemaDiffDatabases: typeof schemaDiffDatabases;
  schemaDiffStart: typeof schemaDiffStart;
  schemaDiffCancel: typeof schemaDiffCancel;
  schemaDiffDetail: typeof schemaDiffDetail;
  schemaDiffScript: typeof schemaDiffScript;
  schemaDiffSave: typeof schemaDiffSave;
  schemaDiffClose: typeof schemaDiffClose;
};

export const diffApi: DiffApi = {
  schemaDiffDatabases,
  schemaDiffStart,
  schemaDiffCancel,
  schemaDiffDetail,
  schemaDiffScript,
  schemaDiffSave,
  schemaDiffClose,
};
