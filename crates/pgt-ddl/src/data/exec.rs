//! Veri düzenleyici yürütücüsü: planın DML ifadelerini **tek transaction'da** çalıştırır, `RETURNING` satırlarını
//! toplar ve her ifadenin **tam 1 satır** etkilediğini denetler.
//!
//! - Her ifade ayrı `simple_query` ile (metin protokolü: dönen hücreler Query Tool okumasıyla aynı metin biçimindedir;
//!   ifade tek olduğundan çok ifadeli dizge sorunu yok). Etkilenen satır sayısı `CommandComplete`'ten.
//! - ≠ 1 satır (bayat/silinmiş satır, `BEFORE` tetikleyicisi `RETURN NULL`, kural) → `ROLLBACK` + `Conflict`.
//! - PG hatası → `ROLLBACK` + `Failed` (konum `error_position`); iptal bayrağı her ifadeden ve `COMMIT`'ten önce
//!   denetlenir (bayrak kalkmışsa `ROLLBACK` + iptal sonucu).
//! - `COMMIT` hatası: SQLSTATE'li (ertelenmiş kısıt) → `Failed` (son ifade); SQLSTATE'siz → sonuç belirsiz.
//! - `standard_conforming_strings` her ifade öncesi yeniden kurulmaz (DDL'den farklı): ifadeler serbest SQL içermez
//!   ve `quote_literal` ters eğik çizgili değeri `E'…'` ile yazdığı için metin bu ayardan bağımsızdır.
//!
//! SQL metni, değerler ve dönen satırlar günlüğe yazılmaz.

use pgt_core::Error;
use tokio_postgres::{Client, SimpleQueryMessage};

use super::model::RowResult;
use crate::apply::{self, Registration};
use crate::plan::Statement;

/// Yürütme sonucu (süre/günlük çağıranda). `Debug` yalnızca sıra ve sonuç sayısını yazar.
pub(crate) enum ExecResult {
    Done(Vec<RowResult>),
    /// `index`. ifade 1'den farklı sayıda satır etkiledi; transaction geri alındı.
    Conflict { index: u32 },
    /// `outcome_unknown`: `COMMIT` yanıtı alınamadı (G/Ç); kalıcılaşmış olabilir.
    Failed { index: u32, error: Error, position: Option<u32>, outcome_unknown: bool },
}

impl std::fmt::Debug for ExecResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecResult::Done(r) => f.debug_tuple("Done").field(&r.len()).finish(),
            ExecResult::Conflict { index } => f.debug_struct("Conflict").field("index", index).finish(),
            ExecResult::Failed { index, error, position, outcome_unknown } => f
                .debug_struct("Failed")
                .field("index", index)
                .field("sqlstate", &apply::sqlstate(error))
                .field("position", position)
                .field("outcome_unknown", outcome_unknown)
                .finish(),
        }
    }
}

/// Tek ifadenin çıktısı: etkilenen satır sayısı ve (varsa) ilk dönen satır.
struct Affected {
    rows: u64,
    returned: Option<Vec<Option<String>>>,
}

fn affected(messages: Vec<SimpleQueryMessage>) -> Affected {
    let mut out = Affected { rows: 0, returned: None };
    for m in messages {
        match m {
            SimpleQueryMessage::Row(r) if out.returned.is_none() => {
                out.returned = Some((0..r.len()).map(|i| r.get(i).map(str::to_string)).collect());
            }
            SimpleQueryMessage::CommandComplete(n) => out.rows = n,
            _ => {}
        }
    }
    out
}

