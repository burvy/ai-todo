//! The paper's own date and time pickers, in place of the browser's native ones
//! (which carry browser chrome and follow the browser's locale, not the paper's
//! clock). Each is a handwritten field that opens a small page beneath it.
//!
//! Values travel as the same strings the native inputs used (`YYYY-MM-DD`,
//! `HH:MM`, `YYYY-MM-DDTHH:MM`), so the dialog's saving code and the BUG-7 messages
//! in `tasks::fields::typed_when` read them unchanged.
//!
//! Keyboard: the field is a button; Enter or Space opens the page. On the day grid
//! the arrows move by a day or a week, Page Up / Page Down by a month, Home / End to
//! the week's ends, Enter picks, Escape closes (and only closes the page, not the
//! dialog around it). On the time page Tab walks hours, minutes, am/pm.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveTime, Weekday};
use leptos::ev;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::clock::{self, Clock};
use crate::tuning::MINUTE_STEP;

/// The month the grid shows, as `Mon`-first rows of seven; `None` pads the days of
/// the neighbouring months.
fn month_grid(year: i32, month: u32) -> Vec<Option<NaiveDate>> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Vec::new();
    };
    let lead = first.weekday().num_days_from_monday() as usize;
    let days = std::iter::successors(Some(first), |day| day.succ_opt())
        .take_while(|day| day.month() == month)
        .map(Some);
    let mut cells: Vec<_> = std::iter::repeat_n(None, lead).chain(days).collect();
    cells.resize(cells.len().next_multiple_of(7), None);
    cells
}

/// Where a navigation key moves the day cursor, or `None` for any other key.
fn step_day(from: NaiveDate, key: &str) -> Option<NaiveDate> {
    match key {
        "ArrowLeft" => from.checked_sub_signed(Duration::days(1)),
        "ArrowRight" => from.checked_add_signed(Duration::days(1)),
        "ArrowUp" => from.checked_sub_signed(Duration::days(7)),
        "ArrowDown" => from.checked_add_signed(Duration::days(7)),
        "PageUp" => from.checked_sub_months(Months::new(1)),
        "PageDown" => from.checked_add_months(Months::new(1)),
        "Home" => {
            from.checked_sub_signed(Duration::days(from.weekday().num_days_from_monday().into()))
        }
        "End" => from.checked_add_signed(Duration::days(
            (6 - from.weekday().num_days_from_monday()).into(),
        )),
        _ => None,
    }
}

/// A day as the field writes it: `Tue 14 Oct`.
fn day_label(day: NaiveDate) -> String {
    day.format("%a %-d %b").to_string()
}

/// A minute of the day split for the 12-hour face: hour 1 to 12, minute, pm?
fn twelve_hour_parts(minutes_in_day: u16) -> (u16, u16, bool) {
    let hour = minutes_in_day / 60;
    (
        match hour % 12 {
            0 => 12,
            h => h,
        },
        minutes_in_day % 60,
        hour >= 12,
    )
}

/// The minute of the day a 12-hour face names.
fn from_twelve_hour(hour: u16, minute: u16, pm: bool) -> u16 {
    ((hour % 12) + if pm { 12 } else { 0 }) * 60 + minute
}

/// The minutes the page offers: every `MINUTE_STEP`, plus the task's own minute when
/// it falls between (an imported 9:07 stays pickable and visible).
fn minute_choices(current: Option<u16>) -> Vec<u16> {
    let mut minutes: Vec<u16> = (0..60).step_by(usize::from(MINUTE_STEP)).collect();
    if let Some(minute) = current.filter(|minute| !minutes.contains(minute)) {
        minutes.push(minute);
        minutes.sort_unstable();
    }
    minutes
}

fn parse_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

/// Minutes into the day for an `HH:MM` value.
fn parse_minutes(value: &str) -> Option<u16> {
    let time = NaiveTime::parse_from_str(value, "%H:%M").ok()?;
    Some(chrono::Timelike::hour(&time) as u16 * 60 + chrono::Timelike::minute(&time) as u16)
}

