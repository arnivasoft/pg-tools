use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PropertyRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PropertySection {
    pub title: String,
    pub rows: Vec<PropertyRow>,
}

impl PropertySection {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), rows: Vec::new() }
    }
    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.rows.push(PropertyRow { label: label.into(), value: value.into() });
        self
    }
    pub fn row_opt(self, label: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        match value.map(Into::into).filter(|v: &String| !v.is_empty()) {
            Some(v) => self.row(label, v),
            None => self,
        }
    }
}

pub fn yes_no(value: bool) -> &'static str {
    if value { "Evet" } else { "Hayır" }
}

/// `pg_class.reltuples` tahminini okunur hale getirir; -1 (hiç analiz edilmemiş) için `None`.
pub fn approx_rows(reltuples: f64) -> Option<String> {
    if reltuples < 0.0 {
        return None;
    }
    let text = if reltuples < 1_000.0 {
        format!("~{} satır", reltuples.round() as u64)
    } else if reltuples < 1_000_000.0 {
        format!("~{:.1} B satır", reltuples / 1_000.0)
    } else {
        format!("~{:.1} M satır", reltuples / 1_000_000.0)
    };
    Some(text.replace('.', ","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_skips_none_and_empty() {
        let s = PropertySection::new("Genel")
            .row("Ad", "orders")
            .row_opt("Yorum", None::<String>)
            .row_opt("Boş", Some(""))
            .row_opt("Sahip", Some("app"));
        assert_eq!(s.title, "Genel");
        let labels: Vec<_> = s.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Ad", "Sahip"]);
    }

    #[test]
    fn approx_rows_formats_turkish() {
        assert_eq!(approx_rows(-1.0), None);
        assert_eq!(approx_rows(0.0).as_deref(), Some("~0 satır"));
        assert_eq!(approx_rows(950.0).as_deref(), Some("~950 satır"));
        assert_eq!(approx_rows(12_345.0).as_deref(), Some("~12,3 B satır"));
        assert_eq!(approx_rows(1_200_000.0).as_deref(), Some("~1,2 M satır"));
    }

    #[test]
    fn yes_no_is_turkish() {
        assert_eq!((yes_no(true), yes_no(false)), ("Evet", "Hayır"));
    }
}
