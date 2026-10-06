//! İfade çalıştırıcı: tek bir sorgu oturumunda (`DedicatedConn`) ifadeleri birer birer çalıştırır ve sonucu
//! `QueryEvent` olarak akıtır.
//!
//! - Satır döndüren ifadeler (`prepare` sütun bildirir) mümkünse `DECLARE pgt_c NO SCROLL CURSOR` + parça parça
//!   `FETCH FORWARD` ile okunur (bellek `chunk` ile sınırlı; ilk yüklemede toplam `limit`, devamı `more()`).
//!   İmlece uygun olmayanlar (INSERT … RETURNING, EXPLAIN, SHOW…) ya da imleç açılamayan durumlar doğrudan
//!   yoldan (`simple_query` akışı, `limit`'ten sonrası atılır) okunur.
//! - Değerler metin protokolüyle gelir: `NULL` → `None`, boş dize → `Some("")`.
//! - Otomatik onay (autocommit) kipinde imleç için gereken transaction'ı çalıştırıcı açar ve kapatır; kullanıcının
//!   açık bir transaction'ı varsa ya da manuel kipte (`autocommit = false`) kendi BEGIN/COMMIT/ROLLBACK'ini asla
//!   eklemez.
//! - NOTICE'lar her sunucu çağrısı **sürerken** kanaldan alınır (ifade başına en çok `NOTICES_PER_STMT_MAX`
//!   tutulur, fazlası sayılıp atılır): uzun bir NOTICE seli kanalda birikmez.
//!
//! SQL metni asla günlüğe yazılmaz: `tracing` alanlarında yalnızca `kind`, `elapsed_ms`, `rows_sent` ve hata
//! `code`'u bulunur.

use std::future::Future;
use std::time::Instant;

use futures_util::StreamExt;
use pgt_core::connection::{CancelHandle, DedicatedConn, Notice, ServerInfo};
use pgt_core::{Error, ErrorPayload, Result};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_postgres::SimpleQueryMessage;
use tokio_postgres::error::ErrorPosition;

use crate::classify::{StmtKind, TxOp, analyze, tx_op};
use crate::events::{Column, EventSink, NOTICES_PER_STMT_MAX, NoticeInfo, QueryEvent};
use crate::lex::{Kind, lex};

/// `ExecConfig.limit` üst sınırı; aşan değer buna kısılır.
pub const MAX_LIMIT: u32 = 1_000_000;
/// Parça boyu; aynı zamanda üst sınırıdır (daha büyük `chunk` buna kısılır).
pub const DEFAULT_CHUNK: u32 = 1000;

const DECLARE_PREFIX: &str = "DECLARE pgt_c NO SCROLL CURSOR FOR ";
const BEGIN_DECLARE_PREFIX: &str = "BEGIN; DECLARE pgt_c NO SCROLL CURSOR FOR ";

pub(crate) type TextRows = Vec<Vec<Option<String>>>;
type PgResult<T> = std::result::Result<T, tokio_postgres::Error>;

/// Çalıştırma ayarları. `limit`: ilk yüklemede gönderilecek toplam satır sınırı (0 geçersiz, en çok
/// `MAX_LIMIT`); `chunk`: tek `Rows` olayındaki en çok satır (0 geçersiz, en çok `DEFAULT_CHUNK`);
/// `autocommit`: imleç için gereken transaction'ı çalıştırıcı yönetebilir mi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecConfig {
    pub limit: u32,
    pub chunk: u32,
    pub autocommit: bool,
}