fn hhmm(minutes_in_day: u16) -> String {
    format!("{:02}:{:02}", minutes_in_day / 60, minutes_in_day % 60)
}

/// Splits a `YYYY-MM-DDTHH:MM` value into its date and time halves; a half that
/// is missing is empty.
fn split_datetime(value: &str) -> (String, String) {
    let (day, time) = value.split_once('T').unwrap_or((value, ""));
    (day.to_owned(), time.to_owned())
}

/// Joins the halves back. A lone half stays a lone half, which `typed_when` reports
/// as "That is not a time"; both empty is the empty value ("Pick a time").
pub(crate) fn join_datetime(day: &str, time: &str) -> String {
    if day.is_empty() && time.is_empty() {
        String::new()
    } else {
        format!("{day}T{time}")
    }
}

/// Moves focus to the element in `scope` the page marks as its roving stop, once
/// the browser has laid the page out.
fn focus_roving(scope: NodeRef<leptos::html::Div>) {
    crate::engine::after(0.0, move || {
        if let Some(stop) = scope
            .get_untracked()
            .and_then(|el| el.query_selector("[tabindex='0']").ok().flatten())
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = stop.focus();
        }
    });
}

/// Whether focus leaving the picker closes its page: only when it really went somewhere
/// else. Not while a press inside is under way, and not while the page is rebuilding the
/// thing that had the focus (a month step replaces the day grid, and the browser reports
/// the focused day vanishing as the focus leaving, with nowhere to go).
fn closes_on_focus_leave(to_inside: bool, inside_is_busy: bool) -> bool {
    !to_inside && !inside_is_busy
}

/// Removes the window listeners that end a press, if the picker holds any.
fn remove_release_listeners(
    release: StoredValue<Option<(WindowListenerHandle, WindowListenerHandle)>>,
) {
    if let Some((up, cancel)) = release.try_update_value(Option::take).flatten() {
        up.remove();
        cancel.remove();
    }
}

/// A handwritten field that opens `page` beneath it. Escape or leaving the field
/// closes it; `on_open` runs as it opens (to place the cursor).
#[component]
fn PaperField(
    label: &'static str,
    #[prop(into)] shown: Signal<Option<String>>,
    placeholder: &'static str,
    open: RwSignal<bool>,
    scope: NodeRef<leptos::html::Div>,
    /// True while the page rebuilds what holds the focus (the day grid on a month step).
    rebuilding: RwSignal<bool>,
    children: ChildrenFn,
) -> impl IntoView {
    let field = NodeRef::<leptos::html::Button>::new();
    // A press inside the picker is under way: some browsers (touch ones, Safari) do not
    // focus a tapped button, so the focus leaves for nothing and must not close it.
    let pressing = RwSignal::new(false);
    // A press released outside the picker never reaches its own pointerup, so the window
    // ends it instead. The listeners live only while the page is open.
    let release = StoredValue::new(None::<(WindowListenerHandle, WindowListenerHandle)>);
    Effect::new(move |_| {
        remove_release_listeners(release);
        if open.get() {
            let up = window_event_listener(ev::pointerup, move |_| pressing.set(false));
            let cancel = window_event_listener(ev::pointercancel, move |_| pressing.set(false));
            release.set_value(Some((up, cancel)));
        }
    });
    on_cleanup(move || remove_release_listeners(release));
    // Closing from the page (a pick, Done, Escape) drops the focus with the page it
    // sat on; hand it back to the field. Focus that already moved elsewhere stays.
    Effect::new(move |was_open: Option<bool>| {
        let now_open = open.get();
        if was_open == Some(true) && !now_open {
            let lost = document()
                .active_element()
                .is_none_or(|el| el.tag_name() == "BODY");
            if let (true, Some(field)) = (lost, field.get_untracked()) {
                let _ = field.focus();
            }
        }
        now_open
    });
    view! {
        <div
            class="paper-picker"
            on:pointerdown=move |_| pressing.set(true)
            on:pointerup=move |_| pressing.set(false)
            on:pointercancel=move |_| pressing.set(false)
            on:focusout=move |event| {
                // Focus moving to something outside the picker closes the page.
                let to_inside = event
                    .related_target()
                    .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                    .zip(event.current_target().and_then(|t| t.dyn_into::<web_sys::Node>().ok()))
                    .is_some_and(|(target, root)| root.contains(Some(&target)));
                if closes_on_focus_leave(to_inside, pressing.get_untracked() || rebuilding.get_untracked()) {
                    open.set(false);
                }
            }
            on:keydown=move |event| {
                if event.key() == "Escape" && open.get_untracked() {
                    // Closes this page only: the dialog's own Escape must not fire.
                    event.prevent_default();
                    event.stop_propagation();
                    open.set(false);
                }
            }
        >
            <button
                type="button"
                class="paper-field"
                node_ref=field
                class:empty=move || shown.get().is_none()
                aria-haspopup="dialog"
                aria-expanded=move || open.get().to_string()
                aria-label=label
                on:click=move |_| {
                    open.update(|o| *o = !*o);
                    if open.get_untracked() {
                        focus_roving(scope);
                    }
                }
            >
                {move || shown.get().unwrap_or_else(|| placeholder.to_owned())}
            </button>
            <Show when=move || open.get()>
                <div class="paper-page" role="dialog" aria-label=label node_ref=scope>
                    {children()}
                </div>
            </Show>
        </div>
    }
}

