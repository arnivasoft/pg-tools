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
//!
//! SQL metni asla günlüğe yazılmaz: `tracing` alanlarında yalnızca `kind`, `elapsed_ms`, `rows_sent` ve hata
//! `code`'u bulunur.

use std::time::Instant;

use futures_util::StreamExt;
use pgt_core::connection::{CancelHandle, DedicatedConn, Notice, ServerInfo};
use pgt_core::{Error, ErrorPayload, Result};
use tokio_postgres::SimpleQueryMessage;
use tokio_postgres::error::ErrorPosition;

use crate::classify::{StmtKind, TxOp, analyze, tx_op};
use crate::events::{Column, EventSink, NOTICES_PER_STMT_MAX, NoticeInfo, QueryEvent};
use crate::lex::{Kind, lex};

/// `ExecConfig.limit` üst sınırı; aşan değer buna kısılır.
pub const MAX_LIMIT: u32 = 1_000_000;
/// Önerilen parça boyu.
pub const DEFAULT_CHUNK: u32 = 1000;

const DECLARE_PREFIX: &str = "DECLARE pgt_c NO SCROLL CURSOR FOR ";
const BEGIN_DECLARE_PREFIX: &str = "BEGIN; DECLARE pgt_c NO SCROLL CURSOR FOR ";

type TextRows = Vec<Vec<Option<String>>>;

/// Çalıştırma ayarları. `limit`: ilk yüklemede gönderilecek toplam satır sınırı (0 geçersiz, en çok
/// `MAX_LIMIT`); `chunk`: tek `Rows` olayındaki en çok satır (0 geçersiz); `autocommit`: imleç için gereken
/// transaction'ı çalıştırıcı yönetebilir mi.
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
        let limit = self.limit.min(MAX_LIMIT);
        Ok(ExecConfig { limit, chunk: self.chunk.min(limit), autocommit: self.autocommit })
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
    /// İmlecin transaction'ını çalıştırıcı açtı (kapanışta COMMIT, hatada ROLLBACK).
    own_tx: bool,
    chunk: u32,
}

enum Outcome {
    Done { tag: String, rows_affected: Option<u64>, rows_sent: u64, truncated: bool },
    Failed { error: Error, position: Option<u32> },
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
        Outcome::Failed { error: Error::from_query(e), position }
    }
}

enum CursorOpen {
    Opened,
    /// İmleç kullanılamaz; doğrudan yol kullanılmalı.
    Fallback,
    Failed(Outcome),
}

/// Bir sorgu oturumunun çalıştırıcısı (oturum başına bir tane). Aynı anda en çok bir imleç (`pgt_c`) açıktır.
pub struct Executor {
    conn: DedicatedConn,
    cursor: Option<OpenCursor>,
    /// Oturum bir transaction bloğunda mı (kullanıcının BEGIN/COMMIT/ROLLBACK'lerinden izlenir).
    in_tx: bool,
    /// Bu ifade için toplanan NOTICE'lar (en çok `NOTICES_PER_STMT_MAX`).
    notices: Vec<Notice>,
    last_path: Option<Path>,
    /// Bellekte aynı anda tutulan en çok satır (sınırın tutulduğunu doğrulamak için).
    peak_buffered: usize,
}

