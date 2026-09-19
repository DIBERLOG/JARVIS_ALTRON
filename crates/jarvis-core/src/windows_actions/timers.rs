//! Timers and reminders, with one scheduler for the whole application.
//!
//! The rules the implementation follows, and the reasons for them:
//!
//! * **one thread, not one per timer.** The scheduler sleeps until the nearest deadline and
//!   wakes early at least once a second, so a clock change or a new item is picked up without
//!   a thread per countdown, and shutdown is a flag plus a wake-up.
//! * **firing happens once.** An item's status is moved to `Fired` and persisted *before* the
//!   notification is sent, so a crash or a restart in between cannot fire the same reminder
//!   twice, and an item whose time passed while the application was closed fires exactly once
//!   when it comes back.
//! * **the wall clock is not trusted.** Nothing counts down with a monotonic difference: every
//!   wake-up compares the stored `fires_at` with the current wall clock, so moving the system
//!   clock forwards fires the item and moving it backwards delays it instead of losing it.
//! * **the message is sealed.** A reminder text can be personal, so it is sealed with DPAPI for
//!   the current user. Technical timestamps stay readable, because the interface has to show
//!   them while the encrypted storage is locked. A message that cannot be opened (another
//!   user, another machine) does not stop the reminder from firing: it fires without its text.
//! * **no arbitrary action ever happens on fire.** The notification is a local toast plus a
//!   safe sound. There is no hook by which a firing timer could run a command.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};

use super::error::ActionError;
use super::model::{
    ActionSource, TimerId, MAX_ACTIVE_REMINDERS, MAX_ACTIVE_TIMERS, MAX_REMINDER_SECONDS,
    MAX_TIMER_SECONDS, MIN_REMINDER_SECONDS, MIN_TIMER_SECONDS,
};

/// File that holds the scheduled items.
pub const TIMERS_FILE: &str = "timers.json";
/// Schema version of the stored document.
pub const TIMERS_SCHEMA_VERSION: u32 = 1;
/// How often the scheduler wakes even when nothing is due.
pub const SCHEDULER_TICK: Duration = Duration::from_millis(500);

/// What a scheduled item is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledKind {
    Timer,
    Reminder,
}

impl ScheduledKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Timer => "timer",
            Self::Reminder => "reminder",
        }
    }

    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Timer => "windows-kind-timer",
            Self::Reminder => "windows-kind-reminder",
        }
    }
}

/// Where a scheduled item is in its life.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledStatus {
    Pending,
    Fired,
    Cancelled,
}

impl ScheduledStatus {
    pub fn label_key(&self) -> &'static str {
        match self {
            Self::Pending => "windows-scheduled-pending",
            Self::Fired => "windows-scheduled-fired",
            Self::Cancelled => "windows-scheduled-cancelled",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// One stored timer or reminder.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScheduledItem {
    pub id: String,
    pub kind: ScheduledKind,
    pub status: ScheduledStatus,
    pub source: ActionSource,
    pub created_at_unix_ms: u64,
    pub fires_at_unix_ms: u64,
    /// How long the countdown was when it was created.
    pub duration_seconds: u64,
    /// DPAPI-sealed reminder text. `None` for a timer, or when sealing was unavailable.
    #[serde(default)]
    pub sealed_message: Option<Vec<u8>>,
}

impl ScheduledItem {
    pub fn timer_id(&self) -> Result<TimerId, ActionError> {
        TimerId::from_stored(self.id.clone())
    }

    /// When the item fires, as an RFC 3339 string.
    pub fn fires_at_rfc3339(&self) -> String {
        timestamp_from_unix_ms(self.fires_at_unix_ms)
    }

    pub fn created_at_rfc3339(&self) -> String {
        timestamp_from_unix_ms(self.created_at_unix_ms)
    }

