use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where playback is, readable from any thread at any moment without
/// waking the playback thread. The engine publishes a snapshot whenever it
/// runs anyway (a decode, a command); between snapshots the position is
/// extrapolated from the wall clock, which is exactly how audio is
/// consumed, and never past what has been decoded. It never runs backwards
/// within a run of playback either: extrapolating can get a little ahead
/// of the output (it starts playing a moment after it's told to), and the
/// next snapshot would pull it back -- it holds still until playback
/// catches up instead.
#[derive(Clone, Default)]
pub struct PlaybackClock(Arc<Mutex<Snapshot>>);

#[derive(Clone, Copy, Default)]
struct Snapshot {
    /// Frames heard when `at` was taken. Negative right after a track
    /// change: the previous track's tail is still queued ahead of this one.
    audible_frames: i64,
    /// Frames of the current track handed to the output so far -- what
    /// playback can't get ahead of.
    decoded_frames: i64,
    frame_rate: u32,
    running: bool,
    at: Option<Instant>,
    duration: Duration,
    /// The furthest position read in this run of playback. A run ends when
    /// fewer frames have been heard than before: a seek back, another
    /// track, a restart.
    floor: Duration,
}

impl PlaybackClock {
    /// What is being heard right now.
    pub fn position(&self) -> Duration {
        self.position_at(Instant::now())
    }

    /// The current track's length (zero if unknown or nothing is loaded).
    pub fn duration(&self) -> Duration {
        self.lock().duration
    }

    fn position_at(&self, now: Instant) -> Duration {
        let mut snapshot = self.lock();
        if snapshot.frame_rate == 0 {
            return Duration::ZERO;
        }
        let mut frames = snapshot.audible_frames;
        if snapshot.running {
            if let Some(at) = snapshot.at {
                let elapsed = now.saturating_duration_since(at).as_secs_f64();
                frames += (elapsed * snapshot.frame_rate as f64) as i64;
                frames = frames.min(snapshot.decoded_frames);
            }
        }
        let position = Duration::from_secs_f64(frames.max(0) as f64 / snapshot.frame_rate as f64)
            .max(snapshot.floor);
        snapshot.floor = position;
        position
    }

    /// Called by the playback thread with its current numbers.
    pub(crate) fn publish(
        &self,
        audible_frames: i64,
        decoded_frames: i64,
        frame_rate: u32,
        running: bool,
        duration: Duration,
    ) {
        let mut snapshot = self.lock();
        let same_run = audible_frames >= snapshot.audible_frames && frame_rate == snapshot.frame_rate;
        *snapshot = Snapshot {
            audible_frames,
            decoded_frames,
            frame_rate,
            running,
            at: Some(Instant::now()),
            duration,
            floor: if same_run { snapshot.floor } else { Duration::ZERO },
        };
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        // A snapshot is always whole: a panic elsewhere can't leave it torn.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrapolates_while_running_but_not_past_what_was_decoded() {
        let clock = PlaybackClock::default();
        clock.publish(44_100, 44_100 + 22_050, 44_100, true, Duration::from_secs(60));
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at), Duration::from_secs(1));
        assert_eq!(clock.position_at(at + Duration::from_millis(250)), Duration::from_millis(1250));
        // Only half a second was decoded past the snapshot.
        assert_eq!(clock.position_at(at + Duration::from_secs(5)), Duration::from_millis(1500));
        assert_eq!(clock.duration(), Duration::from_secs(60));
    }

    #[test]
    fn holds_still_when_paused_and_stays_at_zero_until_a_new_track_is_heard() {
        let clock = PlaybackClock::default();
        clock.publish(44_100, 88_200, 44_100, false, Duration::ZERO);
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at + Duration::from_secs(3)), Duration::from_secs(1));

        // 0.2s of the previous track still queued ahead of this one.
        clock.publish(-8_820, 4_410, 44_100, true, Duration::ZERO);
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at + Duration::from_millis(100)), Duration::ZERO);
        assert_eq!(clock.position_at(at + Duration::from_millis(250)), Duration::from_millis(50));
    }

    #[test]
    fn holds_still_rather_than_running_back_while_the_output_starts() {
        let clock = PlaybackClock::default();
        // 1000 frames a second: a frame is a millisecond. Playing, but the
        // output hasn't taken anything yet.
        clock.publish(0, 1_000, 1_000, true, Duration::ZERO);
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at + Duration::from_millis(8)), Duration::from_millis(8));
        // Still nothing heard: the reading holds instead of going back.
        clock.publish(0, 1_000, 1_000, true, Duration::ZERO);
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at + Duration::from_millis(1)), Duration::from_millis(8));
        // Playback catches up and goes on from there.
        assert_eq!(clock.position_at(at + Duration::from_millis(20)), Duration::from_millis(20));

        // Seeking back starts a new run, free to go back.
        clock.publish(10, 1_000, 1_000, true, Duration::ZERO);
        clock.publish(0, 100, 1_000, false, Duration::ZERO);
        let at = clock.lock().at.unwrap();
        assert_eq!(clock.position_at(at), Duration::ZERO);
    }

    #[test]
    fn nothing_loaded_is_zero() {
        assert_eq!(PlaybackClock::default().position(), Duration::ZERO);
    }
}
