//! Rambo's one owner (ARCHITECTURE 8A.5): the engine's world holds his state. A view
//! tells it an event, the world answers his new state, and the page's `mascot_state`
//! mirrors that answer. Only this handle writes the mirror.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;

use crate::engine::World;
use crate::mascot;

/// The world Rambo lives in, shared with every view through Leptos context.
#[derive(Clone, Copy)]
pub(crate) struct RamboWorld {
    world: StoredValue<Rc<RefCell<World>>, LocalStorage>,
    state: ReadSignal<mascot::State>,
    mirror: WriteSignal<mascot::State>,
}

impl RamboWorld {
    /// Rambo at rest in his nest at `home`, with `state` as the page's mirror of him.
    pub(crate) fn provide(
        home: (f64, f64),
        state: ReadSignal<mascot::State>,
        mirror: WriteSignal<mascot::State>,
    ) -> Self {
        let rambo_world = RamboWorld {
            world: StoredValue::new_local(Rc::new(RefCell::new(World::new(home)))),
            state,
            mirror,
        };
        provide_context(rambo_world);
        rambo_world
    }

    /// The world itself, for the engine's tick and his throws.
    pub(crate) fn world(self) -> Rc<RefCell<World>> {
        self.world.get_value()
    }

    /// Tell him `event`: his new state, now also on the page.
    pub(crate) fn tell(self, event: mascot::Event) -> mascot::State {
        let next = {
            let world = self.world();
            let mut world = world.borrow_mut();
            // Views not yet moved here still write the mirror themselves (8A.5, PR 2):
            // what they wrote is his state before this event.
            world.adopt(self.state.get_untracked());
            world.tell(event)
        };
        self.show(&next);
        next
    }

    /// Put `state`, the world's answer, on the page if the page shows another.
    pub(crate) fn show(self, state: &mascot::State) {
        if *state != self.state.get_untracked() {
            self.mirror.set(state.clone());
        }
    }
}
