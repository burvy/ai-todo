//! Turning the page on a phone. There are two ways it can look, chosen in the gear menu
//! (`settings::PageTurn`): the book flip (`flip_turner`, the default) and the corner bend
//! (`fold_turner`, the fast one). Both read the same pointer events and both end by naming
//! the page that landed; this picks one for each gesture and hands the events to it.

use leptos::prelude::*;

use crate::settings::{PageTurn, Settings};

#[derive(Clone)]
pub(crate) struct Turner {
    flip: crate::flip_turner::Turner,
    fold: crate::fold_turner::Turner,
    settings: ReadSignal<Settings>,
    /// The one the gesture under way began with: a setting changed mid-turn waits for the
    /// next turn.
    active: std::rc::Rc<std::cell::Cell<PageTurn>>,
}

impl Turner {
    pub(crate) fn new(
        main: NodeRef<leptos::html::Main>,
        fx: NodeRef<leptos::html::Div>,
        pane: ReadSignal<u8>,
        set_pane: WriteSignal<u8>,
        settings: ReadSignal<Settings>,
    ) -> Self {
        Turner {
            flip: crate::flip_turner::Turner::new(main, fx, pane, set_pane),
            fold: crate::fold_turner::Turner::new(main, fx, pane, set_pane),
            settings,
            active: std::rc::Rc::new(std::cell::Cell::new(PageTurn::default())),
        }
    }

    /// Picks the style for a new gesture, unless one is already under way.
    fn choose(&self) {
        if self.flip.idle() && self.fold.idle() {
            self.active.set(self.settings.get_untracked().page_turn);
        }
    }

    pub(crate) fn down(&self, ev: &leptos::ev::PointerEvent) {
        self.choose();
        match self.active.get() {
            PageTurn::Realistic => self.flip.down(ev),
            PageTurn::Fast => self.fold.down(ev),
        }
    }

    pub(crate) fn moved(&self, ev: &leptos::ev::PointerEvent) {
        match self.active.get() {
            PageTurn::Realistic => self.flip.moved(ev),
            PageTurn::Fast => self.fold.moved(ev),
        }
    }

    pub(crate) fn up(&self, ev: &leptos::ev::PointerEvent) {
        match self.active.get() {
            PageTurn::Realistic => self.flip.up(ev),
            PageTurn::Fast => self.fold.up(ev),
        }
    }

    pub(crate) fn cancel(&self, ev: &leptos::ev::PointerEvent) {
        match self.active.get() {
            PageTurn::Realistic => self.flip.cancel(ev),
            PageTurn::Fast => self.fold.cancel(ev),
        }
    }

    /// A page turn from a button or a tap on a page's note.
    pub(crate) fn turn_to(&self, index: u8) {
        self.choose();
        match self.active.get() {
            PageTurn::Realistic => self.flip.turn_to(index),
            PageTurn::Fast => self.fold.turn_to(index),
        }
    }
}