    /// Seconds left, or zero when the time has passed.
    pub fn remaining_seconds(&self, now_unix_ms: u64) -> u64 {
        self.fires_at_unix_ms
            .saturating_sub(now_unix_ms)
            .div_ceil(1000)
    }
}

/// A timer or reminder as the interface shows it.
///
/// The reminder text is included: the user typed it and has to be able to read it. It is not
/// included in the audit log and is never sent to a model.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScheduledView {
    pub id: String,
    pub kind: ScheduledKind,
    pub status: ScheduledStatus,
    pub source: ActionSource,
    pub created_at: String,
    pub fires_at: String,
    pub remaining_seconds: u64,
    pub message: Option<String>,
    /// Whether the stored text could not be opened (another user or machine).
    pub message_unreadable: bool,
}

/// The persisted document.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StoredTimers {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    items: Vec<ScheduledItem>,
}

/// The pure state of the scheduler: everything except the thread.
#[derive(Clone, Debug)]
pub struct TimersState {
    path: PathBuf,
    items: Vec<ScheduledItem>,
    min_timer_seconds: u64,
    max_timer_seconds: u64,
    min_reminder_seconds: u64,
    max_reminder_seconds: u64,
    max_timers: usize,
    max_reminders: usize,
}

impl TimersState {
    /// Opens the state from `directory`, restoring everything that was pending.
    pub fn open(directory: &Path) -> Self {
        let path = directory.join(TIMERS_FILE);
        let items = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<StoredTimers>(&text).ok())
            .filter(|stored| stored.schema_version == TIMERS_SCHEMA_VERSION)
            .map(|stored| stored.items)
            .unwrap_or_default();
        Self {
            path,
            items,
            min_timer_seconds: MIN_TIMER_SECONDS,
            max_timer_seconds: MAX_TIMER_SECONDS,
            min_reminder_seconds: MIN_REMINDER_SECONDS,
            max_reminder_seconds: MAX_REMINDER_SECONDS,
            max_timers: MAX_ACTIVE_TIMERS,
            max_reminders: MAX_ACTIVE_REMINDERS,
        }
    }

    /// Overrides the accepted ranges and caps, for tests.
    pub fn with_limits(
        mut self,
        min_timer_seconds: u64,
        max_timer_seconds: u64,
        min_reminder_seconds: u64,
        max_reminder_seconds: u64,
    ) -> Self {
        self.min_timer_seconds = min_timer_seconds;
        self.max_timer_seconds = max_timer_seconds;
        self.min_reminder_seconds = min_reminder_seconds;
        self.max_reminder_seconds = max_reminder_seconds;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn items(&self) -> &[ScheduledItem] {
        &self.items
    }

    /// Creates a timer.
    pub fn create_timer(
        &mut self,
        duration_seconds: u64,
        source: ActionSource,
        now_unix_ms: u64,
    ) -> Result<ScheduledItem, ActionError> {
        if duration_seconds < self.min_timer_seconds || duration_seconds > self.max_timer_seconds {
            return Err(ActionError::InvalidArguments {
                detail: format!(
                    "a timer is {} to {} seconds",
                    self.min_timer_seconds, self.max_timer_seconds
                ),
            });
        }
        if self.active_count(ScheduledKind::Timer) >= self.max_timers {
            return Err(ActionError::Busy);
        }
        let item = ScheduledItem {
            id: mint_id()?,
            kind: ScheduledKind::Timer,
            status: ScheduledStatus::Pending,
            source,
            created_at_unix_ms: now_unix_ms,
            fires_at_unix_ms: now_unix_ms.saturating_add(duration_seconds.saturating_mul(1000)),
            duration_seconds,
            sealed_message: None,
        };
        self.store(item)
    }

    /// Creates a reminder, sealing its text.
    pub fn create_reminder(
        &mut self,
        delay_seconds: u64,
        message: &str,
        source: ActionSource,
        now_unix_ms: u64,
    ) -> Result<ScheduledItem, ActionError> {
        if delay_seconds < self.min_reminder_seconds || delay_seconds > self.max_reminder_seconds {
            return Err(ActionError::InvalidArguments {
                detail: format!(
                    "a reminder is {} to {} seconds away",
                    self.min_reminder_seconds, self.max_reminder_seconds
                ),
            });
        }
        if self.active_count(ScheduledKind::Reminder) >= self.max_reminders {
            return Err(ActionError::Busy);
        }
        let trimmed = message.trim();
        if trimmed.is_empty() || trimmed.chars().count() > super::model::MAX_REMINDER_CHARS {
            return Err(ActionError::InvalidArguments {
                detail: "the reminder text is empty or too long".to_string(),
            });
        }
        let item = ScheduledItem {
            id: mint_id()?,
            kind: ScheduledKind::Reminder,
            status: ScheduledStatus::Pending,
            source,
            created_at_unix_ms: now_unix_ms,
            fires_at_unix_ms: now_unix_ms.saturating_add(delay_seconds.saturating_mul(1000)),
            duration_seconds: delay_seconds,
            // Sealing is best effort: a reminder without DPAPI still fires.
            sealed_message: crate::sync::crypto::dpapi_seal_bytes(trimmed.as_bytes()).ok(),
        };
        self.store(item)
    }

    /// Cancels a pending item.
    pub fn cancel(&mut self, id: &str) -> Result<ScheduledItem, ActionError> {
        let index = self
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or(ActionError::TimerNotFound)?;
        if !self.items[index].status.is_active() {
            return Err(ActionError::TimerNotFound);
        }
        self.items[index].status = ScheduledStatus::Cancelled;
        let item = self.items[index].clone();
        self.save()?;
        Ok(item)
    }

    /// The items that are due now, moved to `Fired` and persisted before they are returned.
    ///
    /// Persisting first is what makes "fires once" true across a crash, a restart, and a
    /// system clock that jumped forward.
    pub fn take_due(&mut self, now_unix_ms: u64) -> Result<Vec<ScheduledItem>, ActionError> {
        let due: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.status.is_active() && item.fires_at_unix_ms <= now_unix_ms)
            .map(|(index, _)| index)
            .collect();
        if due.is_empty() {
            return Ok(Vec::new());
        }
        for index in &due {
            self.items[*index].status = ScheduledStatus::Fired;
        }
        let fired: Vec<ScheduledItem> = due
            .into_iter()
            .map(|index| self.items[index].clone())
            .collect();
        self.save()?;
        Ok(fired)
    }

    /// When the next item is due, if any.
    pub fn next_deadline(&self) -> Option<u64> {
        self.items
            .iter()
            .filter(|item| item.status.is_active())
            .map(|item| item.fires_at_unix_ms)
            .min()
    }

    /// How long the scheduler may sleep before it has to look again.
    pub fn sleep_hint(&self, now_unix_ms: u64) -> Duration {
        match self.next_deadline() {
            Some(deadline) if deadline <= now_unix_ms => Duration::from_millis(0),
            Some(deadline) => {
                let wait = Duration::from_millis(deadline - now_unix_ms);
                wait.min(SCHEDULER_TICK)
            }
            None => SCHEDULER_TICK,
        }
    }

    /// Opens the sealed text of an item.
    pub fn message_of(item: &ScheduledItem) -> (Option<String>, bool) {
        let Some(sealed) = item.sealed_message.as_ref() else {
            return (None, false);
        };
        match crate::sync::crypto::dpapi_open_bytes(sealed) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => (Some(text), false),
                Err(_) => (None, true),
            },
            Err(_) => (None, true),
        }
    }

    /// The interface view of one item.
    ///
    /// The text is opened here and only here, for the interface; the audit never sees it.
    pub fn view_of(item: &ScheduledItem, now_unix_ms: u64) -> ScheduledView {
        let (message, unreadable) = Self::message_of(item);
        ScheduledView {
            id: item.id.clone(),
            kind: item.kind,
            status: item.status,
            source: item.source,
            created_at: item.created_at_rfc3339(),
            fires_at: item.fires_at_rfc3339(),
            remaining_seconds: item.remaining_seconds(now_unix_ms),
            message,
            message_unreadable: unreadable,
        }
    }

    /// The interface view of every item, newest deadline first for the active ones.
    pub fn views(&self, now_unix_ms: u64) -> Vec<ScheduledView> {
        let mut views: Vec<ScheduledView> = self
            .items
            .iter()
            .map(|item| Self::view_of(item, now_unix_ms))
            .collect();
        views.sort_by(|left, right| {
            right
                .status
                .is_active()
                .cmp(&left.status.is_active())
                .then_with(|| left.fires_at.cmp(&right.fires_at))
        });
        views
    }

    /// Forgets fired and cancelled items, keeping the pending ones.
    pub fn prune(&mut self) -> Result<usize, ActionError> {
        let before = self.items.len();
        self.items.retain(|item| item.status.is_active());
        let removed = before - self.items.len();
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    fn active_count(&self, kind: ScheduledKind) -> usize {
        self.items
            .iter()
            .filter(|item| item.kind == kind && item.status.is_active())
            .count()
    }

    fn store(&mut self, item: ScheduledItem) -> Result<ScheduledItem, ActionError> {
        self.items.push(item.clone());
        self.save()?;
        Ok(item)
    }

    fn save(&self) -> Result<(), ActionError> {
        let stored = StoredTimers {
            schema_version: TIMERS_SCHEMA_VERSION,
            items: self.items.clone(),
        };
        let text = serde_json::to_string_pretty(&stored)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fsutil::write_bytes_atomic(&self.path, text.as_bytes())
            .map_err(|_| ActionError::StorageError)
    }
}

