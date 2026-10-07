//! Veri düzenleyici yürütücüsü: planın DML ifadelerini **tek transaction'da** çalıştırır, `RETURNING` satırlarını
//! toplar ve her ifadenin **tam 1 satır** etkilediğini denetler.
//!
//! - Her ifade ayrı `simple_query` ile (metin protokolü: dönen hücreler Query Tool okumasıyla aynı metin biçimindedir;
//!   ifade tek olduğundan çok ifadeli dizge sorunu yok). Etkilenen satır sayısı `CommandComplete`'ten.
//! - ≠ 1 satır (bayat/silinmiş satır, `BEFORE` tetikleyicisi `RETURN NULL`, kural) → `ROLLBACK` + `Conflict`.
//! - PG hatası → `ROLLBACK` + `Failed` (konum `error_position`); iptal bayrağı her ifadeden önce denetlenir.
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
