//! Rambl's brain: what the mascot feels and does, decided by what happened and what
//! time it is. It is a pure function of its own state, so a day can be replayed in a
//! test or printed by the example without a browser.
//!
//! The view never decides anything itself. It calls [`step`] and draws whatever comes
//! back. There is no web, Leptos or server code in this crate.

pub mod data;
pub mod rng;

use chrono::{DateTime, Utc};

pub use data::{DataError, Lines, Personality};
pub use rng::{Rng, Seeded};

/// How talkative Rambl is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// He wanders, plays, complains and chats while you work.
    Interactive,
    /// He answers you and little else: no wandering, and far fewer lines.
    Instant,
    /// No brain at all. He stands there looking vacant, says nothing, feels nothing,
    /// and the page can skip ticking him entirely, so it costs nothing to run.
    Lobotomized,
}

/// What Rambl is doing. The view has one picture for each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Home,
    Listening,
    Thinking,
    Asking,
    Reacting,
    Wandering,
    Dozing,
    Playing,
}

/// Something that happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    TaskAdded,
    TaskDone,
    TaskOverdue,
    ListEmpty,
    /// Minutes since anyone did anything.
    UserIdle(u32),
    Poked,
    Dragged,
    Dropped {
        on_task: String,
    },
    /// Nothing happened; he is being ticked.
    Tick,
    /// The client telling us the local hour, so we never do date maths ourselves.
    LocalHour(u8),
}

/// Something Rambl should do about it.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A named pose for the view to show.
    Pose(&'static str),
    /// A line key such as `task_done/2`. The text is [`Lines::resolve`].
    Say(String),
    MoveTo(Spot),
    Animate(&'static str),
}

/// Where he is, so the view can move him.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spot {
    Circle,
    Window,
    Task(String),
}

/// The three slow feelings, each from 0.0 to 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moods {
    pub energy: f32,
    pub cheer: f32,
    pub boredom: f32,
}

impl Moods {
    fn drift(&mut self, p: &Personality, night: bool) {
        // A night costs him more than a day does, and boredom barely builds asleep.
        let weight = if night { 2.0 } else { 1.0 };
        self.energy = clamp01(self.energy - p.energy_drift * weight);
        self.cheer = clamp01(self.cheer - p.cheer_decay);
        self.boredom = clamp01(self.boredom + p.boredom_drift / weight);
    }
}

/// Steps left before he will do that again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cooldowns {
    pub wander: u32,
    pub play: u32,
    pub idle_line: u32,
}

impl Cooldowns {
    fn tick(&mut self) {
        self.wander = self.wander.saturating_sub(1);
        self.play = self.play.saturating_sub(1);
        self.idle_line = self.idle_line.saturating_sub(1);
    }
}

/// Everything the brain carries between steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Brain {
    pub moods: Moods,
    pub state: State,
    pub mode: Mode,
    /// Local hour, as last told by an [`Event::LocalHour`].
    pub hour: u8,
    /// The line key he said last, so he never says the same one twice running.
    pub last_say: Option<String>,
    /// The task he is asking about, if he is asking.
    pub asking_about: Option<String>,
    pub cooldowns: Cooldowns,
    /// Steps since anything at all happened.
    pub idle_rounds: u32,
}

impl Brain {
    pub fn new(mode: Mode) -> Self {
        Self {
            moods: Moods {
                energy: 0.7,
                cheer: 0.5,
                boredom: 0.2,
            },
            state: State::Home,
            mode,
            hour: 9,
            last_say: None,
            asking_about: None,
            cooldowns: Cooldowns::default(),
            idle_rounds: 0,
        }
    }

    /// True when his current hour is one he dozes through.
    pub fn is_night(&self) -> bool {
        data::personality().is_night(self.hour)
    }
}

