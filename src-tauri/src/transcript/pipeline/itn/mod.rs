pub mod decimal;
pub mod number;
pub mod percentage;

use crate::transcript::model::{CanonicalSegment, LanguageProfile};
use crate::transcript::pipeline::edit::apply_text_replacements;
use crate::transcript::transform::{TransformLog, TransformStage};

pub fn run_itn_engine(segments: &mut [CanonicalSegment], profile: LanguageProfile, log: &mut TransformLog) {
    for seg in segments.iter_mut() {
        let decimals = decimal::find_decimal_replacements(&seg.text);
        apply_text_replacements(seg, profile, TransformStage::Itn, &decimals, log);

        let percentages = percentage::find_percentage_replacements(&seg.text);
        apply_text_replacements(seg, profile, TransformStage::Itn, &percentages, log);
    }
}
