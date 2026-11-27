//! Backing off when the computer is at its limit -- a demanding game running,
//! say: the visualizations and the turning record then draw at a lower frame
//! rate for a while, so Openbar5000 asks for less of the CPU, the GPU and
//! memory, and goes back up once things have calmed down.
//!
//! "At its limit" is told from three signs, any of which counts:
//! - the UI's own timer ticking late, again and again: the CPU is too busy
//!   to run it on time;
//! - frames taking far longer to draw than they're due every: the GPU is;
//! - little memory left on the system.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The frame rates backed off to, step by step.
const STEPS: [u32; 2] = [30, 15];
/// How many recent ticks and frames are weighed.
const WINDOW: usize = 90;
/// A tick later than this share of its interval counts as late.
const LATE: f32 = 0.6;
/// Backing off takes this many signs among the recent ticks or frames.
const STRAINED_SHARE: f32 = 0.35;
/// Least time between two steps down.
const STEP_DOWN_AFTER: Duration = Duration::from_secs(3);
/// How long it has to stay calm to go a step back up -- at first: a step
/// up that the strain soon undoes doubles it, up to `MAX_STEP_UP_AFTER`, so
/// it doesn't keep trying while a game runs.
const STEP_UP_AFTER: Duration = Duration::from_secs(20);
const MAX_STEP_UP_AFTER: Duration = Duration::from_secs(300);
/// A step down this soon after a step up undoes it.
const UNDONE_WITHIN: Duration = Duration::from_secs(30);
/// How often the system's free memory is looked at.
const MEMORY_CHECK_EVERY: Duration = Duration::from_secs(5);
/// Below this share of the system's memory free, it's short of memory.
const LOW_MEMORY: f64 = 0.06;

pub struct LoadGuard {
    /// 0: no limit; 1, 2: `STEPS[level - 1]`.
    level: Cell<usize>,
    last_change: Cell<Instant>,
    /// Whether the last change was a step up.
    stepped_up: Cell<bool>,
    calm_since: Cell<Instant>,
    /// How long it now has to stay calm to step up.
    calm_needed: Cell<Duration>,
    /// When the poll last ticked, and the interval it was set to then.
    last_tick: Cell<Option<(Instant, Duration)>>,
    /// Recent ticks: whether each was late.
    ticks: RefCell<VecDeque<bool>>,
    /// Recent frames: whether each was slow.
    frames: RefCell<VecDeque<bool>>,
    frame_started: Cell<Option<Instant>>,
    /// The frame rate being drawn at, to tell a slow frame by.
    frame_budget: Cell<Duration>,
    memory_low: Cell<bool>,
    memory_checked: Cell<Option<Instant>>,
}

impl Default for LoadGuard {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            level: Cell::new(0),
            last_change: Cell::new(now),
            stepped_up: Cell::new(false),
            calm_since: Cell::new(now),
            calm_needed: Cell::new(STEP_UP_AFTER),
            last_tick: Cell::new(None),
            ticks: RefCell::new(VecDeque::with_capacity(WINDOW)),
            frames: RefCell::new(VecDeque::with_capacity(WINDOW)),
            frame_started: Cell::new(None),
            frame_budget: Cell::new(Duration::from_millis(16)),
            memory_low: Cell::new(false),
            memory_checked: Cell::new(None),
        }
    }
}

fn push(recent: &RefCell<VecDeque<bool>>, sign: bool) {
    let mut recent = recent.borrow_mut();
    if recent.len() == WINDOW {
        recent.pop_front();
    }
    recent.push_back(sign);
}

fn share(recent: &RefCell<VecDeque<bool>>) -> Option<f32> {
    let recent = recent.borrow();
    // Too few to tell yet.
    (recent.len() >= WINDOW / 3).then(|| recent.iter().filter(|&&sign| sign).count() as f32 / recent.len() as f32)
}

