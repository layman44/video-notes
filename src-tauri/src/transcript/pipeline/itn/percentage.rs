use regex::Regex;
use std::sync::LazyLock;

use crate::transcript::pipeline::edit::TextReplacement;
use crate::transcript::transform::TransformOperation;
use super::number::parse_decimal;

static PERCENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"百分之\s*([0-9零〇一二两三四五六七八九幺十百千万亿]+(?:[点.][0-9零〇一二两三四五六七八九幺]+)?)").unwrap()
});

pub fn find_percentage_replacements(text: &str) -> Vec<TextReplacement> {
    PERCENT_RE.captures_iter(text).filter_map(|caps| {
        let m = caps.get(0)?;
        let candidate = caps.get(1)?.as_str();
        let parsed = parse_decimal(candidate)?;
        Some(TextReplacement {
            range: m.range(),
            replacement: format!("{}%", parsed.to_standard_string()),
            operation: TransformOperation::NormalizePercentage,
            rule_id: "spoken_percentage",
            confidence: 0.99,
        })
    }).collect()
}