impl Executor {
    pub fn new(conn: DedicatedConn) -> Self {
        Executor { conn, cursor: None, in_tx: false, notices: Vec::new(), last_path: None, peak_buffered: 0 }
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
    pub fn in_transaction(&self) -> bool {
        self.in_tx
    }

    /// `more()` ile okunabilecek açık bir imleç var mı.
    pub fn has_open_cursor(&self) -> bool {
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
        self.collect_notices();
        self.notices.clear();
        self.last_path = None;

        let outcome = match cfg.checked() {
            Ok(cfg) => match unsupported(sql) {
                Some(e) => Outcome::Failed { error: e, position: None },
                None => self.execute(idx, sql, char_offset, kind, &cfg, sink).await,
            },
            Err(e) => Outcome::Failed { error: e, position: None },
        };
        self.collect_notices();
        let notices: Vec<NoticeInfo> = std::mem::take(&mut self.notices).into_iter().map(NoticeInfo::from).collect();
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        match outcome {
            Outcome::Done { tag, rows_affected, rows_sent, truncated } => {
                self.track_tx(sql, true);
                tracing::debug!(?kind, elapsed_ms, rows_sent, "ifade tamamlandı");
                let ev =
                    QueryEvent::Done { stmt_index: idx, tag, rows_affected, rows_sent, truncated, elapsed_ms, notices };
                if sink.send(ev) {
                    StmtOutcome::Completed
                } else {
                    self.close_cursor_quietly().await;
                    StmtOutcome::Failed
                }
            }
            Outcome::Failed { error, position } => {
                self.track_tx(sql, false);
                let code = match &error {
                    Error::Query { sqlstate: Some(s), .. } => s.clone(),
                    other => ErrorPayload::from(other).code,
                };
                tracing::debug!(?kind, elapsed_ms, %code, dropped_notices = notices.len(), "ifade başarısız");
                let error = ErrorPayload::from(&error);
                sink.send(QueryEvent::Failed { stmt_index: idx, error, position, char_offset });
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
    /// Alıcı giderse imleç kapatılır. Açık imleç yoksa `Error::Invalid`.
    pub async fn more(&mut self, idx: u32, sink: &mut dyn EventSink) -> Result<()> {
        let Some(chunk) = self.cursor.as_ref().map(|c| c.chunk) else {
            return Err(Error::Invalid("açık imleç yok".into()));
        };
        let result = self.fetch(chunk).await;
        // `more()` sırasında gelen iletiler bir sonraki ifadeye yazılmasın.
        self.collect_notices();
        self.notices.clear();
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

    async fn execute(
        &mut self,
        idx: u32,
        sql: &str,
        char_offset: u32,
        kind: StmtKind,
        cfg: &ExecConfig,
        sink: &mut dyn EventSink,
    ) -> Outcome {
        let columns: Vec<Column> = match self.conn.client.prepare(sql).await {
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
            match self.open_cursor(sql, cfg).await {
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

    /// `pgt_c` imlecini açar. Transaction yoksa: autocommit'te `BEGIN` ile aynı `batch_execute`'ta açılır,
    /// manuel kipte imleç kullanılmaz (doğrudan yol).
    async fn open_cursor(&mut self, sql: &str, cfg: &ExecConfig) -> CursorOpen {
        for attempt in 0..2 {
            let own_tx = !self.in_tx;
            if own_tx && !cfg.autocommit {
                return CursorOpen::Fallback;
            }
            let prefix = if own_tx { BEGIN_DECLARE_PREFIX } else { DECLARE_PREFIX };
            let result = self.conn.client.batch_execute(&format!("{prefix}{sql}")).await;
            // İzleme yanılmış ve oturum zaten bir transaction'daysa BEGIN 25001 uyarısı verir: transaction
            // kullanıcınındır, çalıştırıcı onu onaylamaz ya da geri almaz.
            let was_in_tx = own_tx && self.take_notice("25001");
            if was_in_tx {
                self.in_tx = true;
            }
            let own_tx = own_tx && !was_in_tx;
            let e = match result {
                Ok(()) => {
                    self.cursor = Some(OpenCursor { own_tx, chunk: cfg.chunk });
                    return CursorOpen::Opened;
                }
                Err(e) => e,
            };
            if own_tx {
                self.rollback_quietly().await;
            }
            let code = e.code().map(|c| c.code().to_string()).unwrap_or_default();
            // İzleme yanılmış, transaction yok: bir kez de kendi transaction'ımızla dene.
            if !own_tx && !was_in_tx && code == "25P01" && attempt == 0 {
                self.in_tx = false;
                continue;
            }
            // İmleçle açılamayan sorgu: hata ayrıştırma/planlama aşamasında, ifade hiç çalışmadı.
            if own_tx && matches!(code.as_str(), "42P11" | "42601" | "0A000") {
                return CursorOpen::Fallback;
            }
            return CursorOpen::Failed(Outcome::failed(e, Some(prefix.chars().count() as u32)));
        }
        CursorOpen::Fallback
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

    async fn fetch(&mut self, n: u32) -> std::result::Result<TextRows, tokio_postgres::Error> {
        let messages = self.conn.client.simple_query(&format!("FETCH FORWARD {n} FROM pgt_c")).await?;
        let rows: TextRows = messages
            .iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::Row(r) => Some(row_values(r)),
                _ => None,
            })
            .collect();
        self.peak_buffered = self.peak_buffered.max(rows.len());
        Ok(rows)
    }

    /// İmleci kapatır; transaction çalıştırıcınınsa onaylar (hata olursa geri alır).
    async fn finish_cursor(&mut self) -> std::result::Result<(), tokio_postgres::Error> {
        let Some(cursor) = self.cursor.take() else { return Ok(()) };
        let sql = if cursor.own_tx { "CLOSE pgt_c; COMMIT" } else { "CLOSE pgt_c" };
        let result = self.conn.client.batch_execute(sql).await;
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
        if let Err(e) = self.conn.client.batch_execute("ROLLBACK").await {
            tracing::debug!(code = ?e.code().map(|c| c.code()), "ROLLBACK başarısız");
        }
    }

    /// İmleçsiz satır okuma: yanıt akış olarak okunur, `limit`'ten sonraki satırlar atılır (`truncated`).
    async fn run_direct(&mut self, idx: u32, sql: &str, cfg: &ExecConfig, sink: &mut dyn EventSink) -> Outcome {
        let stream = match self.conn.client.simple_query_raw(sql).await {
            Ok(s) => s,
            Err(e) => return Outcome::failed(e, Some(0)),
        };
        let mut stream = std::pin::pin!(stream);
        let (chunk, limit) = (cfg.chunk as usize, u64::from(cfg.limit));
        let mut buf: TextRows = Vec::new();
        let (mut kept, mut truncated, mut count) = (0u64, false, None);
        while let Some(message) = stream.next().await {
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
        let stream = match self.conn.client.simple_query_raw(sql).await {
            Ok(s) => s,
            Err(e) => return Outcome::failed(e, Some(0)),
        };
        let mut stream = std::pin::pin!(stream);
        let mut count = None;
        while let Some(message) = stream.next().await {
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

    /// Kanalda bekleyen NOTICE'ları ifadenin listesine alır (en çok `NOTICES_PER_STMT_MAX`, fazlası atılır).
    /// Bağlantı sürücüsü NOTICE'ı yanıtın sonundan önce kanala koyar; yanıt geldikten sonra boşaltmak yeterlidir.
    fn collect_notices(&mut self) {
        while let Ok(n) = self.conn.notices.try_recv() {
            if self.notices.len() < NOTICES_PER_STMT_MAX {
                self.notices.push(n);
            }
        }
    }

    fn take_notice(&mut self, code: &str) -> bool {
        self.collect_notices();
        let before = self.notices.len();
        self.notices.retain(|n| n.code != code);
        self.notices.len() != before
    }

    /// Kullanıcının transaction komutlarından oturumun transaction durumunu izler. Başarısız COMMIT de
    /// transaction'ı bitirir (geri alınır).
    fn track_tx(&mut self, sql: &str, ok: bool) {
        match tx_op(sql) {
            Some(TxOp::Begin) if ok => self.in_tx = true,
            Some(TxOp::Commit | TxOp::Rollback) => self.in_tx = ok && chained(sql),
            Some(TxOp::Other) if ok && first_word(sql).as_deref() == Some("prepare") => self.in_tx = false,
            _ => {}
        }
    }
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
fn cursor_capable(sql: &str, kind: StmtKind) -> bool {
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
        let small = ExecConfig { limit: 5, chunk: 1000, autocommit: true }.checked().unwrap();
        assert_eq!(small.chunk, 5);
    }
}