impl LoadGuard {
    /// The poll ticked, its timer set to `interval`. `seen`: whether the
    /// window can be seen -- while it can't, the poll is slow on purpose,
    /// and nothing is drawn to weigh.
    pub fn tick(&self, now: Instant, interval: Duration, seen: bool) {
        if !seen {
            self.last_tick.set(None);
            return;
        }
        if let Some((last, last_interval)) = self.last_tick.replace(Some((now, interval))) {
            // Only between ticks at the same interval: a change of rate
            // isn't lateness.
            if last_interval == interval && !interval.is_zero() {
                let took = now.saturating_duration_since(last);
                let late = took.saturating_sub(interval).as_secs_f32() / interval.as_secs_f32();
                push(&self.ticks, late > LATE);
            }
        }
    }

    /// The frame rate being drawn at, for telling a slow frame.
    pub fn set_frame_rate(&self, fps: u32) {
        self.frame_budget.set(Duration::from_micros(1_000_000 / u64::from(fps.max(1))));
    }

    pub fn frame_started(&self) {
        self.frame_started.set(Some(Instant::now()));
    }

    pub fn frame_finished(&self) {
        if let Some(started) = self.frame_started.take() {
            // Slow: more than twice its share of time, and noticeably long.
            let took = started.elapsed();
            let budget = self.frame_budget.get();
            push(&self.frames, took > budget * 2 && took > Duration::from_millis(12));
        }
    }

    /// Weighs the signs; `Some(new limit)` when the frame rate limit
    /// changes (`None` inside: no limit any more).
    pub fn evaluate(&self, now: Instant) -> Option<Option<u32>> {
        if self.memory_checked.get().is_none_or(|at| now.saturating_duration_since(at) >= MEMORY_CHECK_EVERY) {
            self.memory_checked.set(Some(now));
            let low = free_memory_share().is_some_and(|free| free < LOW_MEMORY);
            if low != self.memory_low.replace(low) {
                tracing::info!("system memory {}", if low { "low" } else { "fine again" });
            }
        }
        let strained = self.memory_low.get()
            || share(&self.ticks).is_some_and(|late| late > STRAINED_SHARE)
            || share(&self.frames).is_some_and(|slow| slow > STRAINED_SHARE);
        let level = self.level.get();
        let since_change = now.saturating_duration_since(self.last_change.get());
        if strained {
            self.calm_since.set(now);
            if level < STEPS.len() && since_change >= STEP_DOWN_AFTER {
                if self.stepped_up.get() && since_change < UNDONE_WITHIN {
                    self.calm_needed.set((self.calm_needed.get() * 2).min(MAX_STEP_UP_AFTER));
                }
                self.stepped_up.set(false);
                return Some(self.set_level(level + 1, now));
            }
        } else if level > 0 && now.saturating_duration_since(self.calm_since.get()) >= self.calm_needed.get() {
            self.calm_since.set(now);
            self.stepped_up.set(true);
            return Some(self.set_level(level - 1, now));
        } else if level == 0 && now.saturating_duration_since(self.calm_since.get()) >= MAX_STEP_UP_AFTER {
            // Long calm: the patience needed starts over.
            self.calm_needed.set(STEP_UP_AFTER);
        }
        None
    }

    fn set_level(&self, level: usize, now: Instant) -> Option<u32> {
        self.level.set(level);
        self.last_change.set(now);
        // The signs so far were at the old rate.
        self.ticks.borrow_mut().clear();
        self.frames.borrow_mut().clear();
        let limit = self.limit();
        match limit {
            Some(fps) => tracing::warn!("the system is busy: drawing at {fps} fps for now"),
            None => tracing::info!("the system is fine again: drawing at the chosen frame rate"),
        }
        limit
    }

    /// The frame rate limit in force, if any.
    pub fn limit(&self) -> Option<u32> {
        self.level.get().checked_sub(1).map(|step| STEPS[step])
    }

    /// `fps` within the limit in force.
    pub fn capped(&self, fps: u32) -> u32 {
        self.limit().map_or(fps, |limit| fps.min(limit))
    }
}