impl ExecConfig {
    fn checked(&self) -> Result<ExecConfig> {
        if self.limit == 0 {
            return Err(Error::Invalid("satır sınırı 0 olamaz".into()));
        }
        if self.chunk == 0 {
            return Err(Error::Invalid("parça boyu 0 olamaz".into()));
        }
        Ok(ExecConfig {
            limit: self.limit.min(MAX_LIMIT),
            chunk: self.chunk.min(DEFAULT_CHUNK),
            autocommit: self.autocommit,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StmtOutcome {
    Completed,
    /// İfade başarısız oldu ya da olay alıcısı gitti; betik durmalıdır.
    Failed,
}

/// Son ifadenin izlediği yol (tanı ve testler için).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Satır döndürmeyen ifade.
    NoRows,
    Cursor,
    Direct,
}

struct OpenCursor {
    /// İmleci açan ifadenin dizini (`more()` yalnızca onu kabul eder).
    stmt_index: u32,
    /// İmlecin transaction'ını çalıştırıcı açtı (kapanışta COMMIT, hatada ROLLBACK).
    own_tx: bool,
    chunk: u32,
}

enum Outcome {
    Done { tag: String, rows_affected: Option<u64>, rows_sent: u64, truncated: bool },
    /// `reached_server`: hata sunucudan geldi (ifade sunucuya ulaştı); transaction izlemesi yalnızca o zaman güncellenir.
    Failed { error: Error, position: Option<u32>, reached_server: bool },
    /// Olay alıcısı gitti.
    Gone,
}

impl Outcome {
    /// `prefix`: sunucuya gönderilen metinde ifadeden önceki karakter sayısı (PG konumu tüm metne göredir);
    /// `None` → konum ifadeyle ilgisiz (ör. `FETCH` metnine göre), atılır.
    fn failed(e: tokio_postgres::Error, prefix: Option<u32>) -> Outcome {
        let position = match (e.as_db_error().and_then(|db| db.position()), prefix) {
            (Some(ErrorPosition::Original(n)), Some(p)) => n.checked_sub(p).filter(|&n| n > 0),
            _ => None,
        };
        let reached_server = e.as_db_error().is_some();
        Outcome::Failed { error: Error::from_query(e), position, reached_server }
    }

    fn local(error: Error) -> Outcome {
        Outcome::Failed { error, position: None, reached_server: false }
    }
}

enum CursorOpen {
    Opened,
    /// İmleç kullanılamaz; doğrudan yol kullanılmalı.
    Fallback,
    Failed(Outcome),
}

/// İfade başına toplanan NOTICE'lar.
#[derive(Default)]
struct NoticeBuf {
    kept: Vec<Notice>,
    dropped: u64,
    /// Kanalda görülen en büyük birikim (testler: NOTICE'lar çalışma sırasında alınıyor mu).
    peak_backlog: usize,
}

impl NoticeBuf {
    fn push(&mut self, n: Notice) {
        if self.kept.len() < NOTICES_PER_STMT_MAX {
            self.kept.push(n);
        } else {
            self.dropped += 1;
        }
    }

    fn absorb(&mut self, rx: &mut UnboundedReceiver<Notice>) {
        self.peak_backlog = self.peak_backlog.max(rx.len());
        while let Ok(n) = rx.try_recv() {
            self.push(n);
        }
    }

    fn reset(&mut self) {
        self.kept.clear();
        self.dropped = 0;
    }
}

/// `fut`'u beklerken gelen NOTICE'ları da alır; böylece uzun bir ifadenin NOTICE seli kanalda birikmez.
async fn with_notices<F: Future>(fut: F, rx: &mut UnboundedReceiver<Notice>, buf: &mut NoticeBuf) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut open = true;
    loop {
        tokio::select! {
            biased;
            out = &mut fut => {
                // Sürücü NOTICE'ı yanıtın sonundan önce kanala koyar: kalanlar şimdi alınır.
                buf.absorb(rx);
                return out;
            }
            n = rx.recv(), if open => match n {
                Some(n) => {
                    buf.push(n);
                    buf.absorb(rx);
                }
                None => open = false,
            },
        }
    }
}

/// Bir sorgu oturumunun çalıştırıcısı (oturum başına bir tane). Aynı anda en çok bir imleç (`pgt_c`) açıktır.
pub struct Executor {
    conn: DedicatedConn,
    cursor: Option<OpenCursor>,
    /// Oturum bir transaction bloğunda mı. Kullanıcının BEGIN/COMMIT/ROLLBACK'lerinden izlenir ve sunucu
    /// yanıtlarıyla (25001, 25P01, 25P02) düzeltilir; oturum katmanı `set_in_tx` ile kesin değeri yazabilir.
    in_tx: bool,
    notices: NoticeBuf,
    last_path: Option<Path>,
    /// Bellekte aynı anda tutulan en çok satır (sınırın tutulduğunu doğrulamak için).
    peak_buffered: usize,
}

impl Executor {
    pub fn new(conn: DedicatedConn) -> Self {
        Executor {
            conn,
            cursor: None,
            in_tx: false,
            notices: NoticeBuf::default(),
            last_path: None,
            peak_buffered: 0,
        }
    }

    /// Çalışan ifadeyi iptal etmek için; `run_statement` sürerken başka bir görevden kullanılır. İptal edilen
    /// ifade 57014 ile `Failed` olur, oturum kullanılabilir kalır.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.conn.cancel.clone()
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.conn.info
    }

    /// Çalıştırıcının bildiği kadarıyla oturum bir transaction bloğunda mı.
    pub fn in_tx(&self) -> bool {
        self.in_tx
    }

    /// Oturum katmanının kesin bilgisiyle (ör. `statement_timestamp() <> transaction_timestamp()` yoklaması)
    /// transaction durumunu düzeltir.
    pub(crate) fn set_in_tx(&mut self, in_tx: bool) {
        self.in_tx = in_tx;
    }

    /// Çalıştırıcının kendi iç SQL'i (yoklama, ayar): olay yok, transaction izlemesi yok, NOTICE'lar atılır.
    /// Satırlar metin olarak döner. Açık imlece dokunmaz.
    pub(crate) async fn exec_internal(&mut self, sql: &str) -> Result<TextRows> {
        let messages = self.simple(sql).await;
        self.notices.reset();
        Ok(text_rows(&messages.map_err(Error::from_query)?))
    }

    /// Parametreli (`$n`) salt okunur katalog sorgusu; her sütun metin olmalıdır. Açık imlece ve transaction
    /// izlemesine dokunmaz, NOTICE'lar atılır.
    pub(crate) async fn query_text(
        &mut self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<TextRows> {
        let result =
            with_notices(self.conn.client.query(sql, params), &mut self.conn.notices, &mut self.notices).await;
        self.notices.reset();
        let rows = result.map_err(Error::from_query)?;
        Ok(rows
            .iter()
            .map(|r| (0..r.len()).map(|i| r.try_get::<_, Option<String>>(i).ok().flatten()).collect())
            .collect())
    }

    /// İfadenin sonuç sütun adları (`prepare`); boş liste satır döndürmeyen ifade demektir.
    pub(crate) async fn prepare_columns(&mut self, sql: &str) -> Result<Vec<String>> {
        let prepared = with_notices(self.conn.client.prepare(sql), &mut self.conn.notices, &mut self.notices).await;
        self.notices.reset();
        let stmt = prepared.map_err(Error::from_query)?;
        Ok(stmt.columns().iter().map(|c| c.name().to_string()).collect())
    }

    /// Yalnızca Parse/Describe (`prepare`; Bind/Execute yok): sonuç sütunlarının adları ve tür adları. Hazırlanan
    /// deyim düşürülünce kapanır. Açık imlece ve transaction izlemesine dokunmaz, NOTICE'lar atılır.
    pub(crate) async fn prepare_described(&mut self, sql: &str) -> Result<Vec<(String, String)>> {
        let prepared = with_notices(self.conn.client.prepare(sql), &mut self.conn.notices, &mut self.notices).await;
        self.notices.reset();
        let stmt = prepared.map_err(Error::from_query)?;
        Ok(stmt.columns().iter().map(|c| (c.name().to_string(), c.type_().name().to_string())).collect())
    }

    /// Dışa aktarma imlecinden (`pgt_x`) bir parça; bellekteki en çok satır izlenir.
    pub(crate) async fn fetch_export_chunk(&mut self, n: u32) -> Result<TextRows> {
        let rows = self.exec_internal(&format!("FETCH FORWARD {n} FROM pgt_x")).await?;
        self.peak_buffered = self.peak_buffered.max(rows.len());
        Ok(rows)
    }

    /// Bağlantı kapandı mı (sunucu oturumu sonlandırdı, ağ koptu…).
    pub fn is_closed(&self) -> bool {
        self.conn.client.is_closed()
    }

    /// `more()` ile okunabilecek açık bir imleç var mı.
    pub fn cursor_open(&self) -> bool {
        self.cursor.is_some()
    }

    pub fn last_path(&self) -> Option<Path> {
        self.last_path
    }

    /// Test yardımcısı: bellekte aynı anda tutulmuş en çok satır sayısı.
    #[doc(hidden)]
    pub fn peak_buffered_rows(&self) -> usize {
        self.peak_buffered
    }

    /// Test yardımcısı: NOTICE kanalında görülmüş en büyük birikim.
    #[doc(hidden)]
    pub fn peak_notice_backlog(&self) -> usize {
        self.notices.peak_backlog
    }

    /// Tek bir ifadeyi çalıştırır. `char_offset` olaylara aynen geçer (ifadenin betikteki karakter ofseti).
    /// Açık bir imleç varsa önce kapatılır.
    pub async fn run_statement(
        &mut self,
        idx: u32,
        sql: &str,
        char_offset: u32,
        cfg: &ExecConfig,
        sink: &mut dyn EventSink,
    ) -> StmtOutcome {
        let started = Instant::now();
        let kind = analyze(sql).kind;
        if self.cursor.is_some()
            && let Err(e) = self.close_cursor().await
        {
            tracing::debug!(code = %ErrorPayload::from(&e).code, "önceki imleç kapatılamadı");
        }
        // Önceki ifadelerden kalan iletiler bu ifadeye yazılmasın.
        self.notices.absorb(&mut self.conn.notices);
        self.notices.reset();
        self.last_path = None;

        let outcome = match cfg.checked() {
            Ok(cfg) => match unsupported(sql) {
                Some(e) => Outcome::local(e),
                None => self.execute(idx, sql, char_offset, kind, &cfg, sink).await,
            },
            Err(e) => Outcome::local(e),
        };
        self.notices.absorb(&mut self.conn.notices);
        let dropped_notices = self.notices.dropped;
        let notices: Vec<NoticeInfo> =
            std::mem::take(&mut self.notices.kept).into_iter().map(NoticeInfo::from).collect();
        self.notices.reset();
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        match outcome {
            Outcome::Done { tag, rows_affected, rows_sent, truncated } => {
                self.track_tx(sql, true);
                tracing::debug!(?kind, elapsed_ms, rows_sent, dropped_notices, "ifade tamamlandı");
                let ev =
                    QueryEvent::Done { stmt_index: idx, tag, rows_affected, rows_sent, truncated, elapsed_ms, notices };
                if sink.send(ev) {
                    StmtOutcome::Completed
                } else {
                    self.close_cursor_quietly().await;
                    StmtOutcome::Failed
                }
            }
            Outcome::Failed { error, position, reached_server } => {
                if reached_server {
                    self.track_tx(sql, false);
                }
                let code = match &error {
                    Error::Query { sqlstate: Some(s), .. } => s.clone(),
                    other => ErrorPayload::from(other).code,
                };
                tracing::debug!(?kind, elapsed_ms, %code, dropped_notices, "ifade başarısız");
                let error = ErrorPayload::from(&error);
                sink.send(QueryEvent::Failed { stmt_index: idx, error, position, char_offset, notices });
                StmtOutcome::Failed
            }
            Outcome::Gone => {
                tracing::debug!(?kind, elapsed_ms, "olay alıcısı gitti, çalıştırma durdu");
                self.close_cursor_quietly().await;
                StmtOutcome::Failed
            }
        }
    }

    /// Açık imleçten sonraki parçayı (en çok `chunk` satır) `Rows` olarak gönderir; imleç tükenirse kapatılır.
    /// Alıcı giderse imleç kapatılır. Açık imleç yoksa ya da `idx` imleci açan ifade değilse `Error::Invalid`.
    pub async fn more(&mut self, idx: u32, sink: &mut dyn EventSink) -> Result<()> {
        let chunk = match &self.cursor {
            Some(c) if c.stmt_index == idx => c.chunk,
            Some(_) => return Err(Error::Invalid("açık imleç bu ifadeye ait değil".into())),
            None => return Err(Error::Invalid("açık imleç yok".into())),
        };
        let result = self.fetch(chunk).await;
        // `more()` sırasında gelen iletiler bir sonraki ifadeye yazılmasın.
        self.notices.reset();
        let rows = match result {
            Ok(rows) => rows,
            Err(e) => {
                self.abandon_cursor().await;
                return Err(Error::from_query(e));
            }
        };
        let exhausted = rows.len() < chunk as usize;
        if !sink.send(QueryEvent::Rows { stmt_index: idx, rows, more: !exhausted }) {
            self.close_cursor_quietly().await;
            return Ok(());
        }
        if exhausted {
            self.finish_cursor().await.map_err(Error::from_query)?;
        }
        Ok(())
    }

    /// Açık imleci kapatır (çalıştırıcının açtığı transaction'ı onaylar). İmleç yoksa bir şey yapmaz.
    pub async fn close_cursor(&mut self) -> Result<()> {
        self.finish_cursor().await.map_err(Error::from_query)
    }

    async fn close_cursor_quietly(&mut self) {
        if let Err(e) = self.finish_cursor().await {
            tracing::debug!(code = ?e.code().map(|c| c.code()), "imleç kapatılamadı");
        }
    }

    /// NOTICE'ları alarak `simple_query`.
    async fn simple(&mut self, sql: &str) -> PgResult<Vec<SimpleQueryMessage>> {
        with_notices(self.conn.client.simple_query(sql), &mut self.conn.notices, &mut self.notices).await
    }

    /// NOTICE'ları alarak `batch_execute`.
    async fn batch(&mut self, sql: &str) -> PgResult<()> {
        with_notices(self.conn.client.batch_execute(sql), &mut self.conn.notices, &mut self.notices).await
    }

    async fn execute(
        &mut self,
        idx: u32,
        sql: &str,
        char_offset: u32,
        kind: StmtKind,
        cfg: &ExecConfig,
        sink: &mut dyn EventSink,
    ) -> Outcome {
        let prepared =
            with_notices(self.conn.client.prepare(sql), &mut self.conn.notices, &mut self.notices).await;
        let columns: Vec<Column> = match prepared {
            Ok(stmt) => stmt
                .columns()
                .iter()
                .map(|c| Column {
                    name: c.name().to_string(),
                    type_name: c.type_().name().to_string(),
                    type_oid: c.type_().oid(),
                })
                .collect(),
            Err(e) => return Outcome::failed(e, Some(0)),
        };
        let has_rows = !columns.is_empty();
        if !sink.send(QueryEvent::Started { stmt_index: idx, kind, columns, char_offset }) {
            return Outcome::Gone;
        }
        if !has_rows {
            self.last_path = Some(Path::NoRows);
            return self.run_without_rows(sql).await;
        }
        if cursor_capable(sql, kind) {
            match self.open_cursor(idx, sql, cfg).await {
                CursorOpen::Opened => {
                    self.last_path = Some(Path::Cursor);
                    return self.fetch_initial(idx, sql, cfg, sink).await;
                }
                CursorOpen::Fallback => {}
                CursorOpen::Failed(outcome) => return outcome,
            }
        }
        self.last_path = Some(Path::Direct);
        self.run_direct(idx, sql, cfg, sink).await
    }

    /// `pgt_c` imlecini açar.
    /// - autocommit ve transaction yok: `BEGIN; DECLARE …` tek `batch_execute` (kendi transaction'ımız).
    /// - aksi halde düz `DECLARE`. Transaction bloğu dışında 25P01 ile reddedilir ve hiçbir durum değişmez:
    ///   izleme düzeltilir; manuel kipte doğrudan yola, autocommit'te bir kez kendi transaction'ımızla denenir.
    ///   Manuel kipte izleme yanılıp oturum aslında bir transaction'daysa `DECLARE` başarılı olur ve düzeltilir.
    async fn open_cursor(&mut self, idx: u32, sql: &str, cfg: &ExecConfig) -> CursorOpen {
        let mut retried = false;
        loop {
            let own_tx = !self.in_tx && cfg.autocommit;
            let prefix = if own_tx { BEGIN_DECLARE_PREFIX } else { DECLARE_PREFIX };
            let result = self.batch(&format!("{prefix}{sql}")).await;
            // İzleme yanılmış ve oturum zaten bir transaction'daysa BEGIN 25001 uyarısı verir: transaction
            // kullanıcınındır, çalıştırıcı onu onaylamaz ya da geri almaz.
            let was_in_tx = own_tx && self.take_notice("25001");
            let own_tx = own_tx && !was_in_tx;
            let e = match result {
                Ok(()) => {
                    // Kendi transaction'ımız değilse imleç bir transaction bloğunda açıldı demektir.
                    if !own_tx {
                        self.in_tx = true;
                    }
                    self.cursor = Some(OpenCursor { stmt_index: idx, own_tx, chunk: cfg.chunk });
                    return CursorOpen::Opened;
                }
                Err(e) => e,
            };
            if was_in_tx {
                self.in_tx = true;
            }
            if own_tx {
                self.rollback_quietly().await;
            }
            let code = e.code().map(|c| c.code().to_string()).unwrap_or_default();
            if !own_tx && !was_in_tx && code == "25P01" {
                self.in_tx = false;
                if cfg.autocommit && !retried {
                    retried = true;
                    continue;
                }
                return CursorOpen::Fallback;
            }
            if code == "25P02" {
                self.in_tx = true; // kullanıcının transaction'ı zaten `aborted`
            }
            // İmleçle açılamayan sorgu: hata ayrıştırma/planlama aşamasında, ifade hiç çalışmadı.
            if own_tx && matches!(code.as_str(), "42P11" | "42601" | "0A000") {
                return CursorOpen::Fallback;
            }
            return CursorOpen::Failed(Outcome::failed(e, Some(prefix.chars().count() as u32)));
        }
    }

    async fn fetch_initial(&mut self, idx: u32, sql: &str, cfg: &ExecConfig, sink: &mut dyn EventSink) -> Outcome {
        let limit = u64::from(cfg.limit);
        let mut sent = 0u64;
        loop {
            let want = u64::from(cfg.chunk).min(limit - sent) as u32;
            let rows = match self.fetch(want).await {
                Ok(rows) => rows,
                Err(e) => {
                    self.abandon_cursor().await;
                    return Outcome::failed(e, None);
                }
            };
            let got = rows.len() as u64;
            sent += got;
            let exhausted = got < u64::from(want);
            if !sink.send(QueryEvent::Rows { stmt_index: idx, rows, more: !exhausted }) {
                return Outcome::Gone;
            }
            if exhausted {
                if let Err(e) = self.finish_cursor().await {
                    return Outcome::failed(e, None);
                }
                return Outcome::Done { tag: command_tag(sql), rows_affected: None, rows_sent: sent, truncated: false };
            }
            if sent >= limit {
                return Outcome::Done { tag: command_tag(sql), rows_affected: None, rows_sent: sent, truncated: true };
            }
        }
    }

    async fn fetch(&mut self, n: u32) -> PgResult<TextRows> {
        let messages = self.simple(&format!("FETCH FORWARD {n} FROM pgt_c")).await?;
        let rows = text_rows(&messages);
        self.peak_buffered = self.peak_buffered.max(rows.len());
        Ok(rows)
    }

    /// İmleci kapatır; transaction çalıştırıcınınsa onaylar (hata olursa geri alır).
    async fn finish_cursor(&mut self) -> PgResult<()> {
        let Some(cursor) = self.cursor.take() else { return Ok(()) };
        let sql = if cursor.own_tx { "CLOSE pgt_c; COMMIT" } else { "CLOSE pgt_c" };
        let result = self.batch(sql).await;
        if result.is_err() && cursor.own_tx {
            self.rollback_quietly().await;
        }
        result
    }

    /// Hata sonrası: çalıştırıcının transaction'ı geri alınır; kullanıcının transaction'ı (artık `aborted`)
    /// olduğu gibi bırakılır, imleç onunla birlikte kapanır.
    async fn abandon_cursor(&mut self) {
        if let Some(cursor) = self.cursor.take()
            && cursor.own_tx
        {
            self.rollback_quietly().await;
        }
    }

    async fn rollback_quietly(&mut self) {
        if let Err(e) = self.batch("ROLLBACK").await {
            tracing::debug!(code = ?e.code().map(|c| c.code()), "ROLLBACK başarısız");
        }
    }

    /// İmleçsiz satır okuma: yanıt akış olarak okunur, `limit`'ten sonraki satırlar atılır (`truncated`).
    async fn run_direct(&mut self, idx: u32, sql: &str, cfg: &ExecConfig, sink: &mut dyn EventSink) -> Outcome {
        let opened =
            with_notices(self.conn.client.simple_query_raw(sql), &mut self.conn.notices, &mut self.notices).await;
        let stream = match opened {
            Ok(s) => s,
            Err(e) => return Outcome::failed(e, Some(0)),
        };
        let mut stream = std::pin::pin!(stream);
        let (chunk, limit) = (cfg.chunk as usize, u64::from(cfg.limit));
        let mut buf: TextRows = Vec::new();
        let (mut kept, mut truncated, mut count) = (0u64, false, None);
        while let Some(message) = with_notices(stream.next(), &mut self.conn.notices, &mut self.notices).await {
            match message {
                Ok(SimpleQueryMessage::Row(row)) if kept < limit => {
                    buf.push(row_values(&row));
                    kept += 1;
                    self.peak_buffered = self.peak_buffered.max(buf.len());
                    if buf.len() == chunk
                        && !sink.send(QueryEvent::Rows { stmt_index: idx, rows: std::mem::take(&mut buf), more: true })
                    {
                        return Outcome::Gone;
                    }
                }
                Ok(SimpleQueryMessage::Row(_)) => truncated = true,
                Ok(SimpleQueryMessage::CommandComplete(n)) => count = Some(n),
                Ok(_) => {}
                Err(e) => return Outcome::failed(e, Some(0)),
            }
        }
        if !sink.send(QueryEvent::Rows { stmt_index: idx, rows: buf, more: false }) {
            return Outcome::Gone;
        }
        let tag = command_tag(sql);
        let rows_affected = affected(&tag, count);
        Outcome::Done { tag, rows_affected, rows_sent: kept, truncated }
    }

    async fn run_without_rows(&mut self, sql: &str) -> Outcome {
        let opened =
            with_notices(self.conn.client.simple_query_raw(sql), &mut self.conn.notices, &mut self.notices).await;
        let stream = match opened {
            Ok(s) => s,
            Err(e) => return Outcome::failed(e, Some(0)),
        };
        let mut stream = std::pin::pin!(stream);
        let mut count = None;
        while let Some(message) = with_notices(stream.next(), &mut self.conn.notices, &mut self.notices).await {
            match message {
                Ok(SimpleQueryMessage::CommandComplete(n)) => count = Some(n),
                Ok(_) => {}
                Err(e) => return Outcome::failed(e, Some(0)),
            }
        }
        let tag = command_tag(sql);
        let rows_affected = affected(&tag, count);
        Outcome::Done { tag, rows_affected, rows_sent: 0, truncated: false }
    }

    fn take_notice(&mut self, code: &str) -> bool {
        self.notices.absorb(&mut self.conn.notices);
        let before = self.notices.kept.len();
        self.notices.kept.retain(|n| n.code != code);
        self.notices.kept.len() != before
    }

    /// Kullanıcının transaction komutlarından oturumun transaction durumunu izler (yalnızca sunucuya ulaşan
    /// ifadeler için çağrılır). Başarısız COMMIT de transaction'ı bitirir (geri alınır).
    fn track_tx(&mut self, sql: &str, ok: bool) {
        match tx_op(sql) {
            Some(TxOp::Begin) if ok => self.in_tx = true,
            Some(TxOp::Commit | TxOp::Rollback) => self.in_tx = ok && chained(sql),
            Some(TxOp::Other) if ok && first_word(sql).as_deref() == Some("prepare") => self.in_tx = false,
            _ => {}
        }
    }
}

fn text_rows(messages: &[SimpleQueryMessage]) -> TextRows {
    messages
        .iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some(row_values(r)),
            _ => None,
        })
        .collect()
}

