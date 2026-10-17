# pg-tools — Alt Proje 7: Yetkiler (GRANT/REVOKE) ve Varsayılan Yetkiler Tasarımı

- **Tarih:** 2026-10-17
- **Durum:** Taslak (Plan 20 arka ucu yazıldı; Plan 21 ön yüzü kalan)
- **Kapsam:** Explorer'dan bir nesnenin **yetkilerini** görüntüleme ve düzenleme (GRANT/REVOKE; `WITH GRANT OPTION`) ve **varsayılan yetkileri** (`ALTER DEFAULT PRIVILEGES`) düzenleme; önizlemeli, korumalı uygulama.
- **Temel:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (mimari §2, risk modeli §3, arayüz §4, güvenlik §6) — `pgt-ddl` çerçevesi aynen kullanılır; bu belge yalnız yetki modelini, kurallarını ve kararlarını ekler. O spec'in §1.3 "GRANT/REVOKE matrisi, varsayılan yetkiler" kapsam dışı maddesi buraya taşınır.
- **Teslim planları:** Plan 20 (arka uç: model, yetki kataloğu, saf üreticiler, yükleyiciler, yönetici entegrasyonu, PG 10–18 Docker matrisi) → Plan 21 (ön yüz: "Yetkiler…" ve "Varsayılan yetkiler…" sekmeleri, yetki ızgarası).

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| Nesne türleri | Table (sıradan ve bölümlenmiş), View, Materialized View, Sequence, Function, Procedure (PG 11+), Schema, Database, Type, Domain. Kolon düzeyi yetkiler **salt okunur** listelenir. Tablespace, dil, FDW/sunucu, large object, parametre (PG 15+ `GRANT SET`) yetkileri ve `GRANTED BY` kapsam dışı (backlog). |
| Etkin ACL | Yükleme `coalesce(acl, acldefault(tür, sahip))` → `aclexplode`: NULL ACL (varsayılan) de açıkça gösterilir (ör. fonksiyonlarda `PUBLIC EXECUTE`, veritabanında `PUBLIC CONNECT, TEMPORARY`, tiplerde `PUBLIC USAGE`). İlk değişiklik ACL'yi açık hâle getirir (PG davranışı). |
| Düzenlenebilir satırlar | Yalnızca **verenin (grantor) nesne sahibi** olduğu girdiler düzenlenir (süper kullanıcının verdiği yetkiler PG'de sahip adına kaydedilir). Başka bir rolün `WITH GRANT OPTION` ile verdiği yetkiler **salt okunur** (veren adıyla) listelenir; onları geri almak o rolün işidir. |
| Sahip satırı | Sahibin kendi yetkileri gösterilir ve düzenlenebilir (PG izin verir); sahibin kendi yetkisini kaldırması **uyarı** üretir (sahip istediği an geri verebilir). |
| Üretilen ifadeler | Alıcı (grantee) başına fark: eklenen yetkiler `GRANT p, … ON <nesne> TO <alıcı>`; `WITH GRANT OPTION` eklenenler ayrı `GRANT … WITH GRANT OPTION`; kaldırılanlar `REVOKE p, … ON <nesne> FROM <alıcı>`; yetki kalıp yalnız seçenek kaldırılırsa `REVOKE GRANT OPTION FOR p ON … FROM …`. `ALL PRIVILEGES` yazılmaz (açık liste). Sıra: REVOKE'lar, REVOKE GRANT OPTION FOR'lar, GRANT'lar; alıcılar `PUBLIC` önce, sonra ad sırası. |
| CASCADE | `REVOKE` varsayılan olarak `RESTRICT`'tir: alıcının başkalarına verdiği bağımlı yetkiler varsa PG `2BP01` ile reddeder (ipucu). Kullanıcı açıkça **"bağımlı yetkilerle birlikte geri al (CASCADE)"** seçebilir. |
| Risk | `GRANT` Safe; `REVOKE` (ve `GRANT OPTION FOR`) **Destructive** (yetki kaybı; Plan 14 üyelik REVOKE kuralı); `REVOKE … CASCADE` **DataLoss** (başka rollerin verdiği yetkiler de silinir; üretimde ad yazma). Varsayılan yetkiler: `GRANT` ve `REVOKE` **Safe** + not "yalnızca bundan sonra oluşturulan nesneleri etkiler". |
| Nesne adresleme | Tablo/görünüm/MV `ON TABLE <nitelikli ad>`; dizi `ON SEQUENCE`; fonksiyon/procedure `ON FUNCTION|PROCEDURE <qualified(şema, ad)>(<pg_get_function_identity_arguments>)` (imza metni sunucudan; yeniden yükleme eşitliği sunucu metnini doğrular, nihai ifade `composed_is_clean` ile taranır); şema `ON SCHEMA`; veritabanı `ON DATABASE`; type/domain `ON TYPE` / `ON DOMAIN`. |
| Yetki kümeleri | Tablo/görünüm/MV: `SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER` (+ `MAINTAIN` PG 17+); dizi: `USAGE, SELECT, UPDATE`; fonksiyon/procedure: `EXECUTE`; şema: `USAGE, CREATE`; veritabanı: `CONNECT, CREATE, TEMPORARY`; type/domain: `USAGE`. Başka yetki doğrulama hatası. |
| Varsayılan yetkiler | Kapsam: bir rol (`FOR ROLE`) için **genel** (veritabanı düzeyi) ya da **şema** (`IN SCHEMA`). Nesne türleri: Tables, Sequences, Functions (procedure'leri de kapsar), Types, Schemas (PG 10+, yalnız genel). Genel kapsamda etkin değer `coalesce(defaclacl, acldefault(...))`; şema kapsamında yalnız **eklenen** yetkiler düzenlenir (PG şema düzeyinde genelden geri alma yapamaz) ve genel taban salt okunur gösterilir. |
| Kendini kilitleme uyarıları | Bağlı rolden yetki geri alma, bakım veritabanında `PUBLIC`/bağlı rolden `CONNECT` geri alma, sahibin kendi yetkisini kaldırması, şemadan `USAGE` geri alma (bağlı rol): **uyarı** (engellenmez). |
| Kapsam | Database ACL'i sunucu kapsamlıdır (bakım veritabanında uygulanır; Plan 14 kuralı); diğerleri veritabanı kapsamlı. |

## 2. Model (özet)

- `AclDef { kind, schema, name, signature, owner, viewer, explicit, grants: [AclGrant{ grantee: string | null (PUBLIC), privileges: [{ privilege, grantOption }] }], foreign: [ForeignGrant{ grantor, grantee, privilege, grantOption }] (salt okunur), columns: [ColumnGrant] (salt okunur), editable, unsupported }` — `viewer`: bağlı rol (`current_user`; kendini kilitleme uyarıları için), `explicit`: ACL NULL değil.
- `DefaultAclDef { role, schema: string | null, viewer, sections: [{ objectType, grants: [AclGrant], baseline: [AclGrant] (yalnız şema kapsamında; salt okunur genel taban) }], editable, unsupported }`; kapsam `DefaultAclScope { role: string | null (null → bağlı rol), schemaOid: number | null }`.
- Plan istekleri `PlanRequest::Privileges { object, old, new, cascade }` ve `PlanRequest::DefaultPrivileges { scope, old, new }`; önizleme/uygulama mevcut `ddl_preview`/`ddl_apply` kapılarından geçer (kilit, yeniden yükleme eşitliği, plan özeti, onay, üretimde ad yazma). Uygulama sonrası ACL yeniden yüklenir; istenenden farklıysa (ör. bağlı rol sahip değil ve yetki kendi adıyla "veren" olarak kaydedildi) uyarı döner (Plan 14 üyelik uyarısı kalıbı).

## 3. Arayüz (Plan 21 özeti)

- Explorer: desteklenen nesne düğümlerinde **"Yetkiler…"**; veritabanı ve şema düğümünde **"Varsayılan yetkiler…"**.
- Yetki ızgarası: satır = alıcı (PUBLIC, roller; "Alıcı ekle" — `grantable_roles` lookup), sütun = türün yetkileri; hücre üç durumlu (yok / var / var + GRANT OPTION); sahip satırı işaretli; başka verenlerin yetkileri ve kolon yetkileri salt okunur bölümde; CASCADE seçeneği; önizleme + kapılı uygulama (mevcut designer çerçevesi).
- Varsayılan yetkiler sekmesi: rol seçici (varsayılan: bağlı rol), nesne türü başına ızgara; şema kapsamında genel taban salt okunur.

## 4. Güvenlik

- Tanımlayıcılar yalnız `quote_ident`/`qualified`; fonksiyon imzası sunucunun `regprocedure` metni (yeniden yükleme eşitliği + nihai ifade taraması); yetki adları sabit kümeden.
- Yetki listeleri, rol adları ve SQL `tracing`'e yazılmaz.

## 5. Test stratejisi

- Saf üreticiler: insta (her tür; ekleme/kaldırma/seçenek yükseltme–düşürme/CASCADE; PUBLIC; sahip satırı; yabancı veren satırı değişmez; varsayılan yetkiler genel/şema), sürüm kapıları (MAINTAIN PG 17+, procedure PG 11+, Schemas default ACL PG 10+).
- Docker PG 10–18: `apply → load → beklenen`, `alter(loaded, loaded)` boş plan, NULL ACL'nin etkin gösterimi ve ilk değişiklik, yabancı veren (`WITH GRANT OPTION` alan rolün verdiği yetki), `RESTRICT` 2BP01 ve `CASCADE`, varsayılan yetkilerin yeni nesneye uygulanması, `FOR ROLE` yetkisizlik 42501.

## 6. Planlama notu

- **Plan 20 (arka uç):** model + istekler → yetki kataloğu + nesne ACL üreticisi → varsayılan yetki üreticisi → nesne ACL yükleyicisi → varsayılan yetki yükleyicisi → yönetici/komut entegrasyonu → belgeler ve matris.
- **Plan 21 (ön yüz):** menü kancası + telafi → yetki ızgarası modeli → "Yetkiler…" sekmesi → "Varsayılan yetkiler…" sekmesi → smoke listesi.
