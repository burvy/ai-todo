//! Rambo's machine is in `rambl_web_core::mascot` (ARCHITECTURE 8A); it is re-exported
//! here under the path the page has always used. What stays is what needs the browser,
//! and the tests that read the page's own sources and style sheet.

pub use rambl_web_core::mascot::*;

#[cfg(target_arch = "wasm32")]
pub fn prefers_reduced_motion() -> bool {
    leptos::prelude::window()
        .match_media("(prefers-reduced-motion: reduce)")
        .ok()
        .flatten()
        .is_some_and(|query| query.matches())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn prefers_reduced_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn he_sits_in_the_nest_between_its_back_and_its_rim() {
        // The order of the nest's layers is data in :root: its back is drawn with
        // the capture bar, he is above that, and the rim is above him.
        let theme = include_str!("../theme.css");
        let layer = |name: &str| -> i32 {
            let at = theme
                .find(&format!("--layer-{name}:"))
                .unwrap_or_else(|| panic!("--layer-{name} in :root"));
            theme[at..]
                .split([':', ';'])
                .nth(1)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("--layer-{name} is a number"))
        };
        assert!(
            layer("capture") < layer("rambo"),
            "the nest's back is behind him"
        );
        assert!(
            layer("rambo") < layer("nest-rim"),
            "the rim is in front of him"
        );
        assert!(
            layer("nest-rim") < layer("rambo-bubble"),
            "the rim never covers what he says"
        );
    }

    /// Each `property` set on the Rambo button itself, in any state, as
    /// (selector, value).
    fn rambo_rules(css: &str, property: &str) -> Vec<(String, String)> {
        let property = format!("{property}:");
        let mut bare = String::new();
        let mut rest = css;
        while let Some((before, after)) = rest.split_once("/*") {
            bare.push_str(before);
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        }
        bare.push_str(rest);
        let is_button = |selector: &str| {
            selector.strip_prefix(".rambo").is_some_and(|tail| {
                tail.split(['.', ':'])
                    .all(|part| part.chars().all(|c| c.is_alphanumeric() || c == '-'))
                    && !tail.contains("::")
            })
        };
        bare.split('}')
            .filter_map(|block| block.rsplit_once('{'))
            .flat_map(|(head, body)| {
                let selector = head.rsplit(['{', ';']).next().unwrap_or(head);
                selector
                    .split(',')
                    .map(str::trim)
                    .filter(|one| is_button(one))
                    .flat_map(|one| {
                        body.split(';')
                            .filter_map(|decl| decl.trim().strip_prefix(property.as_str()))
                            .map(|value| (one.to_owned(), value.trim().to_owned()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Every argument handed to a `.set(...)` signal write, trimmed. A call
    /// whose argument names a `State` constructor writes the mascot's state
    /// directly instead of asking `transition`.
    fn direct_state_sets(source: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some((_, after)) = rest.split_once(".set(") {
            // The argument runs to the parenthesis that closes the call.
            let mut depth = 1;
            let end = after
                .char_indices()
                .find_map(|(i, c)| match c {
                    '(' => {
                        depth += 1;
                        None
                    }
                    ')' => {
                        depth -= 1;
                        (depth == 0).then_some(i)
                    }
                    _ => None,
                })
                .unwrap_or(after.len());
            let arg = after[..end].trim();
            if arg.starts_with("State::") || arg.starts_with("mascot::State::") {
                found.push(arg);
            }
            rest = &after[end..];
        }
        found
    }

    /// The body of the handler written right after `on:{event}=` in `rambo.rs`.
    fn rambo_handler(event: &str) -> &'static str {
        let source = include_str!("rambo.rs");
        let (_, rest) = source
            .split_once(&format!("on:{event}="))
            .expect("rambo.rs has the handler");
        rest.split_once("\n            on:")
            .map_or(rest, |(body, _)| body)
    }

    #[test]
    fn picking_him_up_does_not_listen_at_once() {
        // Emilio saw "No speech was detected." after dropping him on a task: every
        // grab listened. Listening waits for a still hold (`LISTEN_HOLD_MS`).
        let down = rambo_handler("pointerdown");
        assert!(down.contains("LISTEN_HOLD_MS"), "{down}");
        let before_timer = down.split_once("LISTEN_HOLD_MS").map_or(down, |(b, _)| b);
        assert!(
            !before_timer.contains("start_speech_down();"),
            "a grab must not start listening before the hold: {down}"
        );
    }

    #[test]
    fn edit_in_the_ask_bubble_opens_the_edit_sheet() {
        // QA on 39eb0d9: "Edit" closed the bubble and bumped the task's priority
        // instead of opening the sheet. The button must ask the list for its sheet.
        // It also listens for the change. The bubble's buttons are `Choice`s; Edit's
        // errand (`rambo::chosen`) is its arm in rambo.rs's `choose`.
        let source = include_str!("rambo.rs");
        let (_, arm) = source
            .split_once("model::Errand::Edit(")
            .expect("the bubble has an Edit choice");
        let handler = arm
            .split_once("model::Errand::Move")
            .map_or(arm, |(edit, _)| edit);
        assert!(handler.contains("listen_for_edit("), "{handler}");
        assert!(
            handler.contains("Event::EditAsked"),
            "Edit must open the edit sheet: {handler}"
        );
        assert!(
            !handler.contains("patch_and_show"),
            "Edit must not change the task itself: {handler}"
        );
    }

    #[test]
    fn every_state_change_goes_through_the_machine() {
        // The DOM layer never writes a `State` into the signal; it asks
        // `transition`, so the machine's tests cover every path the page can
        // take. `set(transition(&State::…, …))` is fine: that `State` is the
        // *before*, not the write.
        for (name, source) in [
            ("rambo.rs", include_str!("rambo.rs")),
            ("tasks/mod.rs", include_str!("tasks/mod.rs")),
            ("tasks/write.rs", include_str!("tasks/write.rs")),
            ("capture.rs", include_str!("capture.rs")),
        ] {
            let direct = direct_state_sets(source);
            assert!(
                direct.is_empty(),
                "{name} writes a State directly instead of asking `transition`: {direct:?}"
            );
        }
    }

    #[test]
    fn no_style_rule_moves_his_button() {
        // The engine owns his transform (engine/look.rs): a rule's transform or
        // animation on the button would fight it, or place him by itself and send him
        // to the window's top-left corner.
        let css = include_str!("../theme.css");
        let moved: Vec<_> = ["transform", "animation", "translate", "rotate", "scale"]
            .into_iter()
            .flat_map(|property| rambo_rules(css, property))
            .collect();
        assert!(moved.is_empty(), "a rule moves him: {moved:?}");
        assert!(
            !rambo_rules(css, "cursor").is_empty(),
            "the parser finds the Rambo rules"
        );
    }

    #[test]
    fn nothing_glides_him_from_place_to_place() {
        // He hops (`crate::hop`), drawn frame by frame. A transform transition on
        // the button would glide him instead, and also runs when a state's
        // animation ends, where Chrome starts it near the window's corner: every
        // drop sent him to the top-left and back.
        let eased: Vec<_> = rambo_rules(include_str!("../theme.css"), "transition")
            .into_iter()
            .filter(|(_, value)| value.contains("transform") || value.contains("all"))
            .collect();
        assert!(eased.is_empty(), "glides: {eased:?}");
    }

    /// docs/FRONTEND-PLAN.md 1.5: every z-index in the style sheets names a rung of the
    /// scale in `:root`, so no loose number can put Rambo over a dialog again.
    #[test]
    fn every_z_index_reads_the_layer_scale() {
        let css = include_str!("../theme.css");
        let loose: Vec<_> = css
            .lines()
            .filter(|line| line.trim_start().starts_with("z-index:"))
            .filter(|line| !line.contains("var(--layer-"))
            .collect();
        assert!(
            loose.is_empty(),
            "theme.css has loose z-index numbers: {loose:?}"
        );
    }

    /// The dimmed light while he listens never takes a touch: a drop, a tap off his
    /// bubble or a tap on the page lands on what is under it.
    #[test]
    fn the_dim_while_he_listens_never_takes_a_touch() {
        let css = include_str!("../theme.css");
        let rule = css
            .split(".listening-dim {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css styles .listening-dim");
        assert!(rule.contains("pointer-events: none"), "{rule}");
        assert!(rule.contains("z-index: var(--layer-dim)"), "{rule}");
    }

    /// Rambo and his bubbles sit under the edit sheet and the gear menu; the sheet is
    /// the highest rung a person can open.
    #[test]
    fn rambo_and_his_bubbles_sit_under_the_sheet() {
        let css = include_str!("../theme.css");
        let layer = |name: &str| -> i32 {
            css.lines()
                .find_map(|line| line.trim().strip_prefix(&format!("--layer-{name}:")))
                .and_then(|rest| rest.split(';').next())
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or_else(|| panic!("--layer-{name} is not a plain number in :root"))
        };
        assert!(layer("rambo") < layer("rambo-bubble"));
        assert!(layer("rambo-bubble") < layer("menu"));
        assert!(layer("menu") < layer("sheet"));
        assert!(layer("note") < layer("rambo"));
        // The light turned down while he listens lies over the page and under him.
        assert!(layer("note") < layer("dim"));
        assert!(layer("dim") < layer("rambo"));
        // The capture bar is above everything sticky: day names.
        assert!(layer("sticky") < layer("capture"));
    }
}