/// What the scheduler does when an item fires.
pub type FireSink = Arc<dyn Fn(&ScheduledItem) + Send + Sync>;

/// The scheduler: one thread, one lock, one wake-up condition.
pub struct TimerScheduler {
    state: Arc<(Mutex<TimersState>, Condvar)>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl TimerScheduler {
    /// Opens the state and starts the scheduler thread.
    pub fn start(directory: &Path, on_fire: FireSink) -> Self {
        Self::start_with_state(TimersState::open(directory), on_fire)
    }

    /// Starts with an explicit state, for tests.
    pub fn start_with_state(state: TimersState, on_fire: FireSink) -> Self {
        let state = Arc::new((Mutex::new(state), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("jarvis-windows-timers".to_string())
            .spawn(move || {
                let (lock, condition) = &*thread_state;
                loop {
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = now_unix_ms();
                    let (due, sleep) = {
                        let mut guard = lock.lock();
                        let due = guard.take_due(now).unwrap_or_default();
                        (due, guard.sleep_hint(now))
                    };
                    for item in &due {
                        on_fire(item);
                    }
                    // The wake-up is bounded: a new item, a cancellation, or a changed
                    // system clock is noticed at the next tick at the latest.
                    let mut guard = lock.lock();
                    if sleep.is_zero() {
                        continue;
                    }
                    condition.wait_for(&mut guard, sleep);
                }
            })
            .ok();
        Self {
            state,
            stop,
            handle: Mutex::new(handle),
        }
    }

    /// Runs one iteration of the loop body, for a caller that drives its own thread.
    pub fn run_once(state: &Arc<(Mutex<TimersState>, Condvar)>, on_fire: &FireSink) -> usize {
        let now = now_unix_ms();
        let (due, _) = {
            let mut guard = state.0.lock();
            let due = guard.take_due(now).unwrap_or_default();
            (due, ())
        };
        for item in &due {
            on_fire(item);
        }
        due.len()
    }

    /// The shared state, for the interface and for tests.
    pub fn state(&self) -> Arc<(Mutex<TimersState>, Condvar)> {
        Arc::clone(&self.state)
    }

    /// Runs `action` against the state.
    pub fn with_state<T>(&self, action: impl FnOnce(&mut TimersState) -> T) -> T {
        let mut guard = self.state.0.lock();
        action(&mut guard)
        // The lock is released here; the scheduler wakes on its own tick.
    }

    /// Wakes the scheduler now, so a new item does not wait for the tick.
    pub fn wake(&self) {
        self.state.1.notify_all();
    }

    /// Stops the thread and waits for it, so nothing keeps running after exit.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.state.1.notify_all();
        if let Some(handle) = self.handle.lock().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TimerScheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl std::fmt::Debug for TimerScheduler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TimerScheduler")
            .field("running", &!self.stop.load(Ordering::SeqCst))
            .field("pending", &self.with_state(|state| state.items().len()))
            .finish()
    }
}

/// Current wall clock in milliseconds since the Unix epoch.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// The current wall clock as an RFC 3339 timestamp, used by the audit log.
pub fn now_timestamp() -> String {
    timestamp_from_unix_ms(now_unix_ms())
}

/// Formats a millisecond timestamp as RFC 3339.
pub fn timestamp_from_unix_ms(unix_ms: u64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_millis_opt(unix_ms as i64).single() {
        Some(time) => time.to_rfc3339(),
        None => String::new(),
    }
}

fn mint_id() -> Result<String, ActionError> {
    TimerId::mint().map(|id| id.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tempfile::tempdir;

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn a_timer_counts_down_and_fires_once() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        let timer = state.create_timer(60, ActionSource::Voice, NOW).unwrap();
        assert_eq!(timer.fires_at_unix_ms, NOW + 60_000);
        assert_eq!(timer.remaining_seconds(NOW), 60);
        assert!(state.take_due(NOW + 59_000).unwrap().is_empty());
        let fired = state.take_due(NOW + 60_000).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, timer.id);
        // A second look does not fire it again, even after a clock jump.
        assert!(state.take_due(NOW + 10_000_000).unwrap().is_empty());
    }

    #[test]
    fn a_reminder_keeps_its_text_sealed_and_fires_late_once() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        let reminder = state
            .create_reminder(120, "позвонить в сервис", ActionSource::LocalAi, NOW)
            .unwrap();
        // The stored document does not contain the text in the clear on this platform.
        let stored = std::fs::read_to_string(state.path()).unwrap();
        if cfg!(windows) {
            assert!(!stored.contains("позвонить в сервис"));
            assert!(reminder.sealed_message.is_some());
        }
        let (message, unreadable) = TimersState::message_of(&reminder);
        if cfg!(windows) {
            assert_eq!(message.as_deref(), Some("позвонить в сервис"));
            assert!(!unreadable);
        }

        // The application was closed across the fire time: it fires exactly once on restart.
        let mut reopened = TimersState::open(directory.path());
        let fired = reopened.take_due(NOW + 500_000).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(reopened.take_due(NOW + 500_001).unwrap().is_empty());
    }

