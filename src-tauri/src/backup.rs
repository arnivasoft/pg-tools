//! Yedekleme / geri yükleme Tauri komutları (`backup_*`, `restore_*`; spec 2026-10-18 §1, Plan 26 Task 4).
//!
//! - **Bağlantı:** `Conn` sunucu kaydından üretilir (`Conn::from_server`: SSL eşlemesi uygulamanınkiyle aynı
//!   anlamdadır). Veritabanı adı `databaseOid`'den bakım bağlantısıyla çözülür; `CreateFromArchive`'da hedef
//!   oturumun bakım veritabanıdır. Sunucu bağlı değilse `not_connected`.
//! - **Parola:** `Credentials::password`; yoksa ve oturum parolayla açılmışsa `password_required` (ön yüz yeniden
//!   bağlanır). Oturum parolasız açıldıysa (trust vb.) araç da parolasız çalışır. Parola yalnız `Secret` olarak
//!   yürütücüye geçer; günlüğe, olaya ya da hataya yazılmaz.
//! - **Kapılar:** `backup_start` yalnız plan doğrulaması. `restore_start` sırası: kilitli → plan doğrulaması →
//!   risk ≥ Destructive ise `confirmed` → DataLoss ve üretim sunucusu ise `confirmed_production`.
//! - **Hata kodları:** `locked`, `invalid_request` (plan alan hataları; `detail` alan yolu), `confirmation_required`,
//!   `production_confirmation_required`, `invalid_source` (kaynak okunamadı/uygunsuz), `busy` (aynı hedefe yazan iş
//!   var), `password_required`, `not_connected`, `not_found`, `unknown_server`.
//! - **Günlük:** bu katman `tracing` yazmaz; yürütücü yalnız iş türü, sonuç, çıkış kodu ve süreyi yazar.
//!
//! Gelecekler `Box::pin` ile kutulanır (bkz. ddl.rs: ana iş parçacığı yığını).

use pgt_backup::{
    ArchiveInfo, BackupEvent, BackupRequest, BackupScope, Conn, EventSink, Invocation, JobKind, JobPlan, JobSummary,
    RegisterError, RestoreRequest, RestoreSource, RestoreTarget, Secret, SourceFormat, ToolDetection, ToolKind, ToolSet,
    backup_invocation, pattern_regex, plain_backup_note, restore_invocation, windows_name_problems,
};
use pgt_core::config::ServerConfig;
use pgt_core::connection::Target;
use pgt_core::{Error, ErrorPayload};
use pgt_ddl::{FieldError, Risk};
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

pub const LOCKED: &str = "Sunucu salt okunur kilitli; geri yükleme yapılamaz.";
pub const CONFIRMATION_REQUIRED: &str = "Geri yükleme veritabanını değiştirir ve onay gerektiriyor.";
pub const PRODUCTION_CONFIRMATION_REQUIRED: &str = "Üretim sunucusunda veri kaybı olan geri yükleme ek onay gerektiriyor.";

fn payload(code: &str, message: impl Into<String>, detail: Option<String>) -> ErrorPayload {
    ErrorPayload { code: code.to_string(), message: message.into(), detail, hint: None }
}

/// Kapı reddi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    Locked,
    InvalidRequest,
    ConfirmationRequired,
    ProductionConfirmationRequired,
}

/// `restore_start` kapıları (saf). Sıra: kilitli → plan doğrulaması → onay → üretim onayı.
pub fn restore_gate(
    locked: bool,
    plan_ok: bool,
    risk: Risk,
    production: bool,
    confirmed: bool,
    confirmed_production: bool,
) -> Result<(), GateError> {
    if locked {
        return Err(GateError::Locked);
    }
    if !plan_ok {
        return Err(GateError::InvalidRequest);
    }
    if risk >= Risk::Destructive && !confirmed {
        return Err(GateError::ConfirmationRequired);
    }
    if risk == Risk::DataLoss && production && !confirmed_production {
        return Err(GateError::ProductionConfirmationRequired);
    }
    Ok(())
}

