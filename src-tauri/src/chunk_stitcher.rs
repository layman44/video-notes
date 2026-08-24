use crate::transcriber::TranscriptSegment;

#[derive(Debug, Clone, Default)]
pub(crate) struct StitchStats {
    pub match_tokens: usize,
    pub method: &'static str,
}

#[derive(Debug, Clone)]
struct TokenRef {
    norm: String,
    byte_start: usize,
    byte_end: usize,
}

fn is_cjk_like(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0xAC00..=0xD7AF
            | 0x1100..=0x11FF
    )
}

fn tokenize_segment(text: &str) -> Vec<TokenRef> {
    let mut tokens = Vec::new();
    let mut ascii_start: Option<usize> = None;
    let mut ascii_end = 0usize;

    let flush_ascii = |tokens: &mut Vec<TokenRef>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            if end > s {
                let raw = &text[s..end];
                tokens.push(TokenRef {
                    norm: raw.to_ascii_lowercase(),
                    byte_start: s,
                    byte_end: end,
                });
            }
        }
    };

    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if ch.is_ascii_alphanumeric() || ch == '\'' {
            if ascii_start.is_none() {
                ascii_start = Some(idx);
            }
            ascii_end = next;
            continue;
        }

        flush_ascii(&mut tokens, &mut ascii_start, ascii_end);
        if is_cjk_like(ch) {
            tokens.push(TokenRef {
                norm: ch.to_string(),
                byte_start: idx,
                byte_end: next,
            });
        } else if ch.is_alphanumeric() {
            tokens.push(TokenRef {
                norm: ch.to_lowercase().collect::<String>(),
                byte_start: idx,
                byte_end: next,
            });
        }
    }

    flush_ascii(&mut tokens, &mut ascii_start, ascii_end);
    tokens
}

fn min_match_tokens(matched_tokens: &[TokenRef]) -> usize {
    if matched_tokens.is_empty() { return 3; }
    let cjk = matched_tokens.iter().filter(|t| t.norm.chars().any(is_cjk_like)).count();
    if cjk * 2 >= matched_tokens.len() {
        5
    } else {
        3
    }
}

#[derive(Debug, Clone, Copy)]
struct ExactAnchor {
    side_start: usize,
    side_end: usize,
    bridge_start: usize,
    bridge_end: usize,
    matched: usize,
}

fn exact_contiguous_anchors(side: &[TokenRef], bridge: &[TokenRef]) -> Vec<ExactAnchor> {
    let mut out = Vec::new();
    if side.is_empty() || bridge.is_empty() { return out; }
    for i in 0..side.len() {
        for j in 0..bridge.len() {
            if side[i].norm != bridge[j].norm { continue; }
            if i > 0 && j > 0 && side[i - 1].norm == bridge[j - 1].norm {
                continue; // not the start of a maximal run
            }
            let mut len = 0usize;
            while i + len < side.len()
                && j + len < bridge.len()
                && side[i + len].norm == bridge[j + len].norm
            {
                len += 1;
            }
            if len == 0 { continue; }
            out.push(ExactAnchor {
                side_start: i,
                side_end: i + len - 1,
                bridge_start: j,
                bridge_end: j + len - 1,
                matched: len,
            });
        }
    }
    out
}

fn trim_leading_noise(text: &str) -> &str {
    text.trim_start_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, ',' | '.' | '?' | '!' | ':' | ';' | '，' | '。' | '？' | '！' | '：' | '；' | '、')
    })
}