/// A hook for something cleverer than the built-in rules: Jev, later. Nothing here
/// calls a network; the brain must stay pure and testable.
pub trait CleverMove {
    /// An idea for what to do instead. `None` means "no idea, carry on".
    fn consider(&mut self, brain: &Brain, event: &Event) -> Option<State>;
}

/// The default: no ideas of its own, so the brain runs on its own rules alone.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCleverMove;

impl CleverMove for NoCleverMove {
    fn consider(&mut self, _brain: &Brain, _event: &Event) -> Option<State> {
        None
    }
}

/// Advance one step and report what he should do about it.
///
/// `event` is what just happened; `None` is the same as [`Event::Tick`].
pub fn step(
    brain: &Brain,
    event: Option<Event>,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
) -> (Brain, Vec<Action>) {
    step_with(brain, event, now, rng, &mut NoCleverMove)
}

/// [`step`], with the clever hook given the last word. A Jev-backed `CleverMove` can
/// override the state the built-in rules would have chosen.
pub fn step_with(
    brain: &Brain,
    event: Option<Event>,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
    clever: &mut impl CleverMove,
) -> (Brain, Vec<Action>) {
    // `now` is the server's stamp on the step. The local hour arrives as an event,
    // because the brain never works out dates itself.
    let _ = now;
    // A lobotomized Rambl has no brain: the only thing he ever does is stand there
    // looking vacant. Nothing is updated, no line is chosen and the clever hook is
    // never consulted, so the page can stop ticking him and this costs nothing.
    if brain.mode == Mode::Lobotomized {
        return (brain.clone(), vec![Action::Pose("lobotomized")]);
    }
    let p = data::personality();
    let mut next = brain.clone();
    let event = event.unwrap_or(Event::Tick);

    if let Event::LocalHour(hour) = &event {
        next.hour = *hour;
    }
    next.cooldowns.tick();
    next.moods.drift(p, next.is_night());

    let mut actions = Vec::new();
    match &event {
        Event::TaskAdded => {
            next.moods.boredom = clamp01(next.moods.boredom - 0.2);
            next.state = State::Thinking;
            next.idle_rounds = 0;
            react(&mut next, "task_added", "think", "nod", rng, &mut actions);
        }
        Event::TaskDone => {
            next.moods.cheer = clamp01(next.moods.cheer + p.cheer_gain);
            next.moods.boredom = clamp01(next.moods.boredom - 0.1);
            next.state = State::Reacting;
            next.idle_rounds = 0;
            react(&mut next, "task_done", "cheer", "bounce", rng, &mut actions);
        }
        Event::TaskOverdue => {
            next.moods.cheer = clamp01(next.moods.cheer - p.cheer_loss);
            next.state = State::Reacting;
            next.idle_rounds = 0;
            react(
                &mut next,
                "task_overdue",
                "worried",
                "frown",
                rng,
                &mut actions,
            );
        }
        Event::ListEmpty => {
            next.moods.cheer = clamp01(next.moods.cheer + p.cheer_gain / 2.0);
            next.state = State::Reacting;
            next.idle_rounds = 0;
            react(&mut next, "list_empty", "smile", "grin", rng, &mut actions);
        }
        Event::Poked => {
            next.moods.cheer = clamp01(next.moods.cheer + 0.05);
            next.asking_about = None;
            next.state = State::Reacting;
            next.idle_rounds = 0;
            // Being poked in the middle of the night is a different thing entirely.
            let situation = if next.is_night() { "wake" } else { "poked" };
            react(&mut next, situation, "look", "wiggle", rng, &mut actions);
        }
        Event::Dragged => {
            next.state = State::Reacting;
            next.idle_rounds = 0;
            react(&mut next, "dragged", "dangle", "swing", rng, &mut actions);
        }
        Event::Dropped { on_task } => {
            next.state = State::Asking;
            next.asking_about = Some(on_task.clone());
            next.idle_rounds = 0;
            react(&mut next, "asking", "ask", "point", rng, &mut actions);
        }
        Event::UserIdle(minutes) => {
            next.idle_rounds = next.idle_rounds.saturating_add(*minutes);
            if next.idle_rounds >= IDLE_AFTER {
                idle(&mut next, p, rng, &mut actions);
            }
        }
        Event::Tick => {
            next.idle_rounds = next.idle_rounds.saturating_add(1);
            if next.idle_rounds >= IDLE_AFTER {
                idle(&mut next, p, rng, &mut actions);
            }
        }
        Event::LocalHour(hour) => {
            let asleep = next.state == State::Dozing;
            if p.is_night(*hour) {
                next.state = State::Dozing;
                next.moods.energy = clamp01(next.moods.energy + 0.05);
                actions.push(Action::Pose("sleep"));
                actions.push(Action::Animate("doze"));
            } else {
                next.state = State::Home;
                actions.push(Action::Pose("home"));
                if asleep {
                    react(&mut next, "wake", "stretch", "wake", rng, &mut actions);
                }
            }
        }
    }
    // The clever hook has the last word, so a Jev-backed implementation can override
    // whatever the built-in rules decided above.
    if let Some(state) = clever.consider(&next, &event) {
        next.state = state;
    }
    (next, actions)
}

