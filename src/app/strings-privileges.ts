// Yetki formlarının metinleri (Plan 22). Yalnızca tembel yetki formları (PrivilegesForm / DefaultPrivilegesForm) içe
// aktarır; ana pakette durmaz. Menü metinleri ana `t.menu`'dedir. Bileşen etiket tipleri yalnız tip olarak alınır.
import type { AclGridLabels } from "../features/designer/AclGrid";
import type { AclReadOnlyLabels } from "../features/designer/AclReadOnlyLists";
import type { Privilege } from "../ipc/bindings/Privilege";

/** Yetki adı: SQL anahtar sözcüğü (SELECT, TEMPORARY…). */
const privilegeName = (p: Privilege) => p.toUpperCase();
const publicName = "PUBLIC";

const grid: AclGridLabels = {
  grid: "Yetki ızgarası",
  privilege: privilegeName,
  state: { none: "yok", granted: "var", grantable: "devredilebilir" },
  publicName,
  owner: "sahip",
  grantee: "Alıcı",
  remove: (g) => `${g} alıcısını kaldır`,
  unknown: (n) => `Bilinmeyen yetki: ${n}`,
  actions: "Satır işlemleri",
  empty: "Bu nesnede hiçbir alıcıya yetki verilmemiş.",
};

const readOnly: AclReadOnlyLabels = {
  foreignTitle: "Başka verenlerin yetkileri",
  columnsTitle: "Kolon yetkileri",
  grantor: "Veren",
  grantee: "Alıcı",
  privilege: "Yetki",
  grantOption: "Devredilebilir",
  column: "Kolon",
  yes: "evet",
  publicName,
  privilegeName,
};

export const tpriv = {
  grid,
  readOnly,
  publicName,
  owner: "Sahip",
  viewer: "Bağlı rol",
  implicit: "Varsayılan (açık ACL yok) yetkiler gösteriliyor; ilk değişiklik ACL'yi açık hâle getirir.",
  legend: "Hücreye tıklayın ya da Space/Enter: yok → var → devredilebilir (GRANT OPTION). PUBLIC ve sahipte GRANT OPTION yoktur.",
  add: {
    label: "Alıcı ekle",
    placeholder: "Rol adı ya da PUBLIC",
    button: "Ekle",
    empty: "Alıcı adı girin.",
    duplicate: "Bu alıcı zaten listede.",
  },
  cascade: "Bağımlı yetkilerle birlikte geri al (CASCADE)",
  cascadeHint: "Alıcının bu yetkiyle başkalarına verdiği yetkiler de geri alınır.",
  cascadeIdle: "Yalnız planda REVOKE varken etkilidir.",
} as const;
