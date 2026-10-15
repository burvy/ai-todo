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

/// Where the sheet hangs when it is up out of sight, in percent of its own height (the
/// page-high sheet's top edge is at 0 when it covers the page).
const SHEET_AWAY: f64 = -100.0;
/// How near its end, in percent, the sheet counts as landed.
const SHEET_NEAR: f64 = 0.4;

/// The sheet's spring: critically damped, so it lands without bouncing.
const SHEET_PULL: crate::engine::Pull = crate::engine::Pull {
    omega: crate::tuning::LAMP_SHEET_OMEGA,
    zeta: 1.0,
};

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

/// Pull the cord from `from` to `to` px on a loose spring (it swings a little past, like a
/// cord let go). One glide at a time: the pull back takes over from a pull still going.
fn pull_cord(set_cord: WriteSignal<f64>, from: f64, to: f64) {
    let pull = crate::engine::Pull {
        omega: crate::tuning::LAMP_CORD_OMEGA,
        zeta: crate::tuning::LAMP_CORD_ZETA,
    };
    crate::engine::glide_keyed(
        "lamp-cord",
        [from],
        [to],
        pull,
        CORD_NEAR_PX,
        move |[y]| set_cord.set(y),
        move || set_cord.set(to),
    );
}

/// How near, in px, the cord must come to its end to count as there.
const CORD_NEAR_PX: f64 = 0.05;

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
    // The paper the sheet carries, and where it hangs: `None` while it is away, else its
    // top edge in percent of its height (-100 up out of sight, 0 over the page).
    let (sheet_paper, set_sheet_paper) = signal(Theme::Dark);
    let (sheet, set_sheet) = signal(None::<f64>);
    // How far the cord is pulled down, in px.
    let (cord, set_cord) = signal(0.0_f64);
    // The paper being switched to, from the click until it has landed: the lamp's own
    // light changes at the click, instantly.
    let (going_to, set_going_to) = signal(None::<Theme>);
    // The glides and the timer of a switch in progress, stopped if the lamp goes first.
    let timers = StoredValue::new_local(crate::engine::Guard::default());
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
        pull_cord(
            set_cord,
            cord.get_untracked(),
            crate::tuning::LAMP_CORD_DROP_PX,
        );
        set_sheet.set(Some(SHEET_AWAY));
        // It comes down onto the page; once it covers it, the paper is swapped under it
        // and it goes back up the way it came.
        let down = crate::engine::glide(
            [SHEET_AWAY],
            [0.0],
            SHEET_PULL,
            SHEET_NEAR,
            move |[y]| set_sheet.set(Some(y)),
            move || {
                swap(set_settings, settings, theme);
                let up = crate::engine::glide(
                    [0.0],
                    [SHEET_AWAY],
                    SHEET_PULL,
                    SHEET_NEAR,
                    move |[y]| set_sheet.set(Some(y)),
                    move || {
                        set_sheet.set(None);
                        set_going_to.set(None);
                    },
                );
                timers.try_update_value(|timers| timers.keep(up));
            },
        );
        timers.update_value(|timers| {
            timers.keep(down);
            timers.keep(crate::engine::after(
                f64::from(crate::tuning::LAMP_PULL_MS),
                move || pull_cord(set_cord, cord.get_untracked(), 0.0),
            ));
        });
    });
    let lit = move || going_to.get().unwrap_or(settings.get().theme) == Theme::Dark;
    view! {
        <button
            type="button"
            class="desk-lamp"
            style:--cord-y=move || format!("{:.2}px", cord.get())
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
            attr:data-theme=move || if sheet_paper.get() == Theme::Dark { "dark" } else { "light" }
            style:visibility=move || sheet.get().map(|_| "visible")
            style:transform=move || sheet.get().map(|y| format!("translateY({y:.2}%)"))
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
