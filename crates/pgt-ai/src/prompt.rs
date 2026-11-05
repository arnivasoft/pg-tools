//! İstem metni. Sistem istemi sabittir; değişken bağlam kullanıcı mesajında etiketli bölümlerle gönderilir.

use crate::model::{AiMode, AiRequest};

/// Bir istekte gönderilebilecek en çok bağlam (karakter). Aşılırsa istek reddedilir (sessizce kırpılmaz).
pub const MAX_INPUT_CHARS: usize = 400_000;

pub const SYSTEM: &str = "You are the SQL assistant inside pg-tools, a PostgreSQL administration desktop app. \
The user is a database developer or administrator. Reply in Turkish; keep SQL keywords and identifiers as they are.

Rules:
- Target PostgreSQL. When a server version is given, use only syntax that version supports.
- Use only tables and columns that appear in the provided schema when one is given. If something needed is missing, say so instead of inventing names.
- Put every SQL statement in a single ```sql fenced block. The user inserts that block into the editor and runs it themselves; nothing runs automatically.
- Prefer read-only queries. If the task needs data-changing or destructive SQL (UPDATE, DELETE, DROP, TRUNCATE, ALTER), say so in one line before the block and keep it scoped (WHERE clause, explicit names).
- Be brief: a short explanation, then the SQL. No preamble.";

fn task(mode: AiMode) -> &'static str {
    match mode {
        AiMode::Generate => "Respond to the request below. When it asks for data or a change, write the SQL; when it is a follow-up question, answer it.",
        AiMode::Explain => "Explain what the SQL below does, step by step, and point out performance or correctness risks. Do not rewrite it unless there is a bug.",
        AiMode::Fix => "The SQL below failed with the given error. Explain the cause in one or two sentences and give the corrected SQL.",
    }
}

fn section(out: &mut String, tag: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    out.push_str(&format!("<{tag}>\n{body}\n</{tag}>\n\n"));
}

/// Kullanıcı mesajı. Eksik zorunlu alan için `Err` (Türkçe açıklama). `include_schema`: sohbette şema özeti
/// yalnız ilk turda ya da değiştiğinde gönderilir (önceki turlar zaten içerir; geçmiş yalnız eklenerek büyür).
pub fn user_message(req: &AiRequest, include_schema: bool) -> Result<String, String> {
    match req.mode {
        AiMode::Generate if req.prompt.trim().is_empty() => return Err("Ne istediğinizi yazın.".into()),
        AiMode::Explain if req.sql.trim().is_empty() => return Err("Açıklanacak SQL yok: editörde bir ifade seçin.".into()),
        AiMode::Fix if req.sql.trim().is_empty() => return Err("Düzeltilecek SQL yok: editörde bir ifade seçin.".into()),
        AiMode::Fix if req.error.trim().is_empty() => return Err("Düzeltilecek bir hata yok: önce sorguyu çalıştırın.".into()),
        _ => {}
    }
    let mut out = String::new();
    section(&mut out, "server_version", &req.server_version);
    if include_schema {
        section(&mut out, "schema", &req.schema);
    }
    section(&mut out, "sql", &req.sql);
    section(&mut out, "error", &req.error);
    section(&mut out, "request", &req.prompt);
    out.push_str(task(req.mode));
    if out.chars().count() > MAX_INPUT_CHARS {
        return Err(format!(
            "Gönderilecek bağlam çok büyük ({MAX_INPUT_CHARS} karakter sınırı). Daha küçük bir SQL seçin."
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: AiMode) -> AiRequest {
        AiRequest {
            mode,
            prompt: String::new(),
            sql: String::new(),
            error: String::new(),
            schema: String::new(),
            server_version: String::new(),
        }
    }

    #[test]
    fn generate_requires_prompt_and_skips_empty_sections() {
        assert!(user_message(&req(AiMode::Generate), true).is_err());
        let mut r = req(AiMode::Generate);
        r.prompt = "son 10 sipariş".into();
        r.schema = "public.orders(id int4, created_at timestamptz)".into();
        let m = user_message(&r, true).unwrap();
        assert!(m.contains("<schema>\npublic.orders"));
        assert!(m.contains("<request>\nson 10 sipariş\n</request>"));
        assert!(!m.contains("<sql>"));
        assert!(!m.contains("<error>"));
        assert!(m.ends_with(task(AiMode::Generate)));
    }

    #[test]
    fn schema_can_be_left_out_on_follow_up_turns() {
        let mut r = req(AiMode::Generate);
        r.prompt = "neden?".into();
        r.schema = "public.t(id int4)".into();
        assert!(!user_message(&r, false).unwrap().contains("<schema>"));
    }

    #[test]
    fn fix_requires_sql_and_error() {
        let mut r = req(AiMode::Fix);
        r.sql = "select * form t".into();
        assert!(user_message(&r, true).is_err());
        r.error = "syntax error at or near \"form\"".into();
        let m = user_message(&r, true).unwrap();
        assert!(m.contains("<sql>\nselect * form t\n</sql>"));
        assert!(m.contains("<error>"));
    }

    #[test]
    fn explain_requires_sql() {
        assert!(user_message(&req(AiMode::Explain), true).is_err());
    }

    #[test]
    fn oversized_context_is_rejected_not_truncated() {
        let mut r = req(AiMode::Explain);
        r.sql = "x".repeat(MAX_INPUT_CHARS + 1);
        assert!(user_message(&r, true).is_err());
    }
}