fn gate_payload(g: GateError) -> ErrorPayload {
    match g {
        GateError::Locked => payload("locked", LOCKED, None),
        GateError::InvalidRequest => payload("invalid_request", "İstek geçersiz.", None),
        GateError::ConfirmationRequired => payload("confirmation_required", CONFIRMATION_REQUIRED, None),
        GateError::ProductionConfirmationRequired => {
            payload("production_confirmation_required", PRODUCTION_CONFIRMATION_REQUIRED, None)
        }
    }
}

/// Plan alan hataları → `invalid_request` (ilk hatanın iletisi, `detail` alan yolu).
fn errors_payload(errors: &[FieldError]) -> ErrorPayload {
    match errors.first() {
        Some(e) => payload("invalid_request", e.message.clone(), Some(e.path.clone())),
        None => gate_payload(GateError::InvalidRequest),
    }
}

/// Parola kararı (saf). Kayıtlı/oturum parolası varsa o; yoksa oturum parolayla açıldıysa `PasswordRequired`
/// (çocuk süreç "no password supplied" ile düşmesin, ön yüz yeniden bağlansın); oturum parolasız açıldıysa `None`.
/// `session_had_password` `None` ise sunucu bağlı değildir.
pub fn password_decision(stored: Option<String>, session_had_password: Option<bool>) -> pgt_core::Result<Option<String>> {
    match (stored, session_had_password) {
        (_, None) => Err(Error::NotConnected),
        (Some(p), _) => Ok(Some(p)),
        (None, Some(true)) => Err(Error::PasswordRequired),
        (None, Some(false)) => Ok(None),
    }
}

async fn password(state: &AppState, server_id: Uuid) -> CmdResult<Option<Secret>> {
    let stored = state.credentials.password(server_id)?;
    let had = state.manager.session_has_password(server_id).await;
    Ok(password_decision(stored, had)?.map(Secret::new))
}

/// Kapsamdaki tablo sayısı sorgusu (ilerleme toplamı). Ad değerleri yalnız parametredir. Yalnız `relkind = 'r'`
/// sayılır: pg_dump "dumping contents of table" satırını veri taşıyan tablolar (bölümler dahil) için yazar;
/// bölümlenmiş üst tablo (`p`) ve MV (`m`, verisi REFRESH ile gelir) için yazmaz (Docker testiyle ölçüldü).
/// Uzantı üyesi tablolar (`pg_depend.deptype = 'e'`) da sayılmaz: pg_dump onları `CREATE EXTENSION` ile yazar,
/// verisini yalnız yapılandırma tablosuysa (`pg_extension.extconfig`, NULL olabilir) döker; bu tablolar sayılır (şema/tablo
/// kapsamında uzantı dökülmezse toplam biraz fazla kalabilir: ön yüz `done`'u `total` ile sınırlar).
macro_rules! count_base {
    () => {
        concat!(
            "SELECT count(*)::int4 FROM pg_catalog.pg_class c ",
            "JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace ",
            "WHERE c.relkind = 'r' AND n.nspname <> 'information_schema' AND n.nspname !~ '^pg_' ",
            "AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d ",
            "JOIN pg_catalog.pg_extension e ON e.oid = d.refobjid ",
            "WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = c.oid ",
            "AND d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype = 'e' ",
            "AND NOT COALESCE(c.oid = ANY (e.extconfig), false))"
        )
    };
}
pub const COUNT_DATABASE: &str = count_base!();
pub const COUNT_SCHEMA: &str = concat!(count_base!(), " AND n.nspname = $1");
pub const COUNT_TABLE: &str = concat!(count_base!(), " AND n.nspname = $1 AND c.relname = $2");

/// Kapsama göre sorgu ve parametreleri (saf).
pub fn scope_count_query(scope: &BackupScope) -> (&'static str, Vec<String>) {
    match scope {
        BackupScope::Database => (COUNT_DATABASE, vec![]),
        BackupScope::Schema { name } => (COUNT_SCHEMA, vec![name.clone()]),
        BackupScope::Table { schema, name } => (COUNT_TABLE, vec![schema.clone(), name.clone()]),
    }
}