/// The share of the system's memory that's free (0 to 1), if it can be told.
#[cfg(target_os = "linux")]
fn free_memory_share() -> Option<f64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| {
        info.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
    };
    let (total, available) = (field("MemTotal:")?, field("MemAvailable:")?);
    (total > 0.0).then(|| available / total)
}

#[cfg(windows)]
fn free_memory_share() -> Option<f64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: a plain call, given a struct with its size set.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    (status.ullTotalPhys > 0).then(|| status.ullAvailPhys as f64 / status.ullTotalPhys as f64)
}

/// macOS keeps its own measure of memory pressure: the share left, 0-100.
#[cfg(target_os = "macos")]
fn free_memory_share() -> Option<f64> {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: a plain call, given a buffer of the size it's told.
    let ok = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_level".as_ptr(),
            (&mut level as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then(|| f64::from(level) / 100.0)
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn free_memory_share() -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_ticks_step_down_and_calm_steps_back_up() {
        let guard = LoadGuard::default();
        let start = Instant::now() + STEP_DOWN_AFTER;
        let interval = Duration::from_millis(16);
        // Every tick twice as late as its interval.
        let mut now = start;
        for _ in 0..WINDOW {
            guard.tick(now, interval, true);
            now += interval * 3;
        }
        assert_eq!(guard.evaluate(now), Some(Some(30)));
        assert_eq!(guard.capped(60), 30);
        assert_eq!(guard.capped(15), 15);
        // Still late: down again, but not at once.
        for _ in 0..WINDOW {
            guard.tick(now, interval, true);
            now += interval * 3;
        }
        assert_eq!(guard.evaluate(now), Some(Some(15)));
        // On time for long enough: back up a step at a time.
        for _ in 0..WINDOW {
            guard.tick(now, interval, true);
            now += interval;
        }
        assert_eq!(guard.evaluate(now), None);
        now += STEP_UP_AFTER;
        assert_eq!(guard.evaluate(now), Some(Some(30)));
        now += STEP_UP_AFTER;
        assert_eq!(guard.evaluate(now + Duration::from_millis(1)), Some(None));
        assert_eq!(guard.capped(60), 60);
    }

    #[test]
    fn a_step_up_undone_at_once_makes_the_next_wait_longer() {
        let guard = LoadGuard::default();
        let interval = Duration::from_millis(16);
        let mut now = Instant::now() + STEP_DOWN_AFTER;
        let strain = |now: &mut Instant| {
            for _ in 0..WINDOW {
                guard.tick(*now, interval, true);
                *now += interval * 3;
            }
        };
        let calm = |now: &mut Instant| {
            for _ in 0..WINDOW {
                guard.tick(*now, interval, true);
                *now += interval;
            }
        };
        strain(&mut now);
        assert_eq!(guard.evaluate(now), Some(Some(30)));
        calm(&mut now);
        now += STEP_UP_AFTER;
        assert_eq!(guard.evaluate(now), Some(None));
        // Strained again right away: back down, and now it waits longer.
        now += STEP_DOWN_AFTER;
        strain(&mut now);
        assert_eq!(guard.evaluate(now), Some(Some(30)));
        calm(&mut now);
        now += STEP_UP_AFTER;
        assert_eq!(guard.evaluate(now), None);
        now += STEP_UP_AFTER;
        assert_eq!(guard.evaluate(now), Some(None));
    }

    #[test]
    fn a_hidden_window_or_a_new_rate_isnt_lateness() {
        let guard = LoadGuard::default();
        let mut now = Instant::now();
        for i in 0..WINDOW {
            // Alternating rates, and hidden now and then.
            let interval = Duration::from_millis(if i % 2 == 0 { 16 } else { 250 });
            guard.tick(now, interval, i % 7 != 0);
            now += Duration::from_millis(250);
        }
        assert!(share(&guard.ticks).is_none_or(|late| late == 0.0));
    }

    #[test]
    fn free_memory_reads_where_it_can() {
        if let Some(free) = free_memory_share() {
            assert!((0.0..=1.0).contains(&free));
        }
    }
}
