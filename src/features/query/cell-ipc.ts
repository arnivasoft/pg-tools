// Hücre görüntüleyici IPC sarıcıları. Yalnız tembel yüklenen CellViewers içe aktarır (ana pakete girmez).
import { call } from "../../ipc/commands";

export const cellIpc = {
  openPdf: (data: string) => call<null>("cell_open_pdf", { data }),
  saveBytes: (data: string, path: string) => call<null>("cell_save_bytes", { data, path }),
};

export type CellIpc = typeof cellIpc;
