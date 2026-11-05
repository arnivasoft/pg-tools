//! Kullanıcının sorgu oturumunda çalışan plan aracı (`explain_query`).

use std::sync::Arc;
use std::time::Duration;

use pgt_core::ErrorPayload;
use pgt_query::explain::ExplainOptions;
use pgt_query::session::{QueryManager, Scope, TxState};
use uuid::Uuid;

/// Plan aracının süre sınırı: aşılınca oturumdaki EXPLAIN sunucu tarafında iptal edilir.
pub const EXPLAIN_LIMIT: Duration = Duration::from_secs(15);

/// `EXPLAIN` (ANALYZE'sız: ifade planlanır, çalıştırılmaz) kullanıcının sorgu oturumunda: arama yolu kullanıcınınki,
/// tek ifade ve kilit/üretim kapıları görsel EXPLAIN'deki gibi (`QueryManager::explain`). Kullanıcının açık ya da
/// hatalı transaction'ı varsa çalışmaz: hatalı bir plan isteği onun transaction'ını bozmasın. Oturum meşgulse
/// `QueryManager` beklemeden reddeder. Ayrı görevde koşar: asistan isteği iptal edilse de EXPLAIN yarıda bırakılmaz
/// (oturum tutarlı kalır); süre aşılırsa sunucuda iptal edilir.
pub async fn explain_in_session(query: Arc<QueryManager>, session: Option<Uuid>, sql: String) -> Result<String, String> {
    let Some(session) = session else {
        return Err("This query tab has no open session yet; ask the user to run any query in the tab first, or answer without the plan.".into());
    };
    match query.tx_state(session) {
        Ok(TxState::Idle) => {}
        Ok(_) => {
            return Err("The user has an open or failed transaction in this tab; EXPLAIN is not run so it is not disturbed. Ask them to COMMIT or ROLLBACK first, or answer without the plan.".into());
        }
        Err(e) => return Err(format!("The query session is not available: {}", ErrorPayload::from(&e).message)),
    }
    let task = tokio::spawn(async move {
        let opts = ExplainOptions { analyze: false, ..ExplainOptions::default() };
        let fut = query.explain(session, &sql, Scope::All, opts, false);
        tokio::pin!(fut);
        tokio::select! {
            r = &mut fut => r,
            () = tokio::time::sleep(EXPLAIN_LIMIT) => {
                query.cancel(session);
                fut.await
            }
        }
    });
    match task.await {
        Ok(Ok(res)) => Ok(crate::tools::clip_plan(&res.json)),
        Ok(Err(e)) => {
            let p = ErrorPayload::from(&e);
            Err(match p.detail {
                Some(d) => format!("EXPLAIN failed: {} ({d})", p.message),
                None => format!("EXPLAIN failed: {}", p.message),
            })
        }
        Err(_) => Err("EXPLAIN failed unexpectedly.".into()),
    }
}