/// Yedekleme toplamı: hata olursa ya da sayı 0 ise (ya da yalnız şema) `None` (belirsiz ilerleme).
async fn backup_total(state: &AppState, server_id: Uuid, database_oid: u32, req: &BackupRequest) -> Option<u32> {
    if req.options.schema_only {
        return None;
    }
    let (sql, params) = scope_count_query(&req.scope);
    let n = Box::pin(count_in(state, server_id, database_oid, sql, params)).await.ok()?;
    u32::try_from(n).ok().filter(|n| *n > 0)
}

/// ASCII dışı adlı kapsam joker desenle aktarılır (`dump_pattern`); desenin uyduğu nesne sayısı sorgusu (saf).
/// Ad değerleri yalnız parametredir. `None`: ad ASCII'dir, denetim gerekmez.
pub fn pattern_match_query(scope: &BackupScope) -> Option<(&'static str, Vec<String>)> {
    match scope {
        BackupScope::Database => None,
        BackupScope::Schema { name } if !name.is_ascii() => {
            Some(("SELECT count(*)::int4 FROM pg_catalog.pg_namespace WHERE nspname ~ $1", vec![pattern_regex(name)]))
        }
        BackupScope::Table { schema, name } if !schema.is_ascii() || !name.is_ascii() => Some((
            // pg_dump `-t` ile aynı tür listesi.
            "SELECT count(*)::int4 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'S', 'v', 'm', 'f', 'p') AND n.nspname ~ $1 AND c.relname ~ $2",
            vec![pattern_regex(schema), pattern_regex(name)],
        )),
        _ => None,
    }
}

const AMBIGUOUS_PATTERN: &str = "Ad ASCII dışı karakter içeriyor: araç adı joker desenle aktarır ve desen başka nesnelere de \
     uyuyor. Veritabanının tamamını yedekleyin ya da nesneyi yeniden adlandırın.";
const PATTERN_CHECK_FAILED: &str = "Ad ASCII dışı karakter içeriyor ve joker desenin başka nesnelere uyup uymadığı \
     denetlenemedi. Veritabanının tamamını yedekleyin ya da yeniden deneyin.";

/// Joker desen denetiminin sonucu (saf): sorgu hatası da plan hatasıdır (kapalı kalınır). Denetim plan ve başlatma
/// anında yapılır; arada ya da iş sürerken desene uyan yeni nesne oluşturulursa (TOCTOU) o da yedeğe girebilir.
pub fn pattern_check(count: pgt_core::Result<i32>) -> Option<FieldError> {
    match count {
        Ok(n) if n <= 1 => None,
        Ok(_) => Some(FieldError { path: "scope.name".into(), message: AMBIGUOUS_PATTERN.into() }),
        Err(_) => Some(FieldError { path: "scope.name".into(), message: PATTERN_CHECK_FAILED.into() }),
    }
}

/// Sorguyu kapsam veritabanında çalıştırır; tek `int4` döner.
async fn count_in(state: &AppState, server_id: Uuid, database_oid: u32, sql: &'static str, params: Vec<String>) -> pgt_core::Result<i32> {
    state
        .manager
        .run(server_id, Target::Database(database_oid), |c| {
            let params = params.clone();
            async move {
                let row = match params.as_slice() {
                    [] => c.client.query_one(sql, &[]).await?,
                    [a] => c.client.query_one(sql, &[a]).await?,
                    [a, b, ..] => c.client.query_one(sql, &[a, b]).await?,
                };
                Ok(row.get::<_, i32>(0))
            }
        })
        .await
}

fn server_config(state: &AppState, server_id: Uuid) -> CmdResult<ServerConfig> {
    Ok(state.registry.read().unwrap().get(server_id).cloned().ok_or(Error::UnknownServer(server_id))?)
}

fn server_version(state: &AppState, server_id: Uuid) -> CmdResult<u32> {
    Ok(state.manager.info(server_id).ok_or(Error::NotConnected)?.version_num)
}

async fn database_name(state: &AppState, server_id: Uuid, oid: u32) -> CmdResult<String> {
    let name: Option<String> = state
        .manager
        .run(server_id, Target::Maintenance, |c| async move {
            let row = c.client.query_opt("SELECT datname::text FROM pg_catalog.pg_database WHERE oid = $1", &[&oid]).await?;
            Ok(row.map(|r| r.get::<_, String>(0)))
        })
        .await?;
    Ok(name.ok_or_else(|| Error::NotFound("veritabanı".into()))?)
}