/// A day picker over a `YYYY-MM-DD` value. Writing an empty string clears it.
#[component]
pub fn PaperDate(
    label: &'static str,
    #[prop(into)] value: Signal<String>,
    #[prop(into)] set: Callback<String>,
    /// Where the grid opens when nothing is picked yet (today).
    #[prop(into)]
    start: Signal<NaiveDate>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let scope = crate::slide::arriving();
    let cursor = RwSignal::new(start.get_untracked());
    let rebuilding = RwSignal::new(false);
    // Moves the day cursor; the grid may be rebuilt under the focus, which must not
    // close the page.
    let go = move |day: NaiveDate, refocus: bool| {
        rebuilding.set(true);
        cursor.set(day);
        crate::engine::after(0.0, move || rebuilding.set(false));
        if refocus {
            focus_roving(scope);
        }
    };
    Effect::new(move |_| {
        if open.get() {
            cursor.set(parse_day(&value.get_untracked()).unwrap_or_else(|| start.get_untracked()));
        }
    });
    let shown = Signal::derive(move || parse_day(&value.get()).map(day_label));
    // The grid is rebuilt only when the month changes, so a key press inside a month
    // keeps the focus on a live button.
    let month_shown = Memo::new(move |_| {
        let day = cursor.get();
        (day.year(), day.month())
    });
    let pick = move |day: NaiveDate| {
        set.run(day.to_string());
        open.set(false);
    };

    view! {
        <PaperField label shown placeholder="Pick a day" open scope rebuilding>
            <div class="paper-month">
                <button
                    type="button"
                    class="paper-step"
                    aria-label="Previous month"
                    on:click=move |_| {
                        if let Some(day) = step_day(cursor.get_untracked(), "PageUp") {
                            go(day, false);
                        }
                    }
                >"‹"</button>
                <span class="paper-month-name" aria-live="polite">
                    {move || cursor.get().format("%B %Y").to_string()}
                </span>
                <button
                    type="button"
                    class="paper-step"
                    aria-label="Next month"
                    on:click=move |_| {
                        if let Some(day) = step_day(cursor.get_untracked(), "PageDown") {
                            go(day, false);
                        }
                    }
                >"›"</button>
            </div>
            <div class="paper-weekdays" aria-hidden="true">
                {["M", "T", "W", "T", "F", "S", "S"]
                    .into_iter()
                    .map(|name| view! { <span>{name}</span> })
                    .collect_view()}
            </div>
            <div
                class="paper-days"
                role="grid"
                on:keydown=move |event| {
                    let key = event.key();
                    if let Some(day) = step_day(cursor.get_untracked(), &key) {
                        event.prevent_default();
                        go(day, true);
                    }
                }
            >
                {move || {
                    let (year, month) = month_shown.get();
                    month_grid(year, month)
                        .into_iter()
                        .map(|cell| match cell {
                            None => view! { <span class="paper-blank"></span> }.into_any(),
                            Some(day) => view! {
                                <button
                                    type="button"
                                    class="paper-day"
                                    class:picked=move || parse_day(&value.get()) == Some(day)
                                    class:today=move || day == start.get()
                                    class:weekend=matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
                                    tabindex=move || if day == cursor.get() { "0" } else { "-1" }
                                    aria-label=day.format("%A %-d %B %Y").to_string()
                                    aria-pressed=move || (parse_day(&value.get()) == Some(day)).to_string()
                                    on:click=move |_| pick(day)
                                >{day.day()}</button>
                            }
                            .into_any(),
                        })
                        .collect_view()
                }}
            </div>
            <div class="paper-actions">
                <button type="button" on:click=move |_| pick(start.get_untracked())>"Today"</button>
                <button
                    type="button"
                    on:click=move |_| {
                        set.run(String::new());
                        open.set(false);
                    }
                >"Clear"</button>
            </div>
        </PaperField>
    }
}

