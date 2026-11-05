//! Tarafı tutarlı anlık görüntüyle yükler: ayrılmış (havuz dışı) katalog bağlantısında tek bir
//! `REPEATABLE READ READ ONLY` transaction; `search_path` yalnız o transaction için `pg_catalog, <şema>`.
//! Havuzdaki bağlantılar etkilenmez; ayrılmış bağlantı her çıkış yolunda düşer.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use pgt_core::connection::{ConnectionManager, SessionProfile, Target};
use pgt_core::{Error, ErrorPayload};
use pgt_ddl::{AclDef, ObjectDef, ObjectRef, acl_supported, load_acl_in, load_in, quote_ident};
use tokio_postgres::Client;

use crate::cancel::CancelToken;
use crate::key::Listed;
use crate::list::{SchemaCheck, list_objects, schema_check};
use crate::model::DiffSide;

/// Aynı istemcide eşzamanlı bekleyen yükleme sayısı (sıra korunur).
pub const LOAD_PIPELINE: usize = 8;
/// Her bu kadar nesnede bir ilerleme bildirilir (ve sonda).
pub const PROGRESS_EVERY: u32 = 25;
/// İptalde sunucuya iletilen iptal isteğini bekleme üst sınırı.
const CANCEL_WAIT: Duration = Duration::from_secs(2);

/// Listelenen bir nesne ve yükleme sonucu. Nesne düzeyindeki hata ileti metni olarak taşınır.
#[derive(Debug)]
pub struct Loaded {
    pub listed: Listed,
    pub def: Result<ObjectDef, String>,
    /// Yetki seçeneği kapalıysa ya da tür yetki desteklemiyorsa `None`.
    pub acl: Option<Result<AclDef, String>>,
}

#[derive(Debug)]
pub struct SideSnapshot {
    pub version_num: u32,
    pub version: String,
    pub database: String,
    pub viewer: String,
    pub objects: Vec<Loaded>,
}

#[derive(Debug)]
pub enum SnapshotError {
    Cancelled,
    Schema(SchemaCheck),
    Core(Error),
}

impl From<Error> for SnapshotError {
    fn from(e: Error) -> Self {
        SnapshotError::Core(e)
    }
}

