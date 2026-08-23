use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageProfile {
    Auto,
    Zh,
    En,
    Ja,
    Ko,
    Mixed,
}

impl LanguageProfile {
    pub fn from_language_tag(tag: Option<&str>) -> Self {
        let Some(tag) = tag.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self::Auto;
        };
        let lower = tag.to_ascii_lowercase();
        if lower == "mixed" || lower == "multi" || lower == "multilingual" {
            Self::Mixed
        } else if lower.starts_with("zh")
            || lower == "cmn"
            || lower == "yue"
            || lower == "chinese"
            || lower == "mandarin"
        {
            Self::Zh
        } else if lower.starts_with("en") || lower == "english" {
            Self::En
        } else if lower.starts_with("ja") || lower == "japanese" {
            Self::Ja
        } else if lower.starts_with("ko") || lower == "korean" {
            Self::Ko
        } else {
            Self::Auto
        }
    }

    pub fn prefers_cjk_spacing(self) -> bool {
        matches!(self, Self::Zh | Self::Ja | Self::Ko)
    }
}