/// A time picker over an `HH:MM` value, written in the paper's clock. Writing an
/// empty string clears it, unless the time is not `clearable` (a setting always has one).
#[component]
pub fn PaperTime(
    label: &'static str,
    #[prop(into)] value: Signal<String>,
    #[prop(into)] set: Callback<String>,
    #[prop(into)] clock: Signal<Clock>,
    #[prop(default = true)] clearable: bool,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let scope = NodeRef::new();
    let shown = Signal::derive(move || {
        parse_minutes(&value.get()).map(|minutes| clock::format_time(minutes, clock.get()))
    });
    // A time not yet picked starts the page on 9:00, a working-day opening that
    // lets the first tap on any hour already name a whole time.
    let chosen = move || parse_minutes(&value.get());
    let minutes = move || chosen().unwrap_or(crate::tuning::TIME_PAGE_START);
    let write = move |next: u16| set.run(hhmm(next % (24 * 60)));

    view! {
        <PaperField label shown placeholder="Pick a time" open scope rebuilding=RwSignal::new(false)>
            {move || {
                let twelve = clock.get().twelve_hour();
                let hours: Vec<u16> = if twelve { (1..=12).collect() } else { (0..24).collect() };
                // The minute the page opened on stays on offer when it is off the step.
                let minute_row = minute_choices(chosen().map(|c| c % 60));
                let hour_now = move || {
                    let (hour12, _, _) = twelve_hour_parts(minutes());
                    if twelve { hour12 } else { minutes() / 60 }
                };
                let hour_cell = move |hour: u16| {
                    let name = if twelve { hour.to_string() } else { format!("{hour:02}") };
                    view! {
                        <button
                            type="button"
                            class="paper-tick"
                            class:picked=move || chosen().is_some() && hour == hour_now()
                            tabindex=move || if hour == hour_now() { "0" } else { "-1" }
                            aria-pressed=move || (chosen().is_some() && hour == hour_now()).to_string()
                            on:click=move |_| {
                                let (_, minute, pm) = twelve_hour_parts(minutes());
                                write(if twelve {
                                    from_twelve_hour(hour, minute, pm)
                                } else {
                                    hour * 60 + minute
                                });
                            }
                            on:keydown=roving_keys
                        >{name}</button>
                    }
                };
                let minute_cell = move |m: u16| view! {
                    <button
                        type="button"
                        class="paper-tick"
                        class:picked=move || chosen().is_some_and(|c| m == c % 60)
                        aria-pressed=move || chosen().is_some_and(|c| m == c % 60).to_string()
                        on:click=move |_| write((minutes() / 60) * 60 + m)
                        on:keydown=roving_keys
                    >{format!("{m:02}")}</button>
                };
                let meridiem = move |name: &'static str, pm: bool| view! {
                    <button
                        type="button"
                        class:picked=move || chosen().is_some_and(|c| twelve_hour_parts(c).2 == pm)
                        aria-pressed=move || chosen().is_some_and(|c| twelve_hour_parts(c).2 == pm).to_string()
                        on:click=move |_| {
                            let (hour, minute, _) = twelve_hour_parts(minutes());
                            write(from_twelve_hour(hour, minute, pm));
                        }
                    >{name}</button>
                };
                view! {
                    <div class="paper-hours" aria-label="Hour" role="group">
                        {hours.into_iter().map(hour_cell).collect_view()}
                    </div>
                    <div class="paper-minutes" aria-label="Minute" role="group">
                        {minute_row.into_iter().map(minute_cell).collect_view()}
                    </div>
                    {twelve.then(|| view! {
                        <div class="paper-meridiem" role="group" aria-label="am or pm">
                            {meridiem("am", false)}
                            {meridiem("pm", true)}
                        </div>
                    })}
                    <div class="paper-actions">
                        <button type="button" on:click=move |_| open.set(false)>"Close"</button>
                        {clearable.then(|| view! {
                            <button
                                type="button"
                                on:click=move |_| {
                                    set.run(String::new());
                                    open.set(false);
                                }
                            >"Clear"</button>
                        })}
                    </div>
                }
            }}
        </PaperField>
    }
}