/// 0.7 authoritative boundary bridge.
///
/// The bridge transcription is centered on the chunk boundary, so it has better
/// left/right context than either main window edge. We use *exact* token anchors
/// on both sides and replace only the unstable text between those anchors with the
/// bridge text. There is no phonetic, edit-distance, or semantic guessing.
pub(crate) fn rewrite_with_authoritative_bridge(
    previous: &mut Vec<TranscriptSegment>,
    current: &mut Vec<TranscriptSegment>,
    bridge: &[TranscriptSegment],
) -> StitchStats {
    if previous.is_empty() || current.is_empty() || bridge.is_empty() {
        return StitchStats::default();
    }

    let prev_index = previous.len() - 1;
    let prev_text = previous[prev_index].text.clone();
    let curr_text = current[0].text.clone();
    let bridge_text = bridge
        .iter()
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if prev_text.trim().is_empty() || curr_text.trim().is_empty() || bridge_text.trim().is_empty() {
        return StitchStats::default();
    }

    let mut p = tokenize_segment(&prev_text);
    let mut c = tokenize_segment(&curr_text);
    let mut r = tokenize_segment(&bridge_text);
    const SIDE_MAX: usize = 96;
    const REF_MAX: usize = 192;
    if p.len() > SIDE_MAX { p = p[p.len() - SIDE_MAX..].to_vec(); }
    if c.len() > SIDE_MAX { c.truncate(SIDE_MAX); }
    if r.len() > REF_MAX { r.truncate(REF_MAX); }
    if p.is_empty() || c.is_empty() || r.is_empty() { return StitchStats::default(); }

    let lefts = exact_contiguous_anchors(&p, &r)
        .into_iter()
        .filter(|a| p.len().saturating_sub(1 + a.side_end) <= 8)
        .filter(|a| a.matched >= min_match_tokens(&p[a.side_start..=a.side_end]))
        .collect::<Vec<_>>();
    let mut rights = Vec::new();
    for anchor in exact_contiguous_anchors(&c, &r) {
        let max_shift = anchor.matched.saturating_sub(3).min(6);
        for shift in 0..=max_shift {
            let adjusted = ExactAnchor {
                side_start: anchor.side_start + shift,
                side_end: anchor.side_end,
                bridge_start: anchor.bridge_start + shift,
                bridge_end: anchor.bridge_end,
                matched: anchor.matched - shift,
            };
            if adjusted.side_start > 6 { continue; }
            let normal_min = min_match_tokens(&c[adjusted.side_start..=adjusted.side_end]);
            let authoritative_min = if c.len() <= 6 && adjusted.side_start <= 1 { normal_min.min(3) } else { normal_min };
            if adjusted.matched < authoritative_min { continue; }
            rights.push(adjusted);
        }
    }
    if lefts.is_empty() || rights.is_empty() { return StitchStats::default(); }

    let mut best: Option<(ExactAnchor, ExactAnchor, usize)> = None;
    for left in &lefts {
        for right in &rights {
            if left.bridge_end >= right.bridge_start { continue; }
            let bridge_gap = right.bridge_start - left.bridge_end - 1;
            if bridge_gap > 48 { continue; }
            let mid_start = r[left.bridge_end].byte_end.min(bridge_text.len());
            let mid_end = r[right.bridge_start].byte_start.min(bridge_text.len());
            let middle_preview = if mid_end >= mid_start {
                bridge_text[mid_start..mid_end].trim()
            } else {
                ""
            };
            let punctuation_bonus = match middle_preview.chars().last() {
                Some('。' | '？' | '！' | '.' | '?' | '!') => 10,
                Some('，' | '；' | '：' | ',' | ';' | ':') => 8,
                _ => 0,
            };
            let score = left.matched + right.matched + punctuation_bonus;
            match best {
                Some((_, _, best_score)) if score <= best_score => {}
                _ => best = Some((*left, *right, score)),
            }
        }
    }
    let Some((left, right, _)) = best else { return StitchStats::default(); };

    let prev_cut = p[left.side_end].byte_end.min(prev_text.len());
    let bridge_mid_start = r[left.bridge_end].byte_end.min(bridge_text.len());
    let bridge_mid_end = r[right.bridge_start].byte_start.min(bridge_text.len());
    let curr_keep = c[right.side_start].byte_start.min(curr_text.len());
    if bridge_mid_end < bridge_mid_start { return StitchStats::default(); }

    let middle = bridge_text[bridge_mid_start..bridge_mid_end].trim();

    let mut new_prev = prev_text[..prev_cut].trim_end().to_string();
    if !new_prev.is_empty() {
        let last = new_prev.chars().last().unwrap();
        let first = middle.chars().next();
        if last.is_ascii_alphanumeric() && first.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false) {
            new_prev.push(' ');
        }
    }
    new_prev.push_str(middle);
    new_prev = new_prev.trim().to_string();

    let new_curr = trim_leading_noise(&curr_text[curr_keep..]).to_string();
    if new_prev.is_empty() || new_curr.is_empty() { return StitchStats::default(); }

    previous[prev_index].text = new_prev;
    current[0].text = new_curr;

    StitchStats {
        match_tokens: left.matched + right.matched,
        method: "authoritative-bridge",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment { id: format!("{start}-{end}"), start, end, text: text.to_string() }
    }

    #[test]
    fn authoritative_bridge_repairs_split_sentence_without_phonetics() {
        let mut previous = vec![seg(98.77, 120.41, "直至把电跑光，最终计算车辆的纯电续航达成率。晚上可能我中了南墙才会回头吧，可能我见了黄河才会倒。")];
        let mut current = vec![seg(121.045, 130.76, "死心吧，可能我偏要一条路走到黑吧，可能我还没遇见那个他吧。")];
        let bridge = vec![seg(115.0, 125.0, "晚上可能我中了南墙才会回头吧，可能我见了黄河才会死心吧，可能我偏要一条路走到黑吧。")];
        let stats = rewrite_with_authoritative_bridge(&mut previous, &mut current, &bridge);
        assert_eq!(stats.method, "authoritative-bridge");
        assert!(previous[0].text.ends_with("可能我见了黄河才会死心吧，"));
        assert!(current[0].text.starts_with("可能我偏要一条路走到黑吧"));
        assert!(!previous[0].text.contains("才会倒"));
    }

    #[test]
    fn authoritative_bridge_can_remove_one_unstable_cjk_leading_char() {
        let mut previous = vec![seg(55.0, 60.0, "很多上班族")];
        let mut current = vec![seg(60.0, 61.0, "足的一天。")];
        let bridge = vec![seg(56.0, 62.0, "很多上班族的一天。")];
        let stats = rewrite_with_authoritative_bridge(&mut previous, &mut current, &bridge);
        assert_eq!(stats.method, "authoritative-bridge");
        assert_eq!(previous[0].text, "很多上班族");
        assert_eq!(current[0].text, "的一天。");
    }
}
