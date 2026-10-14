//! What the page turn and Rambo tell each other: the turner says where the leaf is each
//! frame, and Rambo says whether he is pushing at it from underneath. Neither owns the
//! other, so the two meet here, in one place a test can read.

// Natively only the tests read what the turner shows.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::world::Leaf;
use crate::flip::Push;

thread_local! {
    static LEAF: Cell<Option<Leaf>> = const { Cell::new(None) };
    static PUSH: Cell<Option<Push>> = const { Cell::new(None) };
    static WAKE: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The turner: the leaf as it is now. It is kept after the turn ends (a leaf that lies
/// flat on him keeps him caught), and Rambo is woken when it changes.
pub(crate) fn show(leaf: Leaf) {
    LEAF.set(Some(leaf));
    if let Some(wake) = WAKE.with(|wake| wake.borrow().clone()) {
        wake();
    }
}

/// Rambo: the leaf as the turner last showed it, if it has shown one.
pub(crate) fn leaf() -> Option<Leaf> {
    LEAF.get()
}

/// Rambo: what he pushes the leaf's corner with this frame, if he is under it.
pub(crate) fn set_push(push: Option<Push>) {
    PUSH.set(push);
}

/// The turner: what pushes at the leaf from underneath, if anything.
pub(crate) fn push() -> Option<Push> {
    PUSH.get()
}

/// Rambo: how to wake his loop when the leaf moves while he sleeps.
pub(crate) fn on_leaf(wake: impl Fn() + 'static) {
    WAKE.with(|slot| *slot.borrow_mut() = Some(Rc::new(wake)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(open: f64) -> Leaf {
        Leaf {
            open,
            closing: false,
            origin: (0.0, 0.0),
            size: (390.0, 800.0),
            distance: 1950.0,
        }
    }

    #[test]
    fn the_turner_shows_a_leaf_and_rambo_reads_it_and_is_woken() {
        let woken = Rc::new(Cell::new(0));
        on_leaf({
            let woken = woken.clone();
            move || woken.set(woken.get() + 1)
        });
        show(leaf(0.4));
        show(leaf(0.0));
        assert_eq!(
            super::leaf().map(|leaf| leaf.open),
            Some(0.0),
            "kept after the turn"
        );
        assert_eq!(woken.get(), 2);
    }

    #[test]
    fn rambos_push_is_what_the_turner_reads() {
        set_push(Some(Push {
            accel: 1.0,
            reach: 0.1,
        }));
        assert_eq!(push().map(|push| push.reach), Some(0.1));
        set_push(None);
        assert_eq!(push(), None);
    }
}