/// Left and right walk the neighbouring tick in a row of hours or minutes.
fn roving_keys(event: leptos::ev::KeyboardEvent) {
    let forward = match event.key().as_str() {
        "ArrowRight" | "ArrowDown" => true,
        "ArrowLeft" | "ArrowUp" => false,
        _ => return,
    };
    event.prevent_default();
    let sibling = event
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| {
            if forward {
                el.next_element_sibling()
            } else {
                el.previous_element_sibling()
            }
        })
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(sibling) = sibling {
        let _ = sibling.focus();
    }
}

/// A group of paper radios: one choice of a few, each a handwritten word with a
/// hand-drawn ring. Arrows move the choice (and the focus) the way native radios do;
/// only the chosen one is a tab stop.
#[component]
pub fn PaperRadios<T>(
    label: &'static str,
    options: Vec<(T, &'static str)>,
    #[prop(into)] value: Signal<T>,
    #[prop(into)] set: Callback<T>,
) -> impl IntoView
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let group = NodeRef::<leptos::html::Div>::new();
    let choices: Vec<T> = options.iter().map(|(choice, _)| *choice).collect();
    let step = move |event: leptos::ev::KeyboardEvent| {
        let delta = match event.key().as_str() {
            "ArrowRight" | "ArrowDown" => 1,
            "ArrowLeft" | "ArrowUp" => choices.len() - 1,
            _ => return,
        };
        event.prevent_default();
        let at = choices
            .iter()
            .position(|choice| *choice == value.get_untracked());
        if let Some(next) = at.map(|at| choices[(at + delta) % choices.len()]) {
            set.run(next);
            focus_roving(group);
        }
    };
    view! {
        <div class="paper-radios" role="radiogroup" aria-label=label node_ref=group on:keydown=step>
            {options
                .into_iter()
                .map(|(choice, name)| view! {
                    <button
                        type="button"
                        role="radio"
                        class="paper-radio"
                        class:picked=move || value.get() == choice
                        aria-checked=move || (value.get() == choice).to_string()
                        tabindex=move || if value.get() == choice { "0" } else { "-1" }
                        on:click=move |_| set.run(choice)
                    >{name}</button>
                })
                .collect_view()}
        </div>
    }
}

