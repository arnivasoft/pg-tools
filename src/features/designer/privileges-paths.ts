// Yetki formlarının hata yolu eşlemesi (Plan 22, spec §8; tembel parça). `groupIssues` kalıbıyla çalışır: ızgaranın
// sahiplendiği yollar (`<taban>[i].grantee`, kanonik yetki hücresi `<taban>[i].privileges[j]` ve alt yolları) satır/hücreye
// düşer; geri kalan her şey (catch-all) üst hata şeridine gider: `object`, `schema`, `name`, `signature`, `foreign`,
// `columns`, `sections`, `sections[i].baseline`, `old.*`, çıplak `grants` / `grants[i]` / `grants[i].privileges`,
// `available` dışı (bilinmeyen) yetkinin hücresi ve taslakta olmayan satır. Yollar güncel taslaktan hesaplanır; satır
// ekleme/silme sonrası eski görünümün hataları önizleme yenilenene kadar bayat kalabilir (bilinen pürüz) ama ızgara
// var olmayan hücreye hata göstermez.
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { Privilege } from "../../ipc/bindings/Privilege";

/** Her zaman üst şeride düşen kök yollar (belgeleme/test; catch-all zaten kapsar). */
export const ACL_GENERAL_PATHS = ["object", "schema", "name", "signature", "foreign", "columns", "sections", "old"] as const;

/** Izgaranın sahiplendiği yollar (`groupIssues` `knownPaths`'i; alt yollar `under` ile kapsanır). */
export function aclGridPaths(base: string, grants: readonly AclGrant[], available: readonly Privilege[]): Set<string> {
  const out = new Set<string>();
  grants.forEach((g, i) => {
    out.add(`${base}[${i}].grantee`);
    g.privileges.forEach((p, j) => {
      if (available.includes(p.privilege)) out.add(`${base}[${i}].privileges[${j}]`);
    });
  });
  return out;
}
