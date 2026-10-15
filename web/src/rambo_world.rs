//! Rambo's one owner (ARCHITECTURE 8A.5): the engine's world holds his state. A view
//! tells it an event, the world answers his new state, and the page's `mascot_state`
//! mirrors that answer. Only this handle writes the mirror (a test below holds it).

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
        let next = self.world().borrow_mut().tell(event);
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

#[cfg(test)]
mod tests {
    /// The world is his one owner: nothing but this handle may write his state. The page
    /// makes the signal and hands its writer here (`lib.rs`), and no view takes a writer.
    #[test]
    fn only_the_world_writes_his_state() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut dirs = vec![src.clone()];
        let mut writers = Vec::new();
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("web/src is readable") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).expect("a source file");
                    // Split so this test does not count itself.
                    let takes = text
                        .matches(concat!("WriteSignal<", "mascot::State>"))
                        .count();
                    let names = text.matches(concat!("set_", "mascot_state")).count();
                    if takes + names > 0 {
                        let name = path.strip_prefix(&src).expect("under src");
                        writers.push((name.to_string_lossy().replace('\\', "/"), takes, names));
                    }
                }
            }
        }
        writers.sort();
        assert_eq!(
            writers,
            vec![
                ("lib.rs".to_owned(), 0, 2),
                ("rambo_world.rs".to_owned(), 2, 0)
            ],
            "(file, writer types, writer names)"
        );
    }
}
