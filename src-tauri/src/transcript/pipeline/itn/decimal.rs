use regex::Regex;
use std::sync::LazyLock;

use crate::transcript::pipeline::edit::TextReplacement;
use crate::transcript::transform::TransformOperation;
use super::number::{parse_digit_sequence, parse_integer_component};

static SPOKEN_DECIMAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9零〇一二两三四五六七八九幺十百千万亿]+)\s*点[。，、\s]?\s*([0-9零〇一二两三四五六七八九幺]+)").unwrap()
});
static DOT_SPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*\.\s*(\d+)").unwrap());

pub fn find_decimal_replacements(text: &str) -> Vec<TextReplacement> {
    let mut out = Vec::new();
    for caps in SPOKEN_DECIMAL_RE.captures_iter(text) {
        let Some(m) = caps.get(0) else { continue; };
        let Some(integer) = parse_integer_component(&caps[1]) else { continue; };
        let Some(frac) = parse_digit_sequence(&caps[2]) else { continue; };
        out.push(TextReplacement {
            range: m.range(),
            replacement: format!("{integer}.{frac}"),
            operation: TransformOperation::MergeDecimal,
            rule_id: "spoken_decimal_continuity",
            confidence: 0.99,
        });
    }
    for caps in DOT_SPACE_RE.captures_iter(text) {
        let Some(m) = caps.get(0) else { continue; };
        if out.iter().any(|r| ranges_overlap(&r.range, &m.range())) { continue; }
        out.push(TextReplacement {
            range: m.range(), replacement: format!("{}.{}", &caps[1], &caps[2]),
            operation: TransformOperation::MergeDecimal,
            rule_id: "ascii_decimal_spacing",
            confidence: 1.0,
        });
    }
    out
}

fn ranges_overlap(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool { a.start < b.end && b.start < a.end }