/// Bulunan araç kümeleri; önbellekte yoksa ya da ayar değiştiyse yeniden bulunur. `refresh` her zaman tarar.
async fn tool_sets(state: &AppState, refresh: bool) -> ToolDetection {
    let setting = state.store.settings.lock().unwrap().pg_bin_dir.clone();
    let mut cache = state.backup_tools.lock().await;
    if !refresh
        && let Some((s, d)) = cache.as_ref()
        && *s == setting
    {
        return d.clone();
    }
    let d = Box::pin(pgt_backup::tools::detect(setting.as_deref())).await;
    *cache = Some((setting, d.clone()));
    d
}

fn plan_of(inv: &Invocation) -> JobPlan {
    JobPlan { preview: inv.preview.clone(), risk: inv.risk, notes: inv.notes.clone(), errors: Vec::new() }
}

fn failed_plan(errors: Vec<FieldError>, risk: Risk) -> JobPlan {
    JobPlan { preview: String::new(), risk, notes: Vec::new(), errors }
}

fn tool_error(message: String) -> Vec<FieldError> {
    vec![FieldError { path: "tool".into(), message }]
}

/// Yedekleme çağrısı ya da alan hataları. Dış hata (bağlı değil, veritabanı yok) `Err`.
async fn prepare_backup(
    state: &AppState,
    server_id: Uuid,
    database_oid: u32,
    req: &BackupRequest,
) -> CmdResult<Result<Invocation, Vec<FieldError>>> {
    let config = server_config(state, server_id)?;
    let version = server_version(state, server_id)?;
    let database = Box::pin(database_name(state, server_id, database_oid)).await?;
    let sets = Box::pin(tool_sets(state, false)).await.sets;
    let tool = match pgt_backup::tools::choose(&sets, ToolKind::PgDump, version) {
        Ok(t) => t.clone(),
        Err(m) => return Ok(Err(tool_error(m))),
    };
    let conn = Conn::from_server(&config, &database, tool.version.version_num);
    if cfg!(windows) {
        let problems = windows_name_problems(&conn);
        if !problems.is_empty() {
            return Ok(Err(problems));
        }
    }
    if let Some((sql, params)) = pattern_match_query(&req.scope)
        && let Some(e) = pattern_check(Box::pin(count_in(state, server_id, database_oid, sql, params)).await)
    {
        return Ok(Err(vec![e]));
    }
    let mut inv = match backup_invocation(&tool, &conn, req) {
        Ok(inv) => inv,
        Err(errors) => return Ok(Err(errors)),
    };
    if let Some(note) = plain_backup_note(req.format, tool.version.version_num, version) {
        inv.notes.push(note.to_string());
    }
    if let Some(plan) = inv.output.clone() {
        let checked = tokio::task::spawn_blocking(move || pgt_backup::check_output(&plan)).await;
        match checked {
            Ok(Ok(())) => {}
            Ok(Err(m)) => return Ok(Err(vec![FieldError { path: "outputPath".into(), message: m }])),
            Err(_) => return Ok(Err(vec![FieldError { path: "outputPath".into(), message: "Hedef denetlenemedi.".into() }])),
        }
    }
    Ok(Ok(inv))
}