fn row_values(row: &tokio_postgres::SimpleQueryRow) -> Vec<Option<String>> {
    (0..row.len()).map(|i| row.get(i).map(str::to_string)).collect()
}

/// Sözcük belirteçleri (yorumlar hariç), küçük harfle ve parantez derinliğiyle.
fn words(sql: &str) -> Vec<(String, u32)> {
    lex(sql)
        .into_iter()
        .filter(|t| t.kind == Kind::Word)
        .map(|t| (t.text(sql).to_ascii_lowercase(), t.depth))
        .collect()
}

fn first_word(sql: &str) -> Option<String> {
    words(sql).into_iter().next().map(|(w, _)| w)
}

/// `COMMIT AND CHAIN` / `ROLLBACK AND CHAIN` transaction'ı sürdürür.
fn chained(sql: &str) -> bool {
    let w = words(sql);
    let n = w.len();
    n >= 2 && w[n - 2].0 == "and" && w[n - 1].0 == "chain"
}

/// İstemci protokolünde veri akışı isteyen COPY biçimleri desteklenmez (oturum veri bekleyerek kilitlenirdi).
fn unsupported(sql: &str) -> Option<Error> {
    let w = words(sql);
    (w.first().is_some_and(|(f, _)| f == "copy") && w.iter().any(|(x, _)| x == "stdin" || x == "stdout"))
        .then(|| Error::Invalid("COPY … FROM STDIN / TO STDOUT Sorgu Aracı'nda desteklenmez".into()))
}