/// A day and a time over one `YYYY-MM-DDTHH:MM` value.
#[component]
pub fn PaperDateTime(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] set: Callback<String>,
    #[prop(into)] clock: Signal<Clock>,
    #[prop(into)] start: Signal<NaiveDate>,
) -> impl IntoView {
    let day = Signal::derive(move || split_datetime(&value.get()).0);
    let time = Signal::derive(move || split_datetime(&value.get()).1);
    view! {
        <span class="paper-datetime">
            <PaperDate
                label="Day"
                value=day
                set=move |next: String| set.run(join_datetime(&next, &time.get_untracked()))
                start
            />
            <PaperTime
                label="Time"
                value=time
                set=move |next: String| set.run(join_datetime(&day.get_untracked(), &next))
                clock
            />
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real day")
    }

    #[test]
    fn the_month_grid_starts_on_monday_and_pads_to_whole_weeks() {
        // October 2026 begins on a Thursday and has 31 days: 3 blanks, 31 days, 4 blanks.
        let cells = month_grid(2026, 10);
        assert_eq!(cells.len() % 7, 0);
        assert_eq!(cells.iter().take_while(|c| c.is_none()).count(), 3);
        assert_eq!(cells.iter().flatten().count(), 31);
        assert_eq!(cells[3], Some(day(2026, 10, 1)));
        assert_eq!(cells.last(), Some(&None));
    }

    #[test]
    fn a_month_that_fills_its_weeks_has_no_padding() {
        // February 2027 starts on a Monday and has 28 days.
        let cells = month_grid(2027, 2);
        assert_eq!(cells.len(), 28);
        assert!(cells.iter().all(Option::is_some));
    }

    #[test]
    fn arrows_and_pages_move_the_day_cursor() {
        let from = day(2026, 10, 14); // a Wednesday
        assert_eq!(step_day(from, "ArrowLeft"), Some(day(2026, 10, 13)));
        assert_eq!(step_day(from, "ArrowDown"), Some(day(2026, 10, 21)));
        assert_eq!(step_day(from, "Home"), Some(day(2026, 10, 12)));
        assert_eq!(step_day(from, "End"), Some(day(2026, 10, 18)));
        assert_eq!(step_day(from, "PageDown"), Some(day(2026, 11, 14)));
        assert_eq!(step_day(from, "x"), None);
    }

    #[test]
    fn a_month_page_clamps_to_the_last_day_of_a_shorter_month() {
        assert_eq!(
            step_day(day(2026, 1, 31), "PageDown"),
            Some(day(2026, 2, 28))
        );
    }

    #[test]
    fn the_twelve_hour_face_round_trips_every_minute_of_the_day() {
        for minutes in 0..24 * 60 {
            let (hour, minute, pm) = twelve_hour_parts(minutes);
            assert!((1..=12).contains(&hour));
            assert_eq!(from_twelve_hour(hour, minute, pm), minutes);
        }
    }

    #[test]
    fn midnight_is_twelve_am_and_noon_is_twelve_pm() {
        assert_eq!(twelve_hour_parts(0), (12, 0, false));
        assert_eq!(twelve_hour_parts(12 * 60), (12, 0, true));
        assert_eq!(twelve_hour_parts(13 * 60 + 30), (1, 30, true));
    }

    #[test]
    fn an_off_step_minute_stays_on_offer() {
        assert_eq!(
            minute_choices(Some(30)).len(),
            usize::from(60 / MINUTE_STEP)
        );
        let with_seven = minute_choices(Some(7));
        assert!(with_seven.contains(&7));
        assert!(with_seven.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn only_a_real_move_of_the_focus_elsewhere_closes_the_page() {
        assert!(closes_on_focus_leave(false, false), "focus went elsewhere");
        assert!(!closes_on_focus_leave(true, false), "focus stayed inside");
        assert!(
            !closes_on_focus_leave(false, true),
            "a press or a rebuild inside is under way"
        );
    }

    #[test]
    fn a_datetime_splits_and_joins_without_loss() {
        let value = "2026-10-14T09:30";
        let (d, t) = split_datetime(value);
        assert_eq!((d.as_str(), t.as_str()), ("2026-10-14", "09:30"));
        assert_eq!(join_datetime(&d, &t), value);
    }

    #[test]
    fn a_lone_half_stays_a_lone_half_and_both_empty_is_empty() {
        assert_eq!(join_datetime("2026-10-14", ""), "2026-10-14T");
        assert_eq!(join_datetime("", "09:30"), "T09:30");
        assert_eq!(join_datetime("", ""), "");
        assert_eq!(split_datetime("").0, "");
    }

    #[test]
    fn values_the_pickers_write_are_the_strings_the_native_inputs_wrote() {
        assert_eq!(hhmm(9 * 60 + 5), "09:05");
        assert_eq!(parse_minutes("21:45"), Some(21 * 60 + 45));
        assert_eq!(parse_minutes("nonsense"), None);
    }
}