/// Geri yükleme çağrısı (algılanan formatla; arşivde özetiyle) ya da alan hataları. Arşivde özet bir kez okunur
/// (`pg_restore --list`, en yeni araçla) ve hem araç seçiminde (`choose_restore`: arşivi üreten pg_dump ve sunucu
/// sürümü) hem ilerleme toplamında kullanılır. Hiçbir araç yol ANSI denetiminden geçmeden çalışmaz.
async fn prepare_restore(
    state: &AppState,
    server_id: Uuid,
    database_oid: Option<u32>,
    req: &RestoreRequest,
) -> CmdResult<Result<(Invocation, Option<ArchiveInfo>), Vec<FieldError>>> {
    let config = server_config(state, server_id)?;
    let version = server_version(state, server_id)?;
    let path = req.path.clone();
    let sniffed = tokio::task::spawn_blocking(move || pgt_backup::sniff_path(&path))
        .await
        .unwrap_or_else(|_| Err("Kaynak okunamadı.".into()));
    let format = match sniffed {
        Ok((f, _)) => f,
        Err(message) => return Ok(Err(vec![FieldError { path: "path".into(), message }])),
    };
    let maintenance = state.manager.session_maintenance_db(server_id).await.ok_or(Error::NotConnected)?;
    let database = match (req.target, database_oid) {
        (RestoreTarget::CreateFromArchive, _) => maintenance.clone(),
        (RestoreTarget::Existing, Some(oid)) => Box::pin(database_name(state, server_id, oid)).await?,
        (RestoreTarget::Existing, None) => {
            return Ok(Err(vec![FieldError { path: "target".into(), message: "Hedef veritabanı seçilmedi.".into() }]));
        }
    };
    let sets = Box::pin(tool_sets(state, false)).await.sets;
    let (picked, archive) = if format == SourceFormat::Plain {
        // psql: sürüm önemsiz (düz SQL'in uyumu dosyayı üreten pg_dump'a bağlıdır); en yeni.
        (pgt_backup::tools::choose(&sets, ToolKind::Psql, 0).cloned(), None)
    } else {
        if let Err(m) = pgt_backup::tools::choose(&sets, ToolKind::PgRestore, 0) {
            return Ok(Err(tool_error(m)));
        }
        let info = match Box::pin(pgt_backup::archive_info(&req.path, format, &sets)).await {
            Ok(i) => i,
            Err(message) => return Ok(Err(vec![FieldError { path: "path".into(), message }])),
        };
        let dumped_by = info.archive_version.as_deref().and_then(pgt_backup::tools::archive_version_num);
        (pgt_backup::tools::choose_restore(&sets, version, dumped_by).cloned(), Some(info))
    };
    let tool: ToolSet = match picked {
        Ok(t) => t,
        Err(m) => return Ok(Err(tool_error(m))),
    };
    let conn = Conn::from_server(&config, &database, tool.version.version_num);
    if cfg!(windows) {
        let problems = windows_name_problems(&conn);
        if !problems.is_empty() {
            return Ok(Err(problems));
        }
    }
    // Kullanılmayan araç yuvası denetlenmez; aynı küme iki yuvaya verilir.
    Ok(restore_invocation((&tool, &tool), &conn, &maintenance, req, format).map(|inv| (inv, archive)))
}

/// `Channel<BackupEvent>` alıcısı: gönderim hatası (pencere gitti) işi iptal ettirir.
struct ChannelSink(Channel<BackupEvent>);

impl EventSink for ChannelSink {
    fn send(&mut self, e: BackupEvent) -> bool {
        self.0.send(e).is_ok()
    }
}

fn register_payload(e: RegisterError) -> ErrorPayload {
    match e {
        RegisterError::SameTarget => payload("busy", e.to_string(), None),
        RegisterError::DuplicateId => payload("invalid", e.to_string(), None),
    }
}

/// Araç durumu. Her çağrı yeniden tarar (ayarlardaki "yeniden tara"); bağlıysa `pg_dump` sunucu sürümüne göre seçilir.
#[tauri::command]
pub async fn backup_tools(state: State<'_, AppState>, server_id: Uuid) -> CmdResult<ToolDetection> {
    let detection = Box::pin(tool_sets(&state, true)).await;
    Ok(detection.with_choices(state.manager.info(server_id).map(|i| i.version_num)))
}

#[tauri::command]
pub async fn backup_plan(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    request: BackupRequest,
) -> CmdResult<JobPlan> {
    Ok(match Box::pin(prepare_backup(&state, server_id, database_oid, &request)).await? {
        Ok(inv) => plan_of(&inv),
        Err(errors) => failed_plan(errors, Risk::Safe),
    })
}

