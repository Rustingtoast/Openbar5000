//! Backing off when the computer is at its limit -- a demanding game running,
//! say: the visualizations and the turning record then draw at a lower frame
//! rate for a while, so Openbar5000 asks for less of the GPU and the CPU, and
//! goes back up once things have calmed down.
//!
//! "At its limit" is read from the system itself (see `resources`), over
//! the last few seconds: the GPU at `GPU_BUSY` or more -- the one that
//! matters most, as the GPU is what a game and the visualizations share --
//! the CPU at `CPU_BUSY` or more, or little memory left.

use crate::resources::{Sample, Usage};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The frame rates backed off to, step by step.
const STEPS: [u32; 2] = [30, 15];
/// GPU use (%) counting as at its limit.
const GPU_BUSY: f32 = 80.0;
/// CPU use (%) counting as at its limit.
const CPU_BUSY: f32 = 90.0;
/// Below this share of memory free (%), it's short of memory.
const LOW_MEMORY: f32 = 6.0;
/// How many samples (one every two seconds) are averaged: a moment's
/// spike isn't strain.
const WINDOW: usize = 3;
/// Least time between two steps down.
const STEP_DOWN_AFTER: Duration = Duration::from_secs(4);
/// How long it has to stay calm to go a step back up -- at first: a step
/// up that the strain soon undoes doubles it, up to `MAX_STEP_UP_AFTER`, so
/// it doesn't keep trying while a game runs.
const STEP_UP_AFTER: Duration = Duration::from_secs(20);
const MAX_STEP_UP_AFTER: Duration = Duration::from_secs(300);
/// A step down this soon after a step up undoes it.
const UNDONE_WITHIN: Duration = Duration::from_secs(30);

pub struct LoadGuard {
    usage: Option<Arc<Usage>>,
    /// The last sample taken in.
    seen: Cell<u64>,
    samples: RefCell<VecDeque<Sample>>,
    /// 0: no limit; 1, 2: `STEPS[level - 1]`.
    level: Cell<usize>,
    last_change: Cell<Instant>,
    /// Whether the last change was a step up.
    stepped_up: Cell<bool>,
    calm_since: Cell<Instant>,
    /// How long it now has to stay calm to step up.
    calm_needed: Cell<Duration>,
}

impl LoadGuard {
    /// Watching `usage` (sampled by `resources::start`); with none, it never
    /// backs off.
    pub fn new(usage: Option<Arc<Usage>>) -> Self {
        let now = Instant::now();
        Self {
            usage,
            seen: Cell::new(0),
            samples: RefCell::new(VecDeque::with_capacity(WINDOW)),
            level: Cell::new(0),
            last_change: Cell::new(now),
            stepped_up: Cell::new(false),
            calm_since: Cell::new(now),
            calm_needed: Cell::new(STEP_UP_AFTER),
        }
    }

    /// Takes in a new sample if there is one, and weighs them; `Some(new
    /// limit)` when the frame rate limit changes (`None` inside: no limit
    /// any more).
    pub fn evaluate(&self, now: Instant) -> Option<Option<u32>> {
        let usage = self.usage.as_ref()?;
        let (count, sample) = usage.latest();
        if count == 0 || count == self.seen.replace(count) {
            return None;
        }
        self.take(sample, now)
    }

