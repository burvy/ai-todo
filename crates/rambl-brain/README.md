# rambl-brain

What Rambl, the mascot, **feels and does**.

It is a pure crate: no web, no Leptos, no server, no network. You hand it what happened
and what time it is, and it hands back a new brain and a list of things for the view to
draw. That is the whole contract, and it is what lets a day be replayed in a test or
printed in a terminal.

```rust
use rambl_brain::{Brain, Event, Mode, Seeded, step};

let mut brain = Brain::new(Mode::Interactive);
let mut rng = Seeded::new(1);
let (brain, actions) = step(&brain, Some(Event::TaskDone), chrono::Utc::now(), &mut rng);
```

## Seeing it happen

```sh
cargo run -p rambl-brain --example sim          # one simulated day
cargo run -p rambl-brain --example sim -- 99    # the same day, different seed
cargo test -p rambl-brain
```

The example prints a table of the day: the hour, his state, his three moods, and what he
did about it — every line he said, resolved from its key.

## The shape of it

- **`Brain`** carries three slow moods (`energy`, `cheer`, `boredom`, each `0.0..=1.0`),
  the current `State`, the local `hour`, cooldowns, and the line he said last.
- **`step(brain, event, now, rng) -> (Brain, Vec<Action>)`** is the only entry point.
  `event` is what just happened; `None` means "nothing happened".
- **`Action`s** are `Pose`, `Say`, `MoveTo` and `Animate`. The view has one picture per
  pose and animation, and does no deciding of its own.
- **Idle moments are scored**, not random: wandering, playing and sitting at home each
  have a utility built from his current moods, and a cooldown keeps him from doing the
  same thing over and over.

### States

`Home`, `Listening`, `Thinking`, `Asking`, `Reacting`, `Wandering`, `Dozing`, `Playing`.

### Events

`TaskAdded`, `TaskDone`, `TaskOverdue`, `ListEmpty`, `UserIdle(minutes)`, `Poked`,
`Dragged`, `Dropped { on_task }`, `Tick`, `LocalHour(hour)`.

The client sends `LocalHour` so the brain never does date arithmetic itself — the rule
from the architecture doc that nothing outside the clock module works out what time it
is.

### Modes

- **`Interactive`** — he wanders, plays, complains and chats.
- **`Instant`** — he answers you and little else: never wanders, and stays silent when
  there is nothing to react to.
- **`Lobotomized`** — the settings switch that takes his brain away. He emits one fixed
  `Pose("lobotomized")` and nothing else: no lines, no movement, no mood changes, and
  the `CleverMove` hook is never consulted. The step is inert and cheap, so the page can
  stop ticking him entirely. His moods are **kept, not cleared**, so switching back
  hands him back exactly as he was.

## Editing the lines and his personality

Both files live in `data/` and are compiled into the binary with `include_str!`, so there
is nothing to deploy alongside it and nothing to load at runtime. Change them, rebuild,
see the difference in `sim`.

### `data/lines.toml`

What he can say, by situation. Several lines per key, separated by `|`:

```toml
[lines.task_done]
lines = "There it is.|One down.|Good.|Nice."
```

A line is addressed as `situation/index` — `task_done/0`, `task_done/1` — which is what
`Action::Say` carries. The view turns that key into text with `lines.resolve(key)`. He
never says the same key twice running, so give a situation at least two lines if you
want him to sound alive.

Keep them short and sayable out loud, and no emoji. The situations the brain asks for
are `task_added`, `task_done`, `task_overdue`, `list_empty`, `poked`, `dragged`,
`asking`, `wander`, `play`, `home`, `doze` and `wake`.

### `data/personality.toml`

How he feels and how fast.

| Key | What it does |
|---|---|
| `rates.energy_drift` | how fast energy drains (faster at night) |
| `rates.cheer_decay` | how fast cheer drifts back to flat |
| `rates.boredom_drift` | how fast boredom builds when nothing happens |
| `rates.cheer_gain` / `cheer_loss` | how much finishing or missing a task moves him |
| `cooldowns.wander` / `play` | steps before he'll do that again |
| `cooldowns.idle_line` | steps before he'll chat while idle |
| `cooldowns.react` | steps before he'll react twice running |
| `day.night_start` / `day.night_end` | local hours he dozes through |
| `voice.sass` | `0.0` to `1.0`, how likely an idle moment comes with a line |

## The Jev hook

`CleverMove` is where a Jev-backed brain will plug in: it is consulted first and may
override the state the built-in rules chose.

```rust
pub trait CleverMove {
    fn consider(&mut self, brain: &Brain, event: &Event) -> Option<State>;
}
```

The default `NoCleverMove` returns `None`, so the brain runs on its own rules alone and
the tests stay offline. Use `step_with` instead of `step` to supply one. Nothing here
calls a network — a real implementation belongs behind this trait, not inside the step.