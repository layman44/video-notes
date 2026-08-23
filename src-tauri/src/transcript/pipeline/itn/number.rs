#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedNumber {
    Integer(i64),
    Decimal { integer: i64, fractional: String },
    DigitSequence(String),
}

impl ParsedNumber {
    pub fn to_standard_string(&self) -> String {
        match self {
            ParsedNumber::Integer(v) => v.to_string(),
            ParsedNumber::Decimal { integer, fractional } => format!("{integer}.{fractional}"),
            ParsedNumber::DigitSequence(v) => v.clone(),
        }
    }
}

pub fn parse_digit_sequence(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() { return None; }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        out.push(match ch {
            '0' | '零' | '〇' => '0',
            '1' | '一' | '幺' => '1',
            '2' | '二' | '两' => '2',
            '3' | '三' => '3',
            '4' | '四' => '4',
            '5' | '五' => '5',
            '6' | '六' => '6',
            '7' | '七' => '7',
            '8' | '八' => '8',
            '9' | '九' => '9',
            _ => return None,
        });
    }
    Some(out)
}

/// Conservative cardinal parser. Colloquial omissions such as "二百五" / "一千二" are ambiguous
/// (205 vs 250, 1002 vs 1200) and deliberately return None in Canonical ITN.
pub fn parse_cardinal(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() { return None; }
    if trimmed.chars().all(|c| c.is_ascii_digit()) { return trimmed.parse().ok(); }
    let has_unit = trimmed.chars().any(|c| matches!(c, '十' | '百' | '千' | '万' | '亿'));
    if !has_unit && trimmed.chars().all(|c| digit_value(c).is_some()) {
        return parse_digit_sequence(trimmed)?.parse().ok();
    }
    if is_ambiguous_omitted_unit(trimmed) { return None; }

    let mut total = 0i64;
    let mut section = 0i64;
    let mut number = 0i64;
    let mut seen = false;

    for ch in trimmed.chars() {
        if let Some(d) = digit_value(ch) {
            number = d;
            seen = true;
            continue;
        }
        match ch {
            '十' | '百' | '千' => {
                let unit = match ch { '十' => 10, '百' => 100, _ => 1000 };
                if number == 0 { number = 1; }
                section += number * unit;
                number = 0;
                seen = true;
            }
            '万' => {
                section += number;
                if section == 0 { section = 1; }
                total += section * 10_000;
                section = 0;
                number = 0;
                seen = true;
            }
            '亿' => {
                section += number;
                let base = total + section;
                total = if base == 0 { 100_000_000 } else { base * 100_000_000 };
                section = 0;
                number = 0;
                seen = true;
            }
            _ => return None,
        }
    }
    seen.then_some(total + section + number)
}

pub fn parse_decimal(text: &str) -> Option<ParsedNumber> {
    let trimmed = text.trim();
    if let Some((int_part, frac_part)) = trimmed.split_once(|c| c == '点' || c == '.') {
        let integer = parse_integer_component(int_part)?;
        let fractional = parse_digit_sequence(frac_part)?;
        return Some(ParsedNumber::Decimal { integer, fractional });
    }
    parse_cardinal(trimmed).map(ParsedNumber::Integer)
}

pub fn parse_integer_component(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() { return Some(0); }
    if t.chars().any(|c| matches!(c, '十' | '百' | '千' | '万' | '亿')) {
        parse_cardinal(t)
    } else if t.chars().all(|c| c.is_ascii_digit()) {
        t.parse().ok()
    } else {
        parse_digit_sequence(t)?.parse().ok()
    }
}

fn digit_value(ch: char) -> Option<i64> {
    Some(match ch {
        '0' | '零' | '〇' => 0,
        '1' | '一' | '幺' => 1,
        '2' | '二' | '两' => 2,
        '3' | '三' => 3,
        '4' | '四' => 4,
        '5' | '五' => 5,
        '6' | '六' => 6,
        '7' | '七' => 7,
        '8' | '八' => 8,
        '9' | '九' => 9,
        _ => return None,
    })
}

fn is_ambiguous_omitted_unit(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let Some(last) = chars.last().copied() else { return false; };
    if digit_value(last).is_none() { return false; }
    let Some(pos) = chars.iter().rposition(|c| matches!(c, '百' | '千' | '万' | '亿')) else { return false; };
    let suffix: String = chars[pos + 1..].iter().collect();
    if suffix.is_empty() { return false; }
    let has_explicit_lower_unit = suffix.chars().any(|c| matches!(c, '十' | '百' | '千'));
    let has_zero_placeholder = suffix.chars().any(|c| matches!(c, '零' | '〇' | '0'));
    !has_explicit_lower_unit && !has_zero_placeholder && suffix.chars().filter(|c| digit_value(*c).is_some()).count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unambiguous_numbers() {
        assert_eq!(parse_cardinal("十二"), Some(12));
        assert_eq!(parse_cardinal("八十二"), Some(82));
        assert_eq!(parse_cardinal("一百二十三"), Some(123));
        assert_eq!(parse_cardinal("两百"), Some(200));
        assert_eq!(parse_cardinal("三千五百"), Some(3500));
        assert_eq!(parse_cardinal("一万零五百"), Some(10500));
        assert_eq!(parse_digit_sequence("二零二六"), Some("2026".into()));
    }

    #[test]
    fn refuses_colloquial_ambiguity() {
        assert_eq!(parse_cardinal("二百五"), None);
        assert_eq!(parse_cardinal("一千二"), None);
        assert_eq!(parse_cardinal("一万五"), None);
        assert_eq!(parse_cardinal("二百零五"), Some(205));
    }
}