impl From<tokio_postgres::Error> for SnapshotError {
    fn from(e: tokio_postgres::Error) -> Self {
        SnapshotError::Core(Error::from_query(e))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadErrorClass {
    /// Yalnız o nesne yüklenemedi; iş sürer (transaction bozulmaz).
    Item,
    /// Bağlantı/transaction düzeyinde hata; tarafın yüklemesi biter.
    Fatal,
}

/// `Invalid` (sistem nesnesi, geçersiz metin vb.) ve `NotFound` (yarışta silinmiş) nesne hatasıdır; sunucu
/// sorgu hatası transaction'ı bozduğu için ölümcüldür.
pub fn classify_load_error(e: &Error) -> LoadErrorClass {
    match e {
        Error::Invalid(_) | Error::NotFound(_) => LoadErrorClass::Item,
        _ => LoadErrorClass::Fatal,
    }
}

fn item_message(e: &Error) -> String {
    ErrorPayload::from(e).message
}

async fn load_one(
    client: &Client,
    version: u32,
    with_acl: bool,
    listed: Listed,
) -> Result<Loaded, Error> {
    let obj = ObjectRef {
        kind: listed.kind,
        oid: listed.oid,
    };
    let def = match load_in(client, version, obj).await {
        Ok(d) => Ok(d),
        Err(e) if classify_load_error(&e) == LoadErrorClass::Item => Err(item_message(&e)),
        Err(e) => return Err(e),
    };
    let acl = if with_acl && acl_supported(listed.kind) {
        match load_acl_in(client, version, obj).await {
            Ok(a) => Some(Ok(a)),
            Err(e) if classify_load_error(&e) == LoadErrorClass::Item => {
                Some(Err(item_message(&e)))
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    Ok(Loaded { listed, def, acl })
}

/// Nesneleri sırayla (en çok `LOAD_PIPELINE` eşzamanlı) yükler. `report(yüklenen, toplam)` her
/// `PROGRESS_EVERY` nesnede ve sonda çağrılır; `false` dönerse iş iptal sayılır.
#[doc(hidden)]
pub async fn load_objects(
    client: &Client,
    version: u32,
    objects: Vec<Listed>,
    with_acl: bool,
    report: &(dyn Fn(u32, u32) -> bool + Sync),
    cancel: &CancelToken,
) -> Result<Vec<Loaded>, SnapshotError> {
    let total = objects.len() as u32;
    let mut out = Vec::with_capacity(objects.len());
    let mut reported = 0u32;
    let mut s = stream::iter(objects)
        .map(|l| load_one(client, version, with_acl, l))
        .buffered(LOAD_PIPELINE);
    while let Some(r) = s.next().await {
        out.push(r?);
        let done = out.len() as u32;
        if done.is_multiple_of(PROGRESS_EVERY) {
            reported = done;
            if !report(done, total) || cancel.is_cancelled() {
                return Err(SnapshotError::Cancelled);
            }
        }
    }
    if reported != total && !report(total, total) {
        return Err(SnapshotError::Cancelled);
    }
    Ok(out)
}

/// `search_path` değeri: `pg_catalog` önde, şema gerekirse tırnaklı.
fn search_path_value(schema: &str) -> String {
    format!("pg_catalog, {}", quote_ident(schema))
}

async fn run(
    client: &Client,
    version: String,
    side: &DiffSide,
    with_acl: bool,
    report: &(dyn Fn(u32, u32) -> bool + Sync),
    cancel: &CancelToken,
) -> Result<SideSnapshot, SnapshotError> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;
    let path = search_path_value(&side.schema);
    client
        .execute(
            "SELECT pg_catalog.set_config('search_path', $1::pg_catalog.text, true)",
            &[&path],
        )
        .await?;
    match schema_check(client, &side.schema).await? {
        SchemaCheck::Ok => {}
        other => return Err(SnapshotError::Schema(other)),
    }
    let row = client
        .query_one(
            "SELECT pg_catalog.current_database()::pg_catalog.text, current_user::pg_catalog.text, \
             pg_catalog.current_setting('server_version_num')::pg_catalog.int4",
            &[],
        )
        .await?;
    let (database, viewer, version_num): (String, String, i32) =
        (row.get(0), row.get(1), row.get(2));
    let version_num = version_num as u32;
    let listed = list_objects(client, version_num, &side.schema).await?;
    let objects = load_objects(client, version_num, listed, with_acl, report, cancel).await?;
    client.batch_execute("COMMIT").await?;
    Ok(SideSnapshot {
        version_num,
        version,
        database,
        viewer,
        objects,
    })
}

/// Tarafı yükler. İptal (`cancel` ya da `report` false) → `Cancelled`; ayrılmış bağlantı her durumda düşer
/// (açık transaction sunucuda geri alınır).
pub async fn load_side(
    mgr: &ConnectionManager,
    side: &DiffSide,
    with_acl: bool,
    report: &(dyn Fn(u32, u32) -> bool + Sync),
    cancel: &CancelToken,
) -> Result<SideSnapshot, SnapshotError> {
    let started = Instant::now();
    if cancel.is_cancelled() {
        return Err(SnapshotError::Cancelled);
    }
    let conn = tokio::select! {
        r = mgr.open_dedicated(side.server_id, Target::Database(side.database_oid), SessionProfile::Catalog) => r?,
        _ = cancel.cancelled() => return Err(SnapshotError::Cancelled),
    };
    let mut work = std::pin::pin!(run(
        &conn.client,
        conn.info.version.clone(),
        side,
        with_acl,
        report,
        cancel
    ));
    let result = tokio::select! {
        r = &mut work => r,
        _ = cancel.cancelled() => {
            // Sunucuda süren sorguyu iptal etmeyi dener; ardından gelecek ve bağlantı düşer.
            let _ = tokio::time::timeout(CANCEL_WAIT, conn.cancel.cancel()).await;
            Err(SnapshotError::Cancelled)
        }
    };
    let outcome = match &result {
        Ok(_) => "ok",
        Err(SnapshotError::Cancelled) => "cancelled",
        Err(_) => "failed",
    };
    let count = result.as_ref().map(|s| s.objects.len()).unwrap_or(0);
    tracing::info!(
        outcome,
        objects = count,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "taraf yüklendi"
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_table() {
        let q = || Error::Query {
            sqlstate: None,
            message: "x".into(),
            detail: None,
            hint: None,
        };
        assert_eq!(
            classify_load_error(&Error::Invalid("x".into())),
            LoadErrorClass::Item
        );
        assert_eq!(
            classify_load_error(&Error::NotFound("x".into())),
            LoadErrorClass::Item
        );
        assert_eq!(classify_load_error(&q()), LoadErrorClass::Fatal);
        assert_eq!(
            classify_load_error(&Error::ConnectionLost("x".into())),
            LoadErrorClass::Fatal
        );
        assert_eq!(
            classify_load_error(&Error::NotConnected),
            LoadErrorClass::Fatal
        );
        assert_eq!(
            classify_load_error(&Error::PasswordRequired),
            LoadErrorClass::Fatal
        );
    }

    #[test]
    fn search_path_quotes_schema() {
        assert_eq!(search_path_value("s"), "pg_catalog, s");
        assert_eq!(search_path_value("My S"), "pg_catalog, \"My S\"");
    }
}