/// Idle steps before he starts doing something of his own.
const IDLE_AFTER: u32 = 3;

/// The pose, the line and the cooldown for reacting to something.
fn react(
    brain: &mut Brain,
    situation: &str,
    pose: &'static str,
    animate: &'static str,
    rng: &mut impl Rng,
    actions: &mut Vec<Action>,
) {
    actions.push(Action::Pose(pose));
    actions.push(Action::Animate(animate));
    // Reacting is the whole point of the moment, so it is never rationed away. Only
    // idle chatter is.
    say(brain, situation, rng, actions);
}

/// One thing he could do while nothing is happening.
struct IdleOption {
    state: State,
    /// How much it suits him right now. Anything at or below zero is not worth doing.
    worth: f32,
    situation: &'static str,
    animate: &'static str,
    spot: Spot,
}

/// An idle step: doze at night, otherwise pick something by how well it fits.
fn idle(brain: &mut Brain, p: &Personality, rng: &mut impl Rng, actions: &mut Vec<Action>) {
    if brain.is_night() {
        brain.state = State::Dozing;
        brain.idle_rounds = 0;
        actions.push(Action::Pose("sleep"));
        actions.push(Action::Animate("doze"));
        if brain.cooldowns.idle_line == 0
            && rng.next_f32() < p.sass
            && say(brain, "doze", rng, actions)
        {
            brain.cooldowns.idle_line = p.idle_line_cooldown;
        }
        return;
    }
    if brain.mode == Mode::Instant {
        // Instant Rambl stays put and stays quiet.
        brain.state = State::Home;
        brain.idle_rounds = 0;
        actions.push(Action::Pose("home"));
        actions.push(Action::MoveTo(Spot::Circle));
        return;
    }

    // Utility scoring: each option is worth how well it suits him right now, and is
    // simply unavailable while its cooldown runs.
    let m = brain.moods;
    let mut options = vec![IdleOption {
        state: State::Home,
        worth: 0.6 - m.boredom * 0.7 + m.energy * 0.2,
        situation: "home",
        animate: "idle",
        spot: Spot::Circle,
    }];
    if brain.cooldowns.wander == 0 && m.energy > 0.25 {
        options.push(IdleOption {
            state: State::Wandering,
            worth: 0.5 + m.boredom * 0.8 - m.energy * 0.3,
            situation: "wander",
            animate: "wander",
            spot: Spot::Window,
        });
    }
    if brain.cooldowns.play == 0 && m.cheer > 0.4 {
        options.push(IdleOption {
            state: State::Playing,
            worth: 0.4 + m.cheer * 0.5 + m.boredom * 0.5,
            situation: "play",
            animate: "play",
            spot: Spot::Circle,
        });
    }

    let total: f32 = options.iter().map(|option| option.worth.max(0.0)).sum();
    let mut roll = rng.next_f32() * total;
    let mut chosen = options.len() - 1;
    for (index, option) in options.iter().enumerate() {
        roll -= option.worth.max(0.0);
        if roll <= 0.0 {
            chosen = index;
            break;
        }
    }
    let picked = options.swap_remove(chosen);

    brain.state = picked.state.clone();
    brain.idle_rounds = 0;
    match picked.state {
        State::Wandering => brain.cooldowns.wander = p.wander_cooldown,
        State::Playing => brain.cooldowns.play = p.play_cooldown,
        _ => {}
    }
    actions.push(Action::Pose(picked.situation));
    actions.push(Action::MoveTo(picked.spot));
    actions.push(Action::Animate(picked.animate));
    if brain.cooldowns.idle_line == 0
        && rng.next_f32() < p.sass
        && say(brain, picked.situation, rng, actions)
    {
        brain.cooldowns.idle_line = p.idle_line_cooldown;
    }
}