    fn take(&self, sample: Sample, now: Instant) -> Option<Option<u32>> {
        {
            let mut samples = self.samples.borrow_mut();
            if samples.len() == WINDOW {
                samples.pop_front();
            }
            samples.push_back(sample);
        }
        let reasons = self.strain();
        let level = self.level.get();
        let since_change = now.saturating_duration_since(self.last_change.get());
        if !reasons.is_empty() {
            self.calm_since.set(now);
            if level < STEPS.len() && since_change >= STEP_DOWN_AFTER {
                if self.stepped_up.get() && since_change < UNDONE_WITHIN {
                    self.calm_needed.set((self.calm_needed.get() * 2).min(MAX_STEP_UP_AFTER));
                }
                self.stepped_up.set(false);
                tracing::warn!("the system is busy: {}", reasons.join(", "));
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

    /// What's at its limit, on average over the samples (none: nothing).
    fn strain(&self) -> Vec<String> {
        let samples = self.samples.borrow();
        if samples.len() < WINDOW {
            return Vec::new();
        }
        let average = |figure: fn(&Sample) -> Option<f32>| {
            let known: Vec<f32> = samples.iter().filter_map(figure).collect();
            (known.len() == samples.len()).then(|| known.iter().sum::<f32>() / known.len() as f32)
        };
        let mut reasons = Vec::new();
        if let Some(gpu) = average(|s| s.gpu_busy).filter(|&gpu| gpu >= GPU_BUSY) {
            reasons.push(format!("GPU {gpu:.0}% busy"));
        }
        if let Some(cpu) = average(|s| s.cpu_busy).filter(|&cpu| cpu >= CPU_BUSY) {
            reasons.push(format!("CPU {cpu:.0}% busy"));
        }
        if let Some(free) = average(|s| s.memory_free).filter(|&free| free < LOW_MEMORY) {
            reasons.push(format!("only {free:.0}% of memory free"));
        }
        reasons
    }

    fn set_level(&self, level: usize, now: Instant) -> Option<u32> {
        self.level.set(level);
        self.last_change.set(now);
        let limit = self.limit();
        match limit {
            Some(fps) => tracing::warn!("drawing at {fps} fps for now"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(gpu: f32, cpu: f32) -> Sample {
        Sample { gpu_busy: Some(gpu), cpu_busy: Some(cpu), memory_free: Some(50.0) }
    }

    /// Feeds `samples`, two seconds apart, from `now`; the last result.
    fn feed(guard: &LoadGuard, now: &mut Instant, samples: &[Sample]) -> Option<Option<u32>> {
        let mut last = None;
        for &s in samples {
            *now += Duration::from_secs(2);
            last = guard.take(s, *now);
        }
        last
    }

    #[test]
    fn a_busy_gpu_steps_down_and_calm_steps_back_up() {
        let guard = LoadGuard::new(None);
        let mut now = Instant::now();
        // A moment's spike isn't strain.
        assert_eq!(feed(&guard, &mut now, &[sample(20.0, 10.0), sample(20.0, 10.0), sample(95.0, 10.0)]), None);
        assert_eq!(feed(&guard, &mut now, &[sample(90.0, 10.0), sample(90.0, 10.0)]), Some(Some(30)));
        assert_eq!(guard.capped(60), 30);
        assert_eq!(guard.capped(15), 15);
        assert_eq!(feed(&guard, &mut now, &[sample(90.0, 10.0), sample(90.0, 10.0)]), Some(Some(15)));
        // Calm for long enough: back up a step at a time.
        let calm = [sample(30.0, 10.0); 10];
        assert_eq!(feed(&guard, &mut now, &calm), Some(Some(30)));
        assert_eq!(feed(&guard, &mut now, &calm), Some(None));
        assert_eq!(guard.capped(60), 60);
    }

    #[test]
    fn a_busy_cpu_counts_from_ninety_percent() {
        let guard = LoadGuard::new(None);
        let mut now = Instant::now();
        assert_eq!(feed(&guard, &mut now, &[sample(10.0, 85.0); 4]), None);
        // The average (85, 95, 95) crosses ninety at the second.
        assert_eq!(feed(&guard, &mut now, &[sample(10.0, 95.0); 2]), Some(Some(30)));
    }

    #[test]
    fn unknown_figures_never_count_as_busy() {
        let guard = LoadGuard::new(None);
        let mut now = Instant::now();
        let unknown = Sample { gpu_busy: None, cpu_busy: None, memory_free: None };
        assert_eq!(feed(&guard, &mut now, &[unknown; 6]), None);
    }

    #[test]
    fn a_step_up_undone_at_once_makes_the_next_wait_longer() {
        let guard = LoadGuard::new(None);
        let mut now = Instant::now();
        let busy = [sample(95.0, 10.0); 3];
        let calm = [sample(10.0, 10.0); 10];
        assert_eq!(feed(&guard, &mut now, &busy), Some(Some(30)));
        assert_eq!(feed(&guard, &mut now, &calm), Some(None));
        // Busy again right away: back down, and now it waits longer.
        assert_eq!(feed(&guard, &mut now, &busy), Some(Some(30)));
        assert_eq!(feed(&guard, &mut now, &calm), None);
        assert_eq!(feed(&guard, &mut now, &calm), Some(None));
    }
}
