//! The desk lamp: the dark-mode switch. Tapping it (or pulling its cord) turns the light
//! down over the page, the paper is swapped for the other one under the dimmed light, and
//! the light comes back up on it. Dim and glow are lighting, so they fade; nothing else on
//! the page does (docs/DESIGN-DIRECTION.md, spec item 12). The theme itself stays a page
//! setting the gear menu also sets, and it never follows the OS.

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

/// The lamp on the desk. `settings` is the page's view settings; the swap is saved with
/// them like any other pick.
#[component]
pub fn DeskLamp(
    settings: ReadSignal<Settings>,
    set_settings: WriteSignal<Settings>,
) -> impl IntoView {
    // The light is down: the veil is over the page.
    let (dimmed, set_dimmed) = signal(false);
    // The cord is mid-pull.
    let (pulled, set_pulled) = signal(false);
    // Which way the switch is going, fixed when it starts so the veil keeps its colour
    // while it fades back out over the swapped paper.
    let (to_light, set_to_light) = signal(false);
    let switch = move |_| {
        if dimmed.get_untracked() {
            return;
        }
        let next = Settings {
            theme: other_theme(settings.get_untracked().theme),
            ..settings.get_untracked()
        };
        if crate::mascot::prefers_reduced_motion() {
            set_settings.set(next);
            crate::settings::save(next);
            return;
        }
        set_to_light.set(next.theme == Theme::Light);
        set_pulled.set(true);
        set_dimmed.set(true);
        set_timeout(
            move || {
                // Under the dimmed light the paper is swapped; then the light comes up.
                set_settings.set(next);
                crate::settings::save(next);
                set_dimmed.set(false);
            },
            std::time::Duration::from_millis(u64::from(crate::tuning::LAMP_DIM_MS)),
        );
        set_timeout(
            move || set_pulled.set(false),
            std::time::Duration::from_millis(u64::from(crate::tuning::LAMP_PULL_MS)),
        );
    };
    view! {
        <button
            type="button"
            class="desk-lamp"
            class:pulled=move || pulled.get()
            aria-label=move || lamp_label(settings.get().theme)
            on:click=switch
        >
            <span class="desk-lamp-art" aria-hidden="true"></span>
            <span class="desk-lamp-cord" aria-hidden="true"></span>
        </button>
        // The light turned down: dark under the lamp going on, warm when it goes off.
        <div
            class="lamp-veil"
            class:on=move || dimmed.get()
            class:to-light=move || to_light.get()
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

    /// The veil is only light: it never takes a tap, and it sits in the dim layer, under
    /// Rambo, like the dim while he listens.
    #[test]
    fn the_veil_never_takes_a_touch() {
        let css = include_str!("../theme.css");
        let rule = css
            .split(".lamp-veil {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("theme.css styles .lamp-veil");
        assert!(rule.contains("pointer-events: none"), "{rule}");
        assert!(rule.contains("z-index: var(--layer-dim)"), "{rule}");
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