/// Choose a line for `situation`, never the one he just said, and queue it. Returns
/// whether anything was said.
fn say(brain: &mut Brain, situation: &str, rng: &mut impl Rng, actions: &mut Vec<Action>) -> bool {
    let lines = data::lines();
    let available = lines.all(situation);
    if available.is_empty() {
        return false;
    }
    let keys: Vec<String> = (0..available.len())
        .map(|index| format!("{situation}/{index}"))
        .collect();
    let fresh: Vec<&String> = keys
        .iter()
        .filter(|key| brain.last_say.as_deref() != Some(key.as_str()))
        .collect();
    // With one line left and it being the one he just said, he says nothing at all
    // rather than repeating himself.
    let pick = match fresh.as_slice() {
        [] => return false,
        [only] => (*only).clone(),
        many => many[(rng.next_f32() * many.len() as f32) as usize].clone(),
    };
    brain.last_say = Some(pick.clone());
    actions.push(Action::Say(pick));
    true
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u8) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 13, u32::from(hour), 0, 0)
            .unwrap()
    }

    /// Play a list of events out and collect everything he said.
    fn lines_said(mode: Mode, seed: u64, events: &[Option<Event>]) -> Vec<String> {
        let mut brain = Brain::new(mode);
        let mut rng = Seeded::new(seed);
        let mut said = Vec::new();
        for event in events {
            let (next, actions) = step(&brain, event.clone(), at(12), &mut rng);
            for action in actions {
                if let Action::Say(key) = action {
                    said.push(key);
                }
            }
            brain = next;
        }
        said
    }

    #[test]
    fn finishing_a_task_lifts_him() {
        let mut rng = Seeded::new(1);
        let brain = Brain::new(Mode::Interactive);
        let before = brain.moods.cheer;
        let (after, actions) = step(&brain, Some(Event::TaskDone), at(12), &mut rng);
        assert!(
            after.moods.cheer > before,
            "cheer went {before} to {}",
            after.moods.cheer
        );
        assert_eq!(after.state, State::Reacting);
        assert!(actions.iter().any(|a| matches!(a, Action::Say(_))));
    }

    #[test]
    fn an_overdue_task_sinks_him() {
        let mut rng = Seeded::new(1);
        let brain = Brain::new(Mode::Interactive);
        let (after, _) = step(&brain, Some(Event::TaskOverdue), at(12), &mut rng);
        assert!(after.moods.cheer < brain.moods.cheer);
    }

    #[test]
    fn he_dozes_late_at_night_and_stretches_in_the_morning() {
        let mut rng = Seeded::new(1);
        let mut brain = Brain::new(Mode::Interactive);
        for _ in 0..3 {
            let (next, _) = step(&brain, Some(Event::LocalHour(23)), at(23), &mut rng);
            brain = next;
        }
        assert_eq!(brain.state, State::Dozing);
        let (morning, actions) = step(&brain, Some(Event::LocalHour(9)), at(9), &mut rng);
        assert_eq!(morning.state, State::Home);
        assert!(
            actions.iter().any(|a| a == &Action::Animate("wake")),
            "he should stretch on waking"
        );
    }

    #[test]
    fn he_keeps_dodging_while_it_is_night() {
        let mut rng = Seeded::new(8);
        let brain = Brain::new(Mode::Interactive);
        let (mut brain, _) = step(&brain, Some(Event::LocalHour(2)), at(2), &mut rng);
        assert!(brain.is_night());
        assert_eq!(brain.state, State::Dozing);
        for _ in 0..10 {
            let (next, _) = step(&brain, Some(Event::Tick), at(2), &mut rng);
            assert_eq!(next.state, State::Dozing, "he wandered in the night");
            brain = next;
        }
    }

    #[test]
    fn a_line_never_repeats_back_to_back() {
        let events: Vec<Option<Event>> = (0..80).map(|_| Some(Event::Poked)).collect();
        let said = lines_said(Mode::Interactive, 3, &events);
        assert!(said.len() > 20, "only {} lines were said", said.len());
        for pair in said.windows(2) {
            assert_ne!(pair[0], pair[1], "said {pair:?} twice running");
        }
    }

    #[test]
    fn every_line_key_resolves_to_real_text() {
        let events: Vec<Option<Event>> = (0..150)
            .map(|index| {
                Some(match index % 5 {
                    0 => Event::TaskDone,
                    1 => Event::Tick,
                    2 => Event::Poked,
                    3 => Event::TaskOverdue,
                    _ => Event::ListEmpty,
                })
            })
            .collect();
        for key in lines_said(Mode::Interactive, 9, &events) {
            assert!(data::lines().resolve(&key).is_some(), "no text for {key}");
        }
    }

    #[test]
    fn instant_mode_never_wanders_and_stays_quiet() {
        let events: Vec<Option<Event>> = (0..200).map(|_| Some(Event::Tick)).collect();

        let mut instant = Brain::new(Mode::Instant);
        let mut quiet = 0;
        for event in &events {
            let (next, actions) = step(&instant, event.clone(), at(12), &mut Seeded::new(5));
            quiet += actions
                .iter()
                .filter(|action| matches!(action, Action::Say(_)))
                .count();
            assert!(
                !matches!(next.state, State::Wandering | State::Playing),
                "instant mode wandered into {:?}",
                next.state
            );
            instant = next;
        }

        let chatter = lines_said(Mode::Interactive, 5, &events).len();
        assert_eq!(quiet, 0, "instant mode should stay silent when idle");
        assert!(chatter > quiet, "{chatter} lines vs {quiet}");
    }

    #[test]
    fn cooldowns_stop_him_repeating_himself() {
        let mut brain = Brain::new(Mode::Interactive);
        let mut rng = Seeded::new(11);
        let mut wandering = 0;
        for _ in 0..60 {
            let (next, _) = step(&brain, Some(Event::Tick), at(12), &mut rng);
            if next.state == State::Wandering {
                wandering += 1;
            }
            brain = next;
        }
        assert!(wandering > 0, "he never wandered at all");
        assert!(wandering < 60, "the cooldown never slowed him down");
    }

    #[test]
    fn the_same_seed_replays_the_same_day() {
        let events: Vec<Option<Event>> = (0..40).map(|_| Some(Event::Tick)).collect();
        let mut first = Brain::new(Mode::Interactive);
        let mut second = first.clone();
        let mut rng_a = Seeded::new(21);
        let mut rng_b = Seeded::new(21);
        for event in &events {
            let (a, actions_a) = step(&first, event.clone(), at(12), &mut rng_a);
            let (b, actions_b) = step(&second, event.clone(), at(12), &mut rng_b);
            assert_eq!(a, b);
            assert_eq!(actions_a, actions_b);
            first = a;
            second = b;
        }
    }

    #[test]
    fn moods_stay_inside_their_range() {
        let mut brain = Brain::new(Mode::Interactive);
        let mut rng = Seeded::new(2);
        for _ in 0..400 {
            let (next, _) = step(&brain, Some(Event::Tick), at(3), &mut rng);
            for value in [next.moods.energy, next.moods.cheer, next.moods.boredom] {
                assert!((0.0..=1.0).contains(&value), "{value} out of range");
            }
            brain = next;
        }
    }

    #[test]
    fn dropping_him_on_a_task_makes_him_ask_about_it() {
        let mut rng = Seeded::new(4);
        let brain = Brain::new(Mode::Interactive);
        let (after, _) = step(
            &brain,
            Some(Event::Dropped {
                on_task: "Lab report".into(),
            }),
            at(12),
            &mut rng,
        );
        assert_eq!(after.state, State::Asking);
        assert_eq!(after.asking_about.as_deref(), Some("Lab report"));
    }

    #[test]
    fn the_clever_hook_can_override_the_rules() {
        struct AlwaysPlaying;
        impl CleverMove for AlwaysPlaying {
            fn consider(&mut self, _brain: &Brain, _event: &Event) -> Option<State> {
                Some(State::Playing)
            }
        }
        let mut rng = Seeded::new(1);
        let brain = Brain::new(Mode::Instant);
        let (after, _) = step_with(
            &brain,
            Some(Event::TaskDone),
            at(12),
            &mut rng,
            &mut AlwaysPlaying,
        );
        assert_eq!(after.state, State::Playing);
    }

    #[test]
    fn the_default_hook_changes_nothing() {
        let brain = Brain::new(Mode::Interactive);
        let (plain, plain_actions) =
            step(&brain, Some(Event::TaskDone), at(12), &mut Seeded::new(1));
        let (hooked, hooked_actions) = step_with(
            &brain,
            Some(Event::TaskDone),
            at(12),
            &mut Seeded::new(1),
            &mut NoCleverMove,
        );
        assert_eq!(plain, hooked);
        assert_eq!(plain_actions, hooked_actions);
    }

    /// A hook that counts how often it was asked.
    struct CountingClever(u32);
    impl CleverMove for CountingClever {
        fn consider(&mut self, _brain: &Brain, _event: &Event) -> Option<State> {
            self.0 += 1;
            Some(State::Playing)
        }
    }

    #[test]
    fn a_lobotomized_brain_says_nothing_and_goes_nowhere() {
        let events = [
            Some(Event::TaskDone),
            Some(Event::Poked),
            Some(Event::Tick),
            Some(Event::LocalHour(3)),
            Some(Event::Dropped {
                on_task: "Lab report".into(),
            }),
            None,
        ];
        let brain = Brain::new(Mode::Lobotomized);
        for event in events {
            let (_, actions) = step(&brain, event, at(12), &mut Seeded::new(1));
            assert_eq!(actions, vec![Action::Pose("lobotomized")]);
            assert!(!actions.iter().any(|a| matches!(a, Action::Say(_))));
            assert!(!actions.iter().any(|a| matches!(a, Action::MoveTo(_))));
            assert!(!actions.iter().any(|a| matches!(a, Action::Animate(_))));
        }
    }

    #[test]
    fn lobotomized_freezes_him_and_asks_nobody() {
        let brain = Brain::new(Mode::Lobotomized);
        let mut clever = CountingClever(0);
        let (after, _) = step_with(
            &brain,
            Some(Event::TaskDone),
            at(12),
            &mut Seeded::new(1),
            &mut clever,
        );
        assert_eq!(clever.0, 0, "the clever hook should never be consulted");
        assert_eq!(after, brain, "nothing about him should have changed");
    }

    #[test]
    fn switching_back_brings_the_moods_with_him() {
        let mut brain = Brain::new(Mode::Interactive);
        let mut rng = Seeded::new(4);
        for _ in 0..5 {
            let (next, _) = step(&brain, Some(Event::Tick), at(12), &mut rng);
            brain = next;
        }
        let before = brain.moods;

        let mut blank = brain.clone();
        blank.mode = Mode::Lobotomized;
        for _ in 0..20 {
            let (next, _) = step(&blank, Some(Event::Tick), at(12), &mut rng);
            blank = next;
        }
        assert_eq!(blank.moods, before, "moods should be frozen, not flattened");

        blank.mode = Mode::Interactive;
        assert_eq!(blank.moods, before, "his moods should come back with him");
        let (after, _) = step(&blank, Some(Event::Tick), at(12), &mut rng);
        assert_ne!(after.moods, before, "he should start drifting again");
    }

    #[test]
    fn lobotomized_never_emits_speech_or_personality() {
        let brain = Brain::new(Mode::Lobotomized);
        let events = [
            Some(Event::TaskDone),
            Some(Event::TaskOverdue),
            Some(Event::Tick),
            Some(Event::LocalHour(12)),
            Some(Event::Poked),
        ];
        for event in events {
            let (_, actions) = step(&brain, event, at(12), &mut Seeded::new(1));
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Say(_))),
                "lobotomized said something"
            );
            assert!(
                !actions.iter().any(|a| matches!(a, Action::MoveTo(_))),
                "lobotomized moved"
            );
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Animate(_))),
                "lobotomized animated"
            );
            assert_eq!(actions, vec![Action::Pose("lobotomized")]);
        }
    }

    #[test]
    fn instant_mode_never_waits() {
        let events: Vec<Option<Event>> = (0..60).map(|_| Some(Event::Tick)).collect();
        let mut brain = Brain::new(Mode::Instant);
        for event in events {
            let (next, actions) = step(&brain, event.clone(), at(12), &mut Seeded::new(7));
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Say(_))),
                "instant emitted speech"
            );
            assert!(
                !matches!(
                    next.state,
                    State::Wandering | State::Playing | State::Dozing
                ),
                "instant entered wait-like state {:?}",
                next.state
            );
            brain = next;
        }
    }

    #[test]
    fn every_event_kind_has_a_line() {
        let lines = data::lines();
        let situations: std::collections::HashSet<&str> = lines.situations().collect();
        let required = [
            "task_added",
            "task_done",
            "task_overdue",
            "list_empty",
            "poked",
            "dragged",
            "asking",
            "wander",
            "play",
            "home",
            "doze",
            "wake",
        ];
        for s in required {
            assert!(situations.contains(s), "missing situation {s}");
        }
    }

    #[test]
    fn no_line_is_empty_or_over_120_chars() {
        let lines = data::lines();
        for situation in lines.situations() {
            for line in lines.all(situation) {
                assert!(!line.is_empty(), "empty line in {situation}");
                assert!(
                    line.len() <= 120,
                    "line over 120 chars in {situation}: len={}",
                    line.len()
                );
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_actions_for_events() {
        let events = [
            Some(Event::TaskDone),
            Some(Event::Poked),
            Some(Event::TaskOverdue),
        ];
        let mut first = Brain::new(Mode::Interactive);
        let mut second = first.clone();
        let mut rng_a = Seeded::new(99);
        let mut rng_b = Seeded::new(99);
        for event in events {
            let (a, actions_a) = step(&first, event.clone(), at(12), &mut rng_a);
            let (b, actions_b) = step(&second, event.clone(), at(12), &mut rng_b);
            assert_eq!(a, b, "brain diverged with same seed");
            assert_eq!(actions_a, actions_b, "actions diverged with same seed");
            first = a;
            second = b;
        }
    }
}
