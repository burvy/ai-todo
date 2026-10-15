//! The desk lamp: the dark-mode switch. Tapping it (or picking Light or Dark in the gear
//! menu, which comes here too) lays a sheet of the other paper down over the page from the
//! top edge; the page's own paper is swapped under it, and the sheet is taken away on the
//! next frame, so the page is swapped out and nothing fades (docs/DESIGN-DIRECTION.md, spec
//! item 12). Only with reduced motion does the paper change at once. The theme itself stays
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
    // The paper the sheet carries while it is laid down, and whether it has been sent.
    let (sheet, set_sheet) = signal(Theme::Dark);
    let (laying, set_laying) = signal(false);
    // The cord is mid-pull.
    let (pulled, set_pulled) = signal(false);
    // The paper being switched to, from the click until it has landed: the lamp's own
    // light changes at the click, instantly.
    let (going_to, set_going_to) = signal(None::<Theme>);
    Effect::new(move |_| {
        let Some(theme) = request.get() else { return };
        request.set(None);
        if theme == settings.get_untracked().theme || going_to.get_untracked().is_some() {
            return;
        }
        if crate::mascot::prefers_reduced_motion() {
            swap(set_settings, settings, theme);
            return;
        }
        set_going_to.set(Some(theme));
        set_sheet.set(theme);
        set_pulled.set(true);
        // One frame for the sheet to be drawn above the page, then it slides down.
        crate::engine::next_frame(move || set_laying.set(true));
        set_timeout(
            move || {
                // It covers the page: swap the paper under it, then take it away.
                swap(set_settings, settings, theme);
                crate::engine::next_frame(move || {
                    crate::engine::next_frame(move || {
                        set_laying.set(false);
                        set_going_to.set(None);
                    })
                });
            },
            std::time::Duration::from_millis(u64::from(crate::tuning::LAMP_SLIDE_MS)),
        );
        set_timeout(
            move || set_pulled.set(false),
            std::time::Duration::from_millis(u64::from(crate::tuning::LAMP_PULL_MS)),
        );
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
            class:laying=move || laying.get()
            attr:data-theme=move || if sheet.get() == Theme::Dark { "dark" } else { "light" }
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

    /// The sheet is paper over the page, not a touch target: it takes no tap, and it sits in
    /// the dim layer, under Rambo.
    #[test]
    fn the_sheet_never_takes_a_touch() {
        let css = include_str!("../theme.css");
        let rule = css
            .split(".lamp-sheet {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css styles .lamp-sheet");
        assert!(rule.contains("pointer-events: none"), "{rule}");
        assert!(rule.contains("z-index: var(--layer-dim)"), "{rule}");
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
