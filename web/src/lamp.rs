//! The desk lamp: the dark-mode switch. Tapping it (or picking Light or Dark in the gear
//! menu, which comes here too) lays a sheet of the other paper down over the page from the
//! top edge; the page's own paper is swapped under it, and the sheet lifts back up the way
//! it came, uncovering the new page. The sheet covers everything on the desk, Rambo and his
//! nest too, and uncovers them with it; nothing fades (docs/DESIGN-DIRECTION.md, spec item
//! 12). Only with reduced motion does the paper change at once. The theme itself stays
//! a page setting, and it never follows the OS.

use crate::settings::{Settings, Theme};
use leptos::prelude::*;

/// The paper the lamp switches to.
pub fn other_theme(theme: Theme) -> Theme {
    match theme {
        Theme::Light => Theme::Dark,
        Theme::Dark => Theme::Light,
    }
}

/// What the lamp's button says to a screen reader: what pressing it does.
pub fn lamp_label(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "Turn the lamp on: night paper",
        Theme::Dark => "Turn the lamp off: day paper",
    }
}

/// What the lamp does with a request for `wanted` paper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Nothing to do: it is that paper already, or the sheet is still moving.
    Ignore,
    /// Change the paper at once: the person asked for less motion.
    Now,
    /// Lay the sheet down, swap under it, lift it.
    Slide,
}

/// The rules of the switch, pure: a request for the paper already showing is a no-op, a
/// tap while the sheet is mid-switch is dropped (the switch in flight finishes), and
/// reduced motion changes the paper with no sheet.
pub fn decide(current: Theme, wanted: Theme, busy: bool, reduced_motion: bool) -> Switch {
    if wanted == current || busy {
        Switch::Ignore
    } else if reduced_motion {
        Switch::Now
    } else {
        Switch::Slide
    }
}

/// Where the sheet is in its trip.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sheet {
    /// Up out of sight.
    Away,
    /// Sliding down onto the page.
    Down,
    /// Lifting back up off it.
    Up,
}

/// A pick of paper waiting to be laid down: the gear menu and the lamp both write it, and
/// the one `DeskLamp` on the page runs it.
#[derive(Clone, Copy)]
pub struct SwitchRequest(RwSignal<Option<Theme>>);

/// Make the request channel available to the whole page. Called once, by the app.
pub fn provide_switch() {
    provide_context(SwitchRequest(RwSignal::new(None)));
}

/// Ask for `theme`. The lamp lays the sheet down; with no lamp on the page (a test) nothing
/// happens.
pub fn switch_to(theme: Theme) {
    if let Some(SwitchRequest(request)) = use_context::<SwitchRequest>() {
        request.set(Some(theme));
    }
}

/// Take the paper to `theme` now, and keep it.
fn swap(set_settings: WriteSignal<Settings>, settings: ReadSignal<Settings>, theme: Theme) {
    let next = Settings {
        theme,
        ..settings.get_untracked()
    };
    set_settings.set(next);
    crate::settings::save(next);
}