#[tauri::command]
pub async fn backup_start(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    request: BackupRequest,
    job_id: Uuid,
    on_event: Channel<BackupEvent>,
) -> CmdResult<JobSummary> {
    let inv = Box::pin(prepare_backup(&state, server_id, database_oid, &request)).await?.map_err(|e| errors_payload(&e))?;
    let password = Box::pin(password(&state, server_id)).await?;
    let total = Box::pin(backup_total(&state, server_id, database_oid, &request)).await;
    let target = inv.output.as_ref().map(|o| o.final_path.clone());
    let (token, _guard) = state.backup.register(job_id, target.as_deref()).map_err(register_payload)?;
    let mut sink = ChannelSink(on_event);
    Ok(Box::pin(pgt_backup::run(inv, password, JobKind::Backup, total, &mut sink, token)).await)
}

/// Kaynağı inceler (format ve arşiv özeti). Sunucu gerekmez.
#[tauri::command]
pub async fn restore_inspect(state: State<'_, AppState>, path: String) -> CmdResult<RestoreSource> {
    let sets = Box::pin(tool_sets(&state, false)).await.sets;
    Box::pin(pgt_backup::inspect(&path, &sets)).await.map_err(|m| payload("invalid_source", m, None))
}

#[tauri::command]
pub async fn restore_plan(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: Option<u32>,
    request: RestoreRequest,
) -> CmdResult<JobPlan> {
    Ok(match Box::pin(prepare_restore(&state, server_id, database_oid, &request)).await? {
        Ok((inv, _)) => plan_of(&inv),
        Err(errors) => failed_plan(errors, Risk::Destructive),
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn restore_start(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: Option<u32>,
    request: RestoreRequest,
    job_id: Uuid,
    confirmed: bool,
    confirmed_production: bool,
    on_event: Channel<BackupEvent>,
) -> CmdResult<JobSummary> {
    let config = server_config(&state, server_id)?;
    // Kilit her şeyden önce: kilitli sunucuda araç bulma ya da bağlantı bile yapılmaz.
    restore_gate(config.read_only_lock, true, Risk::Safe, false, false, false).map_err(gate_payload)?;
    let prepared = Box::pin(prepare_restore(&state, server_id, database_oid, &request)).await?;
    let (inv, archive) = match prepared {
        Ok(p) => p,
        Err(errors) => return Err(errors_payload(&errors)),
    };
    restore_gate(false, true, inv.risk, config.is_production(), confirmed, confirmed_production).map_err(gate_payload)?;
    let password = Box::pin(password(&state, server_id)).await?;
    // Plan sırasında okunan özet yeniden kullanılır (ikinci `pg_restore --list` yok).
    let total = pgt_backup::restore_total(&request, archive.as_ref());
    let (token, _guard) = state.backup.register(job_id, None).map_err(register_payload)?;
    let mut sink = ChannelSink(on_event);
    Ok(Box::pin(pgt_backup::run(inv, password, JobKind::Restore, total, &mut sink, token)).await)
}

#[tauri::command]
pub async fn backup_cancel(state: State<'_, AppState>, job_id: Uuid) -> CmdResult<()> {
    state.backup.cancel(job_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_order_is_locked_plan_confirm_production() {
        use GateError::*;
        // Kilit her şeyi ezer.
        assert_eq!(restore_gate(true, false, Risk::DataLoss, true, false, false), Err(Locked));
        assert_eq!(restore_gate(true, true, Risk::Destructive, false, true, true), Err(Locked));
        // Plan hatası onaydan önce.
        assert_eq!(restore_gate(false, false, Risk::DataLoss, true, false, false), Err(InvalidRequest));
        // Destructive ve DataLoss onay ister.
        assert_eq!(restore_gate(false, true, Risk::Destructive, false, false, false), Err(ConfirmationRequired));
        assert_eq!(restore_gate(false, true, Risk::DataLoss, true, false, true), Err(ConfirmationRequired));
        assert_eq!(restore_gate(false, true, Risk::Destructive, true, true, false), Ok(()));
        // Üretimde DataLoss ek onay ister; üretim dışında istemez.
        assert_eq!(restore_gate(false, true, Risk::DataLoss, true, true, false), Err(ProductionConfirmationRequired));
        assert_eq!(restore_gate(false, true, Risk::DataLoss, false, true, false), Ok(()));
        assert_eq!(restore_gate(false, true, Risk::DataLoss, true, true, true), Ok(()));
        assert_eq!(restore_gate(false, true, Risk::Safe, true, false, false), Ok(()));
        let codes: Vec<String> =
            [Locked, InvalidRequest, ConfirmationRequired, ProductionConfirmationRequired].map(|g| gate_payload(g).code).to_vec();
        assert_eq!(codes, ["locked", "invalid_request", "confirmation_required", "production_confirmation_required"]);
    }

    #[test]
    fn password_required_only_when_session_used_a_password() {
        assert_eq!(password_decision(Some("p".into()), Some(true)).unwrap().as_deref(), Some("p"));
        assert_eq!(password_decision(None, Some(false)).unwrap(), None);
        let err = password_decision(None, Some(true)).unwrap_err();
        assert_eq!(ErrorPayload::from(&err).code, "password_required");
        assert_eq!(ErrorPayload::from(&password_decision(Some("p".into()), None).unwrap_err()).code, "not_connected");
    }

    #[test]
    fn scope_count_sql_is_parameterized() {
        let evil = "x'; DROP TABLE t; --";
        let (sql, params) = scope_count_query(&BackupScope::Table { schema: evil.into(), name: "ad\"".into() });
        assert!(!sql.contains(evil) && !sql.contains("ad\""));
        assert!(sql.contains("$1") && sql.contains("$2"), "{sql}");
        assert_eq!(params, vec![evil.to_string(), "ad\"".to_string()]);
        let (sql, params) = scope_count_query(&BackupScope::Schema { name: evil.into() });
        assert!(sql.contains("$1") && !sql.contains("$2") && !sql.contains(evil));
        assert_eq!(params.len(), 1);
        let (sql, params) = scope_count_query(&BackupScope::Database);
        assert!(!sql.contains('$') && params.is_empty());
        assert!(sql.contains("relkind = 'r'"));
    }

    #[test]
    fn non_ascii_scope_is_checked_with_parameterized_regex() {
        assert!(pattern_match_query(&BackupScope::Database).is_none());
        assert!(pattern_match_query(&BackupScope::Table { schema: "app".into(), name: "a".into() }).is_none());
        let (sql, params) = pattern_match_query(&BackupScope::Table { schema: "app".into(), name: "Büyük Ad".into() }).unwrap();
        assert!(sql.contains("$1") && sql.contains("$2") && !sql.contains("Büyük"));
        assert_eq!(params, vec!["^(app)$".to_string(), "^(B.y.k Ad)$".to_string()]);
        let (sql, params) = pattern_match_query(&BackupScope::Schema { name: "şema".into() }).unwrap();
        assert!(sql.contains("$1") && !sql.contains('ş'));
        assert_eq!(params, vec!["^(.ema)$".to_string()]);
    }

    #[test]
    fn pattern_check_fails_closed() {
        assert_eq!(pattern_check(Ok(0)), None);
        assert_eq!(pattern_check(Ok(1)), None);
        let amb = pattern_check(Ok(2)).unwrap();
        assert_eq!((amb.path.as_str(), amb.message.as_str()), ("scope.name", AMBIGUOUS_PATTERN));
        let failed = pattern_check(Err(Error::NotConnected)).unwrap();
        assert_eq!((failed.path.as_str(), failed.message.as_str()), ("scope.name", PATTERN_CHECK_FAILED));
    }

    #[test]
    fn count_query_skips_extension_members_but_keeps_config_tables() {
        for sql in [COUNT_DATABASE, COUNT_SCHEMA, COUNT_TABLE] {
            assert!(sql.contains("deptype = 'e'") && sql.contains("extconfig"), "{sql}");
        }
    }

    #[test]
    fn plan_errors_map_to_invalid_request_with_field_path() {
        let p = errors_payload(&[FieldError { path: "options.jobs".into(), message: "m".into() }]);
        assert_eq!((p.code.as_str(), p.message.as_str(), p.detail.as_deref()), ("invalid_request", "m", Some("options.jobs")));
        assert_eq!(register_payload(RegisterError::SameTarget).code, "busy");
    }
}
