// Yerel dosya/klasör seçici sarmalayıcısı (Plan 27). Eklentiyi içe aktarmaz: `BackupDialogs.tsx` `@tauri-apps/plugin-dialog`'un
// `save`/`open` işlevlerini verir; böylece mantık node ortamında test edilir. Eklenti hatası olduğu gibi fırlar (denetleyici iletir).
import type { Picker } from "./backup-controller";

type OpenResult = string | string[] | null;
export interface DialogLike {
  save(o: unknown): Promise<string | null>;
  open(o: unknown): Promise<OpenResult>;
}

const first = (r: OpenResult): string | null => (Array.isArray(r) ? (r[0] ?? null) : r);

export function makePicker(dialog: DialogLike): Picker {
  return {
    save: (o) => dialog.save(o),
    openFile: async () =>
      first(
        await dialog.open({
          multiple: false,
          directory: false,
          filters: [
            { name: "PostgreSQL yedeği", extensions: ["backup", "dump", "sql", "tar", "gz"] },
            { name: "Tüm dosyalar", extensions: ["*"] },
          ],
        }),
      ),
    openDir: async () => first(await dialog.open({ multiple: false, directory: true })),
  };
}