/// The lamp on the desk. `settings` is the page's view settings; the swap is saved with
/// them like any other pick.
#[component]
pub fn DeskLamp(
    settings: ReadSignal<Settings>,
    set_settings: WriteSignal<Settings>,
) -> impl IntoView {
    let request = use_context::<SwitchRequest>()
        .expect("provide_switch is called before the lamp")
        .0;
    // The paper the sheet carries, and where it is on its trip.
    let (sheet_paper, set_sheet_paper) = signal(Theme::Dark);
    let (sheet, set_sheet) = signal(Sheet::Away);
    // The cord is mid-pull.
    let (pulled, set_pulled) = signal(false);
    // The paper being switched to, from the click until it has landed: the lamp's own
    // light changes at the click, instantly.
    let (going_to, set_going_to) = signal(None::<Theme>);
    let slide = f64::from(crate::tuning::LAMP_SLIDE_MS);
    Effect::new(move |_| {
        let Some(theme) = request.get() else { return };
        // Clearing the request wakes this effect once more, with nothing to do.
        request.set(None);
        let busy = going_to.get_untracked().is_some();
        let reduced = crate::mascot::prefers_reduced_motion();
        match decide(settings.get_untracked().theme, theme, busy, reduced) {
            Switch::Ignore => return,
            Switch::Now => {
                swap(set_settings, settings, theme);
                return;
            }
            Switch::Slide => {}
        }
        set_going_to.set(Some(theme));
        set_sheet_paper.set(theme);
        set_pulled.set(true);
        // One frame for the sheet to be drawn above the page, then it slides down.
        crate::engine::next_frame(move || set_sheet.set(Sheet::Down));
        crate::engine::after(slide, move || {
            // It covers the page: swap the paper under it, then lift it back off.
            swap(set_settings, settings, theme);
            crate::engine::next_frame(move || set_sheet.set(Sheet::Up));
        });
        crate::engine::after(slide * 2.0 + 50.0, move || {
            set_sheet.set(Sheet::Away);
            set_going_to.set(None);
        });
        crate::engine::after(f64::from(crate::tuning::LAMP_PULL_MS), move || {
            set_pulled.set(false)
        });
    });
    let lit = move || going_to.get().unwrap_or(settings.get().theme) == Theme::Dark;
    view! {
        <button
            type="button"
            class="desk-lamp"
            class:pulled=move || pulled.get()
            class:lit=lit
            aria-label=move || lamp_label(settings.get().theme)
            on:click=move |_| switch_to(other_theme(settings.get_untracked().theme))
        >
            <span class="desk-lamp-art" aria-hidden="true"></span>
            <span class="desk-lamp-cord" aria-hidden="true"></span>
        </button>
        // The other paper, laid down from the top edge over the whole page.
        <div
            class="lamp-sheet"
            class:down=move || sheet.get() == Sheet::Down
            class:up=move || sheet.get() == Sheet::Up
            attr:data-theme=move || if sheet_paper.get() == Theme::Dark { "dark" } else { "light" }
            style:--lamp-slide-ms=format!("{}ms", crate::tuning::LAMP_SLIDE_MS)
            aria-hidden="true"
        ></div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lamp_flips_between_the_two_papers() {
        assert_eq!(other_theme(Theme::Light), Theme::Dark);
        assert_eq!(other_theme(Theme::Dark), Theme::Light);
        assert_ne!(lamp_label(Theme::Light), lamp_label(Theme::Dark));
    }

    #[test]
    fn the_switch_rules() {
        use Theme::{Dark, Light};
        assert_eq!(
            decide(Light, Light, false, false),
            Switch::Ignore,
            "same paper"
        );
        assert_eq!(
            decide(Light, Light, false, true),
            Switch::Ignore,
            "same paper, reduced"
        );
        assert_eq!(
            decide(Light, Dark, true, false),
            Switch::Ignore,
            "a tap mid-switch"
        );
        assert_eq!(
            decide(Light, Dark, true, true),
            Switch::Ignore,
            "mid-switch, reduced"
        );
        assert_eq!(
            decide(Light, Dark, false, true),
            Switch::Now,
            "reduced motion: at once"
        );
        assert_eq!(decide(Dark, Light, false, false), Switch::Slide);
        assert_eq!(decide(Light, Dark, false, false), Switch::Slide);
    }

    /// The sheet is paper over the page, not a touch target: it takes no tap, and it sits in
    /// the switch layer, above Rambo and the nest and under a drag.
    #[test]
    fn the_sheet_never_takes_a_touch() {
        let css = include_str!("../theme.css");
        let rule = css
            .split(".lamp-sheet {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css styles .lamp-sheet");
        assert!(rule.contains("pointer-events: none"), "{rule}");
        assert!(rule.contains("z-index: var(--layer-switch)"), "{rule}");
        assert!(
            !rule.contains("opacity"),
            "the sheet moves, it never fades: {rule}"
        );
    }

    /// The lamp's two pictures are in the tree.
    #[test]
    fn the_lamp_art_ships() {
        for name in ["lamp-off.svg", "lamp-on.svg"] {
            assert!(
                std::path::Path::new("assets/lamp").join(name).exists(),
                "assets/lamp/{name} is missing"
            );
        }
    }
}