/// `BEGIN`; ifadeler sırayla; her biri tam 1 satır; `COMMIT`. Bağlantı çağıranındır (uygulama oturumu).
pub(crate) async fn run_changes(client: &Client, statements: &[Statement], reg: &Registration<'_>) -> ExecResult {
    if reg.cancelled() {
        return ExecResult::Failed { index: 0, error: apply::cancelled_error(), position: None, outcome_unknown: false };
    }
    if let Err(e) = client.batch_execute("BEGIN").await {
        let (error, position) = apply::statement_error(e, reg);
        return ExecResult::Failed { index: 0, error, position, outcome_unknown: false };
    }
    let mut results = Vec::with_capacity(statements.len());
    for (k, s) in statements.iter().enumerate() {
        let index = apply::idx(k);
        if reg.cancelled() {
            apply::rollback(client).await;
            return ExecResult::Failed { index, error: apply::cancelled_error(), position: None, outcome_unknown: false };
        }
        match client.simple_query(&s.sql).await {
            Ok(messages) => {
                let a = affected(messages);
                if a.rows != 1 {
                    apply::rollback(client).await;
                    return ExecResult::Conflict { index };
                }
                results.push(RowResult { index, returned: a.returned });
            }
            Err(e) => {
                let (error, position) = apply::statement_error(e, reg);
                apply::rollback(client).await;
                return ExecResult::Failed { index, error, position, outcome_unknown: false };
            }
        }
    }
    // Son ifade çalışırken gelen (sunucuda bir şeyi kesmeye yetişmeyen) iptal: COMMIT yerine geri alınır.
    if reg.cancelled() {
        apply::rollback(client).await;
        let index = apply::idx(statements.len().saturating_sub(1));
        return ExecResult::Failed { index, error: apply::cancelled_error(), position: None, outcome_unknown: false };
    }
    // SQLSTATE'li COMMIT hatasında sunucu transaction'ı zaten geri almıştır; SQLSTATE yoksa sonuç belirsizdir.
    if let Err(e) = client.batch_execute("COMMIT").await {
        let outcome_unknown = e.code().is_none();
        // Konum `COMMIT` metnine aittir, son ifadeye değil → verilmez.
        let (error, _) = apply::statement_error(e, reg);
        let index = apply::idx(statements.len().saturating_sub(1));
        return ExecResult::Failed { index, error, position: None, outcome_unknown };
    }
    ExecResult::Done(results)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pgt_core::testing::{admin_client, test_servers};
    use uuid::Uuid;

    use super::*;
    use crate::apply::CancelMap;
    use crate::plan::Risk;

    fn st(sql: &str) -> Statement {
        Statement { sql: sql.into(), risk: Risk::Safe, transactional: true, note: None, redacted: false }
    }

    /// Docker (`PGT_TEST_URLS` yoksa atlanır). İptal bayrağı son ifade çalışırken kurulur (sunucuya iptal isteği
    /// gitmez, ifade biter): `COMMIT` yerine `ROLLBACK` + iptal sonucu (son ifade); hiçbir satır kalıcı değildir.
    #[tokio::test]
    async fn cancel_flagged_before_commit_rolls_back() {
        for ts in test_servers() {
            let c = admin_client(&ts, &ts.config.maintenance_db).await;
            c.batch_execute("CREATE TEMP TABLE pgt_exec_cancel (id int)").await.unwrap();
            let map = CancelMap::default();
            let id = Uuid::new_v4();
            let reg = map.register(id).unwrap();
            let stmts = [st("INSERT INTO pgt_exec_cancel VALUES (1)"), st("INSERT INTO pgt_exec_cancel SELECT 2 FROM pg_sleep(1)")];
            let (res, _) = tokio::join!(run_changes(&c, &stmts, &reg), async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                // Tutamaç yok → yalnızca bayrak.
                assert!(matches!(map.request(id), Some(None)));
            });
            let ExecResult::Failed { index, error, position, outcome_unknown } = res else { panic!("{res:?}") };
            assert_eq!((index, apply::sqlstate(&error), position, outcome_unknown), (1, Some("57014"), None, false));
            let n: i64 = c.query_one("SELECT count(*) FROM pgt_exec_cancel", &[]).await.unwrap().get(0);
            assert_eq!(n, 0, "iptal edilen uygulama kalıcılaşmamalı");
            // Bağlantı transaction dışında kalır.
            c.batch_execute("DROP TABLE pgt_exec_cancel").await.unwrap();
        }
    }
}