    #[test]
    fn cancelling_stops_a_pending_item() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        let timer = state
            .create_timer(300, ActionSource::DirectGui, NOW)
            .unwrap();
        let cancelled = state.cancel(&timer.id).unwrap();
        assert_eq!(cancelled.status, ScheduledStatus::Cancelled);
        assert!(state.take_due(NOW + 400_000).unwrap().is_empty());
        assert_eq!(
            state.cancel(&timer.id).unwrap_err(),
            ActionError::TimerNotFound
        );
        assert_eq!(
            state.cancel("deadbeef").unwrap_err(),
            ActionError::TimerNotFound
        );
    }

    #[test]
    fn ranges_and_caps_are_enforced() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        assert!(state.create_timer(4, ActionSource::DirectGui, NOW).is_err());
        assert!(state
            .create_timer(MAX_TIMER_SECONDS + 1, ActionSource::DirectGui, NOW)
            .is_err());
        assert!(state
            .create_reminder(10, "x", ActionSource::DirectGui, NOW)
            .is_err());
        assert!(state
            .create_reminder(60, "   ", ActionSource::DirectGui, NOW)
            .is_err());
        assert!(state
            .create_reminder(
                60,
                &"x".repeat(super::super::model::MAX_REMINDER_CHARS + 1),
                ActionSource::DirectGui,
                NOW
            )
            .is_err());

        let mut capped = TimersState::open(directory.path());
        for _ in 0..MAX_ACTIVE_TIMERS {
            capped
                .create_timer(60, ActionSource::DirectGui, NOW)
                .unwrap();
        }
        assert_eq!(
            capped
                .create_timer(60, ActionSource::DirectGui, NOW)
                .unwrap_err(),
            ActionError::Busy
        );
    }

    #[test]
    fn the_sleep_hint_follows_the_nearest_deadline() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        assert_eq!(state.next_deadline(), None);
        assert_eq!(state.sleep_hint(NOW), SCHEDULER_TICK);
        state
            .create_timer(60, ActionSource::DirectGui, NOW)
            .unwrap();
        state
            .create_timer(600, ActionSource::DirectGui, NOW)
            .unwrap();
        assert_eq!(state.next_deadline(), Some(NOW + 60_000));
        assert_eq!(state.sleep_hint(NOW), SCHEDULER_TICK);
        // A deadline that is already due asks for an immediate pass.
        assert_eq!(state.sleep_hint(NOW + 60_000), Duration::from_millis(0));
        // Just before the deadline the hint is the remaining time, bounded by the tick.
        assert_eq!(state.sleep_hint(NOW + 59_800), Duration::from_millis(200));
    }

    #[test]
    fn the_views_hide_nothing_but_the_audit_does() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        state.create_timer(60, ActionSource::Voice, NOW).unwrap();
        state
            .create_reminder(120, "выпить воды", ActionSource::DirectGui, NOW)
            .unwrap();
        let views = state.views(NOW);
        assert_eq!(views.len(), 2);
        let reminder = views
            .iter()
            .find(|view| view.kind == ScheduledKind::Reminder)
            .unwrap();
        if cfg!(windows) {
            assert_eq!(reminder.message.as_deref(), Some("выпить воды"));
        }
        assert!(reminder.status.is_active());
        assert_eq!(reminder.remaining_seconds, 120);
        assert!(!reminder.fires_at.is_empty());
    }

    #[test]
    fn pruning_keeps_only_pending_items() {
        let directory = tempdir().unwrap();
        let mut state = TimersState::open(directory.path());
        let fired = state
            .create_timer(60, ActionSource::DirectGui, NOW)
            .unwrap();
        let pending = state
            .create_timer(600, ActionSource::DirectGui, NOW)
            .unwrap();
        state.take_due(NOW + 61_000).unwrap();
        assert_eq!(state.prune().unwrap(), 1);
        assert_eq!(state.items().len(), 1);
        assert_eq!(state.items()[0].id, pending.id);
        assert_ne!(state.items()[0].id, fired.id);
    }

    #[test]
    fn one_thread_fires_a_short_timer_and_shuts_down_cleanly() {
        let directory = tempdir().unwrap();
        let state = TimersState::open(directory.path()).with_limits(0, 600, 0, 600);
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let scheduler = TimerScheduler::start_with_state(
            state,
            Arc::new(move |_item| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        scheduler.with_state(|state| {
            state
                .create_timer(0, ActionSource::InternalTimer, now_unix_ms())
                .unwrap();
        });
        scheduler.wake();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while hits.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // A second wait proves it does not fire again.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        scheduler.shutdown();
        // The state is on disk with the item already marked as fired.
        let reopened = TimersState::open(directory.path());
        assert_eq!(reopened.items().len(), 1);
        assert_eq!(reopened.items()[0].status, ScheduledStatus::Fired);
    }

    #[test]
    fn run_once_is_the_loop_body_without_a_thread() {
        let directory = tempdir().unwrap();
        let state = Arc::new((
            Mutex::new(TimersState::open(directory.path()).with_limits(0, 600, 0, 600)),
            Condvar::new(),
        ));
        {
            let mut guard = state.0.lock();
            guard
                .create_timer(0, ActionSource::InternalTimer, now_unix_ms())
                .unwrap();
        }
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let sink: FireSink = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(TimerScheduler::run_once(&state, &sink), 1);
        assert_eq!(TimerScheduler::run_once(&state, &sink), 0);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
