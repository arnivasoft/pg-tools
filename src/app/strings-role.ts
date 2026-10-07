// Role formu metinleri. Yalnızca RoleForm.tsx içe aktarır (tembel parça); strings-designer.ts'e eklenmez.
// PostgreSQL terimleri İngilizce kalır.
import type { PasswordChange } from "../ipc/bindings/PasswordChange";

type Flag = "login" | "superuser" | "createdb" | "createrole" | "inherit" | "replication" | "bypassrls";
type Lockout = "nologin" | "connectionLimitZero" | "validUntilPast" | "ownPassword";

const FLAG_NAMES: Record<Flag, string> = {
  login: "LOGIN", superuser: "SUPERUSER", createdb: "CREATEDB", createrole: "CREATEROLE", inherit: "INHERIT",
  replication: "REPLICATION", bypassrls: "BYPASSRLS",
};

export const trole = {
  sections: {
    general: "Genel",
    attributes: "Öznitelikler",
    connection: "Bağlantı",
    password: "Parola",
    memberships: "Üye olduğu roller",
    unsupported: "Bu formda düzenlenmeyenler",
  },
  general: {
    name: "Ad",
    rename: "Ad (değişirse yeniden adlandırılır)",
    namePlaceholder: "rol_adı",
    reservedHint: "public, none ve pg_ ile başlayan adlar ayrılmıştır.",
    reservedWarn: "Bu ad ayrılmış; sunucu reddeder.",
    md5Rename: "Yeniden adlandırma MD5 ile saklanan parolayı siler (SCRAM parolaları etkilenmez).",
  },
  flags: {
    label: (f: Flag) => FLAG_NAMES[f],
    hint: {
      login: "Giriş yapabilir",
      superuser: "Tüm denetimleri atlar",
      createdb: "Veritabanı oluşturabilir",
      createrole: "Rol oluşturabilir, değiştirebilir ve silebilir",
      inherit: "Üye olduğu rollerin yetkilerini devralır",
      replication: "Çoğaltma (replication) bağlantısı açabilir",
      bypassrls: "Satır düzeyi güvenlik (RLS) ilkelerini atlar",
    } satisfies Record<Flag, string>,
  },
  privileged: (list: readonly Flag[]) =>
    `Ayrıcalıklı öznitelik veriliyor: ${list.map((f) => FLAG_NAMES[f]).join(", ")}. Yalnızca gerçekten gerekiyorsa verin.`,
  connection: {
    limit: "Bağlantı sınırı (CONNECTION LIMIT)",
    unlimited: "Sınırsız",
    unlimitedHint: "Sınırsız için kutuyu işaretleyin (-1).",
  },
  validUntil: {
    legend: "Geçerlilik sonu (VALID UNTIL)",
    never: "Süresiz",
    date: "Tarih",
    time: "Saat",
    utc: "UTC olarak kaydedilir",
    tzHelp: "Yerel saatinizle girin; UTC olarak saklanır ve UTC gösterilir.",
    stored: (v: string) => `Kaydedilecek değer: ${v === "infinity" ? "infinity (süresiz)" : `${v} (UTC)`}`,
    noClear: "Yüklü süre sınırı kaldırılamaz; süresiz yapmak için Süresiz kutusunu işaretleyin (infinity).",
    invalid: "Geçerli bir tarih ve saat girin; geçerli değer girilene kadar önceki değer korunur.",
    unparsed: (v: string) => `Sunucudaki değer çözümlenemedi: ${v}`,
  },
  password: {
    legend: "Parola",
    choice: (c: PasswordChange, mode: "create" | "alter") =>
      c === "keep" ? (mode === "create" ? "Parolasız" : "Değiştirme") : c === "set" ? "Parola belirle" : "Parolayı kaldır",
    next: "Yeni parola",
    repeat: "Parola (tekrar)",
    empty: "Parola boş olamaz",
    mismatch: "Parolalar eşleşmiyor",
    privacy:
      "Parola bu bilgisayarda SCRAM-SHA-256 ile özetlenir; sunucuya ve sunucu günlüğüne düz metin gitmez (yalnızca özet). Parola hiçbir zaman yüklenmez ve gösterilmez.",
    clearNote: "Parola kaldırılırsa parola ile giriş yapılamaz.",
  },
  selfLockout: (k: Lockout) =>
    ({
      nologin: "Bağlı olduğunuz rolün girişi kapanıyor; bu bağlantı yeniden kurulamayabilir.",
      connectionLimitZero: "Bağlı olduğunuz rolün bağlantı sınırı 0 oluyor; yeni bağlantı açamazsınız.",
      validUntilPast: "Bağlı olduğunuz rolün süresi geçmiş olacak; yeniden giriş yapılamaz.",
      ownPassword: "Kendi parolanızı değiştiriyorsunuz; kayıtlı parola yeniden bağlanmada istenir.",
    })[k],
  selfLockoutTitle: "Kendinizi kilitleyebilirsiniz",
  memberships: {
    add: "Ekle",
    addLabel: "Eklenecek rol",
    choose: "Rol seçin…",
    empty: "Üyelik yok.",
    none: "Eklenebilecek rol yok.",
    admin: "ADMIN OPTION",
    adminLabel: (r: string) => `${r} için ADMIN OPTION`,
    inherit: (v: boolean) => `INHERIT: ${v ? "evet" : "hayır"}`,
    set: (v: boolean) => `SET: ${v ? "evet" : "hayır"}`,
    pg16Hint: "INHERIT/SET seçenekleri (PG 16+) salt okunurdur.",
    remove: (r: string) => `${r} üyeliğini kaldır`,
    removed: "Kaldırılacaklar",
    restore: (r: string) => `${r} üyeliğini geri al`,
    revokeNote: "Bu rolün o roldeki yetkileri kaybolur (REVOKE).",
  },
  unsupported: {
    note: "Aşağıdakiler bu formda gösterilir ama düzenlenmez: veritabanına özgü ayarlar (IN DATABASE), birden çok grantor ya da başka bir grantor'ın verdiği üyelikler.",
    perDatabase: "Veritabanına özgü ayarlar",
    multiGrantor: "Birden çok grantor",
  },
  dropNote: "Rolün sahip olduğu nesneler varsa silinemez: REASSIGN OWNED / DROP OWNED.",
} as const;
