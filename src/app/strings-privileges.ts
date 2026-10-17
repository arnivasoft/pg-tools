// Yetki formlarının metinleri (Plan 22). Yalnızca tembel yetki formları (PrivilegesForm / DefaultPrivilegesForm) içe
// aktarır; ana pakette durmaz. Menü metinleri ana `t.menu`'dedir. Bileşen etiket tipleri yalnız tip olarak alınır.
import type { AclAddGranteeLabels } from "../features/designer/AclAddGrantee";
import type { AclGridLabels } from "../features/designer/AclGrid";
import type { AclReadOnlyLabels } from "../features/designer/AclReadOnlyLists";
import type { DefaultAclObjectType } from "../ipc/bindings/DefaultAclObjectType";
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
    // Sözde PUBLIC seçeneğinin değeri; "PUBLIC" adlı gerçek rolden ayırt edilir.
    publicOption: "PUBLIC (herkes)",
  } satisfies AclAddGranteeLabels,
  cascade: "Bağımlı yetkilerle birlikte geri al (CASCADE)",
  cascadeHint: "Alıcının bu yetkiyle başkalarına verdiği yetkiler de geri alınır.",
  cascadeIdle: "Yalnız planda REVOKE varken etkilidir.",
  /** "Varsayılan yetkiler…" formu (Task 5). */
  defaults: {
    scope: "Kapsam",
    role: "Rol",
    database: "Veritabanı geneli",
    schema: (name: string) => `Şema: ${name}`,
    note: "Yalnızca bundan sonra oluşturulan nesneleri etkiler; mevcut nesnelerin yetkileri değişmez.",
    info: "Bilgi",
    loadError: "Seçilen rolün varsayılan yetkileri yüklenemedi; önceki rol gösteriliyor.",
    loading: "Yükleniyor…",
    baseline: "Genel varsayılan (salt okunur)",
    baselineEmpty: "Genel kapsamda bu tür için yetki yok.",
    empty: "Bu türde varsayılan yetki yok.",
    section: { tables: "Tablolar", sequences: "Diziler", functions: "Fonksiyonlar", types: "Tipler", schemas: "Şemalar" } satisfies Record<DefaultAclObjectType, string>,
    discardMessage: "Rol değişince tanım yeniden yüklenir ve formdaki kaydedilmemiş değişiklikler kaybolur. Kaydedilmemiş değişiklikler atılsın mı?",
    discardConfirm: "Değişiklikleri at ve rolü değiştir",
  },
} as const;