/// İmleçle okunabilecek ifade: SELECT/VALUES/TABLE ya da veri değiştirmeyen WITH. Diğer satır döndürenler
/// (INSERT … RETURNING, EXPLAIN, SHOW, FETCH, EXECUTE…) `DECLARE` ile açılamaz, doğrudan yoldan okunur.
pub(crate) fn cursor_capable(sql: &str, kind: StmtKind) -> bool {
    kind == StmtKind::Select && matches!(first_word(sql).as_deref(), Some("select" | "values" | "table" | "with"))
}

/// Sınıflandırıcı sözcüklerinden türetilen komut etiketi (tokio-postgres `simple_query` etiketi vermez).
fn command_tag(sql: &str) -> String {
    let w = words(sql);
    let Some((lead, base)) = w.first().cloned() else { return String::new() };
    let at = |i: usize| w.get(i).map(|(x, _)| x.as_str()).unwrap_or("");
    let upper = |parts: &[&str]| parts.iter().map(|p| p.to_ascii_uppercase()).collect::<Vec<_>>().join(" ");
    match lead.as_str() {
        "select" | "values" | "table" => "SELECT".into(),
        "with" => {
            // CTE gövdeleri parantez içindedir: ana ifade, WITH ile aynı derinlikteki ilk eylem sözcüğüdür.
            let main = w.iter().skip(1).find(|(x, d)| {
                *d == base
                    && matches!(x.as_str(), "select" | "insert" | "update" | "delete" | "merge" | "values" | "table")
            });
            match main.map(|(x, _)| x.as_str()) {
                Some("values" | "table") | None => "SELECT".into(),
                Some(x) => x.to_ascii_uppercase(),
            }
        }
        "create" | "alter" | "drop" => {
            const MODIFIERS: [&str; 11] = [
                "or", "replace", "temp", "temporary", "unique", "unlogged", "global", "local", "trusted", "procedural",
                "recursive",
            ];
            let mut i = 1;
            while MODIFIERS.contains(&at(i)) {
                i += 1;
            }
            let object: Vec<&str> = match at(i) {
                "foreign" if at(i + 1) == "data" => vec!["foreign", "data", "wrapper"],
                "text" => vec![at(i), at(i + 1), at(i + 2)],
                "materialized" | "foreign" | "event" | "access" | "operator" | "user" | "default" | "constraint" => {
                    vec![at(i), at(i + 1)]
                }
                x => vec![x],
            };
            let mut parts = vec![lead.as_str()];
            parts.extend(object.into_iter().filter(|p| !p.is_empty()));
            upper(&parts)
        }
        "start" => "START TRANSACTION".into(),
        "end" => "COMMIT".into(),
        "abort" => "ROLLBACK".into(),
        "refresh" => "REFRESH MATERIALIZED VIEW".into(),
        "lock" => "LOCK TABLE".into(),
        "declare" => "DECLARE CURSOR".into(),
        "security" => "SECURITY LABEL".into(),
        "import" => "IMPORT FOREIGN SCHEMA".into(),
        "prepare" if at(1) == "transaction" => "PREPARE TRANSACTION".into(),
        "commit" | "rollback" if at(1) == "prepared" => upper(&[lead.as_str(), "prepared"]),
        _ => lead.to_ascii_uppercase(),
    }
}

