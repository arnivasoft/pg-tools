//! Tek bir SQL ifadesini belirteç taramasıyla sınıflandırır (ayrıştırıcı değil, sezgisel).
//! SQL metni günlüğe yazılmaz.

use serde::Serialize;
use ts_rs::TS;

use crate::lex::{lex, Kind, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StmtKind {
    Select,
    Write,
    Ddl,
    Tx,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Danger {
    UpdateWithoutWhere,
    DeleteWithoutWhere,
    Truncate,
    Drop,
    DropColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Analysis {
    pub kind: StmtKind,
    pub danger: Option<Danger>,
    /// Parola/bağlantı dizesi içerebilir; geçmişe yazılmaz.
    pub sensitive: bool,
    /// Çalıştırıcıya ipucu; kesin karar `prepare` sütunlarındandır.
    pub returns_rows_hint: bool,
    /// Oturumu salt okunur kipten çıkarabilecek ifade.
    pub changes_read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TxOp {
    Begin,
    Commit,
    Rollback,
    Savepoint,
    Release,
    RollbackTo,
    Other,
}

struct Scan<'a> {
    sql: &'a str,
    /// Yorumsuz belirteçler.
    toks: Vec<Token>,
    /// Baştaki `(` ve yorumlardan sonraki ilk Word'ün `toks` içindeki dizini.
    first: Option<usize>,
}

impl<'a> Scan<'a> {
    fn new(sql: &'a str) -> Self {
        let toks: Vec<Token> = lex(sql).into_iter().filter(|t| t.kind != Kind::Comment).collect();
        let first = toks
            .iter()
            .position(|t| !(t.kind == Kind::Punct && t.text(sql) == "("))
            .filter(|&i| toks[i].kind == Kind::Word);
        Scan { sql, toks, first }
    }

    fn word(&self, i: usize) -> Option<String> {
        let t = self.toks.get(i)?;
        (t.kind == Kind::Word).then(|| t.text(self.sql).to_ascii_lowercase())
    }

    fn lead(&self) -> Option<String> {
        self.first.and_then(|i| self.word(i))
    }

    fn base(&self) -> u32 {
        self.first.map_or(0, |i| self.toks[i].depth)
    }

    fn has(&self, w: &str) -> bool {
        self.toks.iter().any(|t| t.is_word(self.sql, w))
    }

    fn has_any(&self, ws: &[&str]) -> bool {
        ws.iter().any(|w| self.has(w))
    }

    /// Gerçek bir DML eylemi var mı. `FOR [NO KEY] UPDATE` kilit cümleleri, `ON CONFLICT DO UPDATE`
    /// ve MERGE kolları (`THEN UPDATE/DELETE`) eylem sayılmaz. Tür tespiti ve tehlike taraması ortak kullanır.
    fn is_dml_action(&self, i: usize) -> bool {
        let t = &self.toks[i];
        if !DML.iter().any(|w| t.is_word(self.sql, w)) {
            return false;
        }
        let prev = if i > 0 { self.word(i - 1) } else { None };
        !matches!(prev.as_deref(), Some("for" | "key" | "do" | "then"))
    }

    fn has_dml(&self) -> bool {
        (0..self.toks.len()).any(|i| self.is_dml_action(i))
    }

    fn has_at_base(&self, w: &str) -> bool {
        let base = self.base();
        self.toks.iter().any(|t| t.depth == base && t.is_word(self.sql, w))
    }
}

const DML: [&str; 4] = ["insert", "update", "delete", "merge"];

pub fn analyze(stmt_sql: &str) -> Analysis {
    let s = Scan::new(stmt_sql);
    let lead = s.lead().unwrap_or_default();
    let second = s.first.and_then(|i| s.word(i + 1));
    let kind = match lead.as_str() {
        "select" if s.has_at_base("into") => StmtKind::Ddl,
        "select" | "values" | "table" | "show" => StmtKind::Select,
        "with" if s.has_dml() => StmtKind::Write,
        "with" if s.has_at_base("into") => StmtKind::Ddl,
        "with" => StmtKind::Select,
        // EXPLAIN ANALYZE ifadeyi gerçekten çalıştırır (DML, CREATE TABLE AS, SELECT INTO, REFRESH).
        "explain"
            if s.has_any(&["analyze", "analyse"])
                && (s.has_dml() || s.has_any(&["create", "refresh", "into"])) =>
        {
            StmtKind::Write
        }
        "explain" => StmtKind::Select,
        "insert" | "update" | "delete" | "merge" => StmtKind::Write,
        "copy" if s.has_at_base("from") || s.has("program") || s.has_dml() => StmtKind::Write,
        "copy" => StmtKind::Select,
        "create" | "alter" | "drop" | "truncate" | "comment" | "grant" | "revoke" | "reindex" | "cluster"
        | "refresh" | "security" | "import" | "checkpoint" | "analyze" | "vacuum" | "reassign" => StmtKind::Ddl,
        "begin" | "start" | "commit" | "end" | "rollback" | "abort" | "savepoint" | "release" => StmtKind::Tx,
        "prepare" | "set" if second.as_deref() == Some("transaction") => StmtKind::Tx,
        _ => StmtKind::Other,
    };

    let danger = match kind {
        StmtKind::Write if matches!(lead.as_str(), "with" | "insert" | "update" | "delete" | "explain" | "copy") => {
            write_danger(&s)
        }
        StmtKind::Ddl => match lead.as_str() {
            "truncate" => Some(Danger::Truncate),
            "drop" => Some(Danger::Drop),
            "alter" if s.has_at_base("drop") => Some(Danger::DropColumn),
            _ => None,
        },
        _ => None,
    };

    let sensitive = matches!(lead.as_str(), "create" | "alter") && s.has_any(&["password", "connection"]);
    // COPY ... TO satır döndürmez (Select sayılsa da).
    let returns_rows_hint = match kind {
        StmtKind::Select => lead != "copy",
        StmtKind::Write => s.has("returning") || lead == "explain",
        _ => false,
    };

    Analysis { kind, danger, sensitive, returns_rows_hint, changes_read_only: changes_read_only(&s) }
}

/// Kendi parantez kapsamında `WHERE` bulunmayan UPDATE/DELETE var mı (CTE içi dahil).
/// MERGE için tehlike denetlenmez (kabul edilen sınır).
fn write_danger(s: &Scan) -> Option<Danger> {
    for (i, t) in s.toks.iter().enumerate() {
        let (is_up, is_del) = (t.is_word(s.sql, "update"), t.is_word(s.sql, "delete"));
        if !(is_up || is_del) || !s.is_dml_action(i) {
            continue;
        }
        let d = t.depth;
        let has_where = s.toks[i + 1..]
            .iter()
            .take_while(|u| u.depth >= d)
            .any(|u| u.depth == d && u.is_word(s.sql, "where"));
        if !has_where {
            return Some(if is_up { Danger::UpdateWithoutWhere } else { Danger::DeleteWithoutWhere });
        }
    }
    None
}

fn changes_read_only(s: &Scan) -> bool {
    let word_at = |i: usize, w: &str| s.toks.get(i).is_some_and(|t| t.is_word(s.sql, w));
    let mentions = s.toks.iter().any(|t| {
        matches!(t.kind, Kind::Word | Kind::Str | Kind::Ident | Kind::Dollar)
            && t.text(s.sql).to_ascii_lowercase().contains("transaction_read_only")
    });
    if mentions && s.has_any(&["set", "set_config", "reset", "discard"]) {
        return true;
    }
    // DO gövdesi gibi dolar tırnaklı metin içinde SET: gövde Word olarak ayrışmaz, metinsel bakılır.
    let in_dollar = s.toks.iter().any(|t| {
        t.kind == Kind::Dollar && {
            let x = t.text(s.sql).to_ascii_lowercase();
            x.contains("transaction_read_only") && ["set", "discard"].iter().any(|w| x.contains(w))
        }
    });
    if in_dollar {
        return true;
    }
    // DO gövdesi / dize içindeki ifadeler Word olarak ayrışmaz: boşlukları normalleştirip metinsel bakılır.
    let phrase_in_text = s.toks.iter().any(|t| {
        matches!(t.kind, Kind::Dollar | Kind::Str) && {
            let x = t.text(s.sql).to_ascii_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
            ["read write", "reset all", "discard all"].iter().any(|p| x.contains(p))
        }
    });
    if phrase_in_text {
        return true;
    }
    (0..s.toks.len()).any(|i| {
        (word_at(i, "read") && word_at(i + 1, "write"))
            || ((word_at(i, "reset") || word_at(i, "discard")) && word_at(i + 1, "all"))
    })
}

pub fn tx_op(stmt_sql: &str) -> Option<TxOp> {
    let s = Scan::new(stmt_sql);
    let i = s.first?;
    let second = s.word(i + 1);
    let second = second.as_deref();
    Some(match s.word(i)?.as_str() {
        "begin" | "start" => TxOp::Begin,
        "commit" | "end" if second == Some("prepared") => TxOp::Other,
        "commit" | "end" => TxOp::Commit,
        "rollback" | "abort" if second == Some("prepared") => TxOp::Other,
        "rollback" | "abort" if second == Some("to") => TxOp::RollbackTo,
        "rollback" | "abort" => TxOp::Rollback,
        "savepoint" => TxOp::Savepoint,
        "release" => TxOp::Release,
        "prepare" | "set" if second == Some("transaction") => TxOp::Other,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_statement_kinds() {
        use StmtKind::*;
        for (sql, want) in [
            ("select 1", Select), ("  /* c */ (select 1) union (select 2)", Select), ("values (1)", Select), ("table t", Select), ("show all", Select),
            ("select * into t2 from t", Ddl),
            ("select * from t for update", Select),
            ("with a as (select 1) select * from a", Select),
            ("with a as (delete from t returning *) select * from a", Write),
            ("explain select 1", Select), ("explain analyze select 1", Select), ("explain analyze delete from t", Write),
            ("explain (analyze, buffers) delete from t", Write),
            ("insert into t values (1)", Write), ("update t set a=1", Write), ("delete from t", Write), ("merge into t using s on true when matched then delete", Write),
            ("copy t from stdin", Write), ("copy t to stdout", Select), ("copy (select * from t) to stdout", Select),
            ("create table t(i int)", Ddl), ("drop table t", Ddl), ("truncate t", Ddl), ("grant select on t to u", Ddl), ("vacuum t", Ddl),
            ("begin", Tx), ("start transaction", Tx), ("commit", Tx), ("end", Tx), ("rollback to sp", Tx), ("savepoint sp", Tx), ("prepare transaction 'x'", Tx),
            ("commit prepared 'x'", Tx), ("set transaction read only", Tx),
            ("set search_path = x", Other), ("do $$ begin end $$", Other), ("call p()", Other), ("prepare q as select 1", Other),
            ("", Other),
            ("explain analyze select * from t for update", Select),
            ("with a as (select 1) select * from a for update", Select),
            ("with a as (select 1) select * into t2 from a", Ddl),
            ("explain analyze create table t2 as select 1", Write),
            ("explain analyze select * into t2 from t", Write),
            ("explain delete from t", Select),
            ("copy t to program 'x'", Write),
            ("copy (delete from t returning *) to stdout", Write),
            ("copy (select 1) to stdout", Select),
        ] {
            assert_eq!(analyze(sql).kind, want, "{sql}");
        }
    }

    #[test]
    fn detects_dangerous_statements() {
        use Danger::*;
        for (sql, want) in [
            ("update t set a=1", Some(UpdateWithoutWhere)),
            ("update t set a=1 where id=1", None),
            ("update t set a=(select max(b) from u where u.x=t.x)", Some(UpdateWithoutWhere)), // alt sorgudaki WHERE sayılmaz
            ("delete from t", Some(DeleteWithoutWhere)),
            ("delete from t where true", None), // bilinen sınır: hileler yakalanmaz
            ("with d as (delete from t returning *) select * from d", Some(DeleteWithoutWhere)),
            ("with d as (select 1) delete from t using d where t.id = d.x", None),
            ("delete from t -- where id=1", Some(DeleteWithoutWhere)),
            ("delete from t /* where */", Some(DeleteWithoutWhere)),
            ("update t set a='where'", Some(UpdateWithoutWhere)),
            ("update \"where\" set a=1", Some(UpdateWithoutWhere)),
            ("truncate t", Some(Truncate)), ("drop table t", Some(Drop)), ("drop index i", Some(Drop)),
            ("alter table t drop column c", Some(DropColumn)), ("alter table t add column c int", None),
            ("select 1", None), ("insert into t values (1)", None), ("create table t(i int)", None),
            ("insert into t select * from u on conflict (id) do update set a=1", None),
            ("update t set a=(select b from u for update) where id=1", None),
            ("with d as (delete from t where id=1 returning *) select * from d", None),
            ("merge into t using s on true when matched then delete", None),
            ("explain analyze update t set a=1", Some(UpdateWithoutWhere)),
            ("explain analyze delete from t", Some(DeleteWithoutWhere)),
            ("explain analyze delete from t where id=1", None),
            ("explain delete from t", None),
            ("copy (delete from t returning *) to stdout", Some(DeleteWithoutWhere)),
            ("copy (select 1) to stdout", None),
            ("explain analyze select * from t for update", None),
        ] {
            assert_eq!(analyze(sql).danger, want, "{sql}");
        }
    }

    #[test]
    fn flags_sensitive_statements() {
        for (sql, want) in [
            ("alter role app password 'x'", true), ("create user u with password 'x'", true),
            ("create subscription s connection 'host=h password=p' publication x", true),
            ("select 'password'", false), ("create table password (i int)", true), // tutucu: ad olarak da geçse işaretlenir
        ] {
            assert_eq!(analyze(sql).sensitive, want, "{sql}");
        }
    }

    #[test]
    fn hints_row_returning() {
        for (sql, want) in [
            ("select 1", true), ("show all", true), ("explain select 1", true),
            ("insert into t values (1) returning id", true), ("insert into t values (1)", false),
            ("create table t(i int)", false), ("set a = 1", false), ("copy t to stdout", false),
        ] {
            assert_eq!(analyze(sql).returns_rows_hint, want, "{sql}");
        }
    }

    #[test]
    fn detects_read_only_escapes() {
        for (sql, want) in [
            ("set default_transaction_read_only = off", true),
            ("set transaction_read_only to off", true),
            ("reset default_transaction_read_only", true),
            ("select set_config('default_transaction_read_only','off',false)", true),
            ("do $$ begin set default_transaction_read_only = off; end $$", true),
            ("alter role app set default_transaction_read_only = off", true),
            ("alter database d set default_transaction_read_only to off", true),
            ("begin read write", true),
            ("start transaction read write", true),
            ("set transaction read write", true),
            ("set session characteristics as transaction read write", true),
            ("reset all", true), ("discard all", true), ("RESET  /* c */ ALL", true),
            ("do $$ begin reset all; end $$", true),
            ("do $$ begin discard all; end $$", true),
            ("do $$ begin start transaction read write; end $$", true),
            ("do $$ begin set transaction read
  write; end $$", true),
            ("do $$ begin null; end $$", false),
            ("show transaction_read_only", false),
            ("show default_transaction_read_only", false),
            ("select current_setting('transaction_read_only')", false),
            ("set statement_timeout = 0", false), ("set search_path = x", false),
            ("select 'read'", false), ("begin read only", false), ("select 1", false),
        ] {
            assert_eq!(analyze(sql).changes_read_only, want, "{sql}");
        }
    }

    #[test]
    fn recognises_transaction_operations() {
        use TxOp::*;
        for (sql, want) in [("begin", Some(Begin)), ("start transaction read only", Some(Begin)), ("commit", Some(Commit)), ("end", Some(Commit)),
            ("rollback", Some(Rollback)), ("abort", Some(Rollback)), ("rollback to savepoint a", Some(RollbackTo)), ("savepoint a", Some(Savepoint)),
            ("release a", Some(Release)), ("select 1", None), ("rollback prepared 'x'", Some(Other)), ("commit prepared 'x'", Some(Other)),
            ("  /* c */ commit", Some(Commit)), ("", None)] {
            assert_eq!(tx_op(sql), want, "{sql}");
        }
    }
}
