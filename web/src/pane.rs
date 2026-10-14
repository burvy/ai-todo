//! The two pages on a phone, the list and the planner. They lie stacked in one cell, the
//! list over the planner, one showing at rest (`main[data-pane]`, set by the page turn in
//! `turner.rs`); there is no scroll row to snap between them. The tests pin the CSS that
//! says so.

#[cfg(test)]
mod tests {
    /// The body of the first rule in `css` whose selector line is exactly `selector`.
    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let start = css
            .find(&format!(
                "
{selector} {{"
            ))
            .unwrap_or_else(|| panic!("no `{selector}` rule"));
        let body = &css[start..];
        &body[..body
            .find(
                "
}",
            )
            .expect("the rule ends")]
    }

    /// BUG-9: `.week` clips its overflow (so the sticky day names stick to
    /// `.week-scroll`), and a clipped box adds nothing to its parent's scroll width. If
    /// `.week` is only as wide as the screen, the later days are cut off and cannot be
    /// scrolled to. It must be at least as wide as its columns on a phone, and may shrink
    /// to the pane on a wide screen.
    #[test]
    fn the_week_is_as_wide_as_its_columns_so_every_day_can_be_scrolled_to() {
        let css = include_str!("../theme.css");
        let week = rule(css, ".week");
        assert!(
            week.contains("overflow-x: clip"),
            "the clip is why the width matters"
        );
        let width = week
            .lines()
            .skip_while(|line| !line.contains("min-width: calc("))
            .take(3)
            .collect::<String>();
        for part in ["--planner-gutter", "--days", "--day-min-w", "--space-1"] {
            assert!(
                width.contains(part),
                "the week's min-width must count {part}: {width}"
            );
        }
        let wide = &css[css
            .find("@media (min-width: 701px)")
            .expect("the wide rules")..];
        assert!(
            rule(wide, "  .week").contains("min-width: 0"),
            "wide screens fit the pane"
        );
    }

    /// The planner's grid keeps its own sideways scroll, and the pages themselves do not
    /// scroll sideways or snap: the pointer decides every sideways swipe on them.
    #[test]
    fn a_sideways_swipe_in_the_grid_does_not_turn_the_page() {
        let css = include_str!("../theme.css");
        assert!(rule(css, ".week-scroll").contains("overscroll-behavior-x: contain"));
        let main = rule(css, "main");
        assert!(
            !main.contains("scroll-snap"),
            "the pages are stacked, not snapped"
        );
        assert!(!main.contains("overflow-x: auto"), "no sideways scroll row");
    }

    /// The planner lies under the list in one grid cell, and at rest only the page that
    /// is showing takes part in the layout.
    #[test]
    fn the_two_pages_lie_in_one_cell_and_one_shows() {
        let css = include_str!("../theme.css");
        assert!(rule(css, "main > *").contains("grid-area: 1 / 1"));
        assert!(
            css.contains(r#"main:not(.bending)[data-pane="0"] > .planner-page"#)
                && css.contains(r#"main:not(.bending)[data-pane="1"] > .tasks-pane"#),
            "the page not showing is left out of the layout at rest"
        );
    }
}
