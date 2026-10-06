//! Biçimlendirici seçenekleri (tasarım §5.1). Biçimlendirici `pgt-query::format`'tadır; tipler burada durur çünkü
//! uygulama ayarlarında (`store::AppSettings`) da saklanır ve `pgt-core` `pgt-query`ye bağlı değildir.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KeywordCase {
    #[default]
    Upper,
    Lower,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CommaStyle {
    #[default]
    Trailing,
    Leading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct FormatOptions {
    pub keyword_case: KeywordCase,
    /// Girinti boşluğu: 2 ya da 4.
    pub indent: u8,
    pub comma: CommaStyle,
    /// Satır uzunluğu: 80–200.
    pub max_line: u16,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions { keyword_case: KeywordCase::Upper, indent: 2, comma: CommaStyle::Trailing, max_line: 120 }
    }
}

impl FormatOptions {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.indent, 2 | 4) {
            return Err(Error::Invalid("girinti 2 ya da 4 olmalı".into()));
        }
        if !(80..=200).contains(&self.max_line) {
            return Err(Error::Invalid("satır uzunluğu 80–200 olmalı".into()));
        }
        Ok(())
    }
}
