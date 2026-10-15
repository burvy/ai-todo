//! Scribbling a row out, DOM side. The row's own `data-title` says how long
//! the crossing-out takes, so a caller only ever has to name the task.

use super::write::inline_style;
use crate::{mascot, pen, sound};

/// Cross `row` out with ink, then let the cloud take it away. The caller
/// deletes the task once this returns.
pub async fn scribble_out(row_id: &str) {
    let Some(row) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|doc| {
            doc.query_selector(&format!("li[data-id=\"{row_id}\"]"))
                .ok()
                .flatten()
        })
    else {
        return;
    };
    sound::play(sound::Cue::Scribble);
    if mascot::prefers_reduced_motion() {
        return;
    }
    let title = row.get_attribute("data-title").unwrap_or_default();
    let ms = pen::scribble_ms(&title);
    // The row's own crossing-out takes exactly as long as the pen needs,
    // so a long title is not crossed out faster than it can be read.
    if let Some(style) = inline_style(&row) {
        style.set_property("--scribble-ms", &format!("{ms}ms")).ok();
    }
    row.class_list().add_1("scribbling").ok();
    crate::engine::wait(f64::from(ms)).await;
    row.class_list().remove_1("scribbling").ok();
    if let Some(style) = inline_style(&row) {
        style.remove_property("--scribble-ms").ok();
    }
}