/// `CommandComplete` sayısı yalnızca sayı taşıyan etiketlerde anlamlıdır (diğerlerinde 0 gelir).
fn affected(tag: &str, count: Option<u64>) -> Option<u64> {
    let counted = matches!(
        tag.split(' ').next().unwrap_or(""),
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "SELECT" | "COPY" | "FETCH" | "MOVE"
    );
    count.filter(|&n| counted || n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_tags_come_from_leading_keywords() {
        for (sql, tag) in [
            ("select 1", "SELECT"),
            ("(select 1) union (select 2)", "SELECT"),
            ("insert into t values (1)", "INSERT"),
            ("with a as (select 1) update t set x = 1", "UPDATE"),
            ("with a as (delete from t returning *) select * from a", "SELECT"),
            ("create temp table t(i int)", "CREATE TABLE"),
            ("create or replace function f() returns int language sql as 'select 1'", "CREATE FUNCTION"),
            ("create unique index i on t(i)", "CREATE INDEX"),
            ("drop materialized view v", "DROP MATERIALIZED VIEW"),
            ("create foreign data wrapper w", "CREATE FOREIGN DATA WRAPPER"),
            ("alter text search configuration c rename to d", "ALTER TEXT SEARCH CONFIGURATION"),
            ("/* c */ do $$ begin end $$", "DO"),
            ("begin", "BEGIN"),
            ("start transaction", "START TRANSACTION"),
            ("end", "COMMIT"),
            ("set statement_timeout = 1", "SET"),
            ("vacuum t", "VACUUM"),
        ] {
            assert_eq!(command_tag(sql), tag, "{sql}");
        }
    }

    #[test]
    fn affected_rows_only_for_counting_tags() {
        assert_eq!(affected("INSERT", Some(2)), Some(2));
        assert_eq!(affected("UPDATE", Some(0)), Some(0));
        assert_eq!(affected("CREATE TABLE", Some(0)), None);
        assert_eq!(affected("CREATE TABLE", Some(5)), Some(5), "CREATE TABLE AS sayı taşır");
        assert_eq!(affected("DELETE", None), None);
    }

    #[test]
    fn cursor_capability_and_chain_detection() {
        assert!(cursor_capable("select 1", StmtKind::Select));
        assert!(cursor_capable("(values (1))", StmtKind::Select));
        assert!(cursor_capable("with a as (select 1) select * from a", StmtKind::Select));
        assert!(!cursor_capable("explain select 1", StmtKind::Select));
        assert!(!cursor_capable("show all", StmtKind::Select));
        assert!(!cursor_capable("insert into t values (1) returning *", StmtKind::Write));
        assert!(!cursor_capable("with a as (delete from t returning *) select * from a", StmtKind::Write));
        assert!(chained("commit and chain"));
        assert!(!chained("commit and no chain"));
        assert!(!chained("commit"));
    }

    #[test]
    fn copy_stdio_is_rejected_before_sending() {
        assert!(unsupported("copy t from stdin").is_some());
        assert!(unsupported("COPY (select 1) TO STDOUT").is_some());
        assert!(unsupported("copy t to '/tmp/x'").is_none());
        assert!(unsupported("select 'stdin'").is_none());
    }

    #[test]
    fn config_is_validated_and_capped() {
        let bad = ExecConfig { limit: 0, chunk: 10, autocommit: true };
        assert!(matches!(bad.checked(), Err(Error::Invalid(_))));
        assert!(matches!(ExecConfig { limit: 10, chunk: 0, autocommit: true }.checked(), Err(Error::Invalid(_))));
        let big = ExecConfig { limit: u32::MAX, chunk: DEFAULT_CHUNK, autocommit: false }.checked().unwrap();
        assert_eq!((big.limit, big.chunk, big.autocommit), (MAX_LIMIT, DEFAULT_CHUNK, false));
        // Parça boyu 1000 ile sınırlıdır (tek FETCH / tek `Rows` en çok 1000 satır); ilk yüklemede ayrıca
        // `limit - gönderilen` ile kısılır, `more()` tam parça kullanır.
        let huge = ExecConfig { limit: MAX_LIMIT, chunk: MAX_LIMIT, autocommit: true }.checked().unwrap();
        assert_eq!((huge.limit, huge.chunk), (MAX_LIMIT, DEFAULT_CHUNK));
        let small = ExecConfig { limit: 5, chunk: 1000, autocommit: true }.checked().unwrap();
        assert_eq!((small.limit, small.chunk), (5, 1000));
    }

    #[test]
    fn notice_buffer_caps_and_counts_drops() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let notice = |i: usize| Notice {
            severity: "NOTICE".into(),
            code: "00000".into(),
            message: i.to_string(),
            detail: None,
            hint: None,
        };
        for i in 0..250 {
            tx.send(notice(i)).unwrap();
        }
        let mut buf = NoticeBuf::default();
        buf.absorb(&mut rx);
        assert_eq!((buf.kept.len(), buf.dropped, buf.peak_backlog), (NOTICES_PER_STMT_MAX, 150, 250));
        assert_eq!(buf.kept[0].message, "0", "ilk iletiler tutulur");
        buf.reset();
        assert_eq!((buf.kept.len(), buf.dropped), (0, 0));
    }
}

