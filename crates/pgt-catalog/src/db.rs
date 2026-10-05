use std::sync::Arc;

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::{Error, Result};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::version::{PgVersion, VersionedSql};

pub(crate) type Params = Arc<Vec<Box<dyn ToSql + Sync + Send>>>;

/// `p![oid, attnum]` → yeniden denemede tekrar kullanılabilen parametre listesi.
macro_rules! p {
    ($($x:expr),* $(,)?) => {
        std::sync::Arc::new(vec![$(Box::new($x) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>),*])
    };
}
pub(crate) use p;

/// Sunucu sürümüne uygun SQL varyantını seçip çalıştırır.
pub(crate) async fn query(mgr: &ConnectionManager, server: Uuid, target: Target, sql: &'static VersionedSql, params: Params) -> Result<Vec<Row>> {
    mgr.run(server, target, move |c| {
        let params = params.clone();
        async move {
            let text = sql.pick(PgVersion(c.version_num()));
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect();
            c.client.query(text, &refs).await
        }
    })
    .await
}

/// Tek satır bekler; satır yoksa `Error::NotFound(what)`.
#[allow(dead_code)] // sonraki görevlerde kullanılacak
pub(crate) async fn query_one(mgr: &ConnectionManager, server: Uuid, target: Target, sql: &'static VersionedSql, params: Params, what: &str) -> Result<Row> {
    query(mgr, server, target, sql, params).await?.into_iter().next().ok_or_else(|| Error::NotFound(what.to_string()))
}