/// Crate içi (`pub(crate)`) oturum yardımcılarını gerçek sunucuda sınar (`PGT_TEST_URLS` yoksa atlanır).
#[cfg(test)]
mod db_tests {
    use pgt_core::connection::{ConnectionManager, SessionProfile, Target};
    use pgt_core::testing::{TestServer, admin_client, test_servers};

    use super::*;

    #[derive(Default)]
    struct Collect(Vec<QueryEvent>);

    impl EventSink for Collect {
        fn send(&mut self, ev: QueryEvent) -> bool {
            self.0.push(ev);
            true
        }
    }

    async fn executor(ts: &TestServer) -> (ConnectionManager, Executor) {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let profile = SessionProfile::Query { read_only: false, statement_timeout_ms: 0 };
        let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, profile).await.unwrap();
        (mgr, Executor::new(conn))
    }

    async fn run(exec: &mut Executor, sql: &str, cfg: &ExecConfig) -> Collect {
        let mut sink = Collect::default();
        let outcome = exec.run_statement(0, sql, 0, cfg, &mut sink).await;
        assert_eq!(outcome, StmtOutcome::Completed, "{:?}", sink.0);
        sink
    }

    async fn state(ts: &TestServer, pid: i32) -> String {
        let admin = admin_client(ts, &ts.config.maintenance_db).await;
        admin.query_one("SELECT state FROM pg_stat_activity WHERE pid = $1", &[&pid]).await.unwrap().get(0)
    }

    async fn temp_table_exists(exec: &mut Executor, name: &str) -> bool {
        let sql = format!("select count(*) from pg_class where relname = '{name}' and relpersistence = 't'");
        exec.exec_internal(&sql).await.unwrap()[0][0].as_deref() == Some("1")
    }

    /// Manuel kip, izleme "transaction yok" sanıyor ama oturum transaction'da (izlenmeyen yoldan BEGIN):
    /// düz DECLARE başarılı olur, imleç yolu kullanılır, bellek sınırlı kalır ve izleme düzelir.
    #[tokio::test]
    async fn manual_mode_recovers_from_untracked_begin() {
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            let pid: i32 = exec.exec_internal("select pg_backend_pid()").await.unwrap()[0][0].as_ref().unwrap().parse().unwrap();
            exec.exec_internal("begin").await.unwrap();
            exec.exec_internal("create temp table untracked(i int)").await.unwrap();
            assert!(!exec.in_tx(), "exec_internal izlemeyi değiştirmez");
            let manual = ExecConfig { limit: 2000, chunk: 500, autocommit: false };
            let sink = run(&mut exec, "select * from generate_series(1,100000)", &manual).await;
            assert_eq!(exec.last_path(), Some(Path::Cursor));
            assert!(exec.in_tx(), "izleme düzeldi");
            assert!(exec.peak_buffered_rows() <= 500, "{}", exec.peak_buffered_rows());
            let rows: usize = sink.0.iter().map(|e| if let QueryEvent::Rows { rows, .. } = e { rows.len() } else { 0 }).sum();
            assert_eq!(rows, 2000);
            run(&mut exec, "select 1", &manual).await;
            assert_eq!(state(&ts, pid).await, "idle in transaction", "çalıştırıcı kullanıcının transaction'ını bitirmez");
            run(&mut exec, "rollback", &manual).await;
            assert!(!exec.in_tx());
            assert!(!temp_table_exists(&mut exec, "untracked").await);
        }
    }

    /// Ters yön: izleme "transaction var" sanıyor ama yok. Manuel kipte DECLARE 25P01 → doğrudan yol (durum
    /// değişmez, BEGIN eklenmez); autocommit'te kendi transaction'ıyla imleç.
    #[tokio::test]
    async fn stale_in_tx_is_corrected_by_25p01() {
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            let pid: i32 = exec.exec_internal("select pg_backend_pid()").await.unwrap()[0][0].as_ref().unwrap().parse().unwrap();
            exec.set_in_tx(true);
            let manual = ExecConfig { limit: 10, chunk: 10, autocommit: false };
            let sink = run(&mut exec, "select * from generate_series(1,50)", &manual).await;
            assert_eq!(exec.last_path(), Some(Path::Direct));
            assert!(!exec.in_tx());
            assert!(matches!(sink.0.last(), Some(QueryEvent::Done { truncated: true, rows_sent: 10, .. })));
            assert_eq!(state(&ts, pid).await, "idle");

            exec.set_in_tx(true);
            let auto = ExecConfig { limit: 10, chunk: 10, autocommit: true };
            run(&mut exec, "select * from generate_series(1,50)", &auto).await;
            assert_eq!(exec.last_path(), Some(Path::Cursor));
            assert!(!exec.in_tx());
            exec.close_cursor().await.unwrap();
            assert_eq!(state(&ts, pid).await, "idle");
        }
    }

    /// Autocommit, izleme "transaction yok" sanıyor ama kullanıcı transaction'ı açık: BEGIN'in 25001 uyarısı
    /// fark edilir, imleç kapanışı COMMIT etmez, uyarı kullanıcıya NOTICE olarak gösterilmez.
    #[tokio::test]
    async fn autocommit_detects_untracked_transaction_via_25001() {
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            exec.exec_internal("begin").await.unwrap();
            exec.exec_internal("create temp table hidden_tx(i int)").await.unwrap();
            let auto = ExecConfig { limit: 5, chunk: 5, autocommit: true };
            let sink = run(&mut exec, "select * from generate_series(1,50)", &auto).await;
            assert_eq!(exec.last_path(), Some(Path::Cursor));
            assert!(exec.in_tx());
            let Some(QueryEvent::Done { notices, .. }) = sink.0.last() else { panic!("{:?}", sink.0) };
            assert!(notices.is_empty(), "{notices:?}");
            exec.close_cursor().await.unwrap();
            run(&mut exec, "rollback", &auto).await;
            assert!(!temp_table_exists(&mut exec, "hidden_tx").await, "COMMIT edilmemeli");
        }
    }

    #[tokio::test]
    async fn exec_internal_returns_text_rows_without_events() {
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            let rows = exec.exec_internal("select statement_timestamp() <> transaction_timestamp(), null::text").await.unwrap();
            assert_eq!(rows, vec![vec![Some("f".to_string()), None]]);
            let err = exec.exec_internal("selectt").await.unwrap_err();
            assert!(matches!(err, Error::Query { .. }), "{err:?}");
        }
    }
}
