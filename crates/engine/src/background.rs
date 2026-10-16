//! Bulk background work -- decoding a whole track for its waveform, reading
//! the tags of a folder full of files -- runs on every spare core in short
//! bursts. Its threads mark themselves as background work, so a burst
//! yields the CPU to the UI, the audio and the user's other programs
//! instead of competing with them as equals.

/// How much nicer than the rest of the app background threads are.
#[cfg(target_os = "linux")]
const BACKGROUND_NICENESS: libc::c_int = 10;

/// Lowers the calling thread's CPU priority, for the rest of its life. Call
/// it only from threads doing bulk background work. Linux sets niceness
/// per thread; elsewhere this does nothing.
pub fn lower_thread_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread's own id. Raising
    // niceness needs no privileges; a failure just leaves it as it was.
    unsafe {
        let tid = libc::gettid() as libc::id_t;
        let current = libc::getpriority(libc::PRIO_PROCESS as _, tid);
        libc::setpriority(libc::PRIO_PROCESS as _, tid, (current + BACKGROUND_NICENESS).min(19));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn makes_only_the_calling_thread_nicer() {
        let niceness = || unsafe { libc::getpriority(libc::PRIO_PROCESS as _, libc::gettid() as libc::id_t) };
        let before = niceness();
        let lowered = std::thread::spawn(move || {
            lower_thread_priority();
            niceness()
        })
        .join()
        .unwrap();
        assert_eq!(lowered, (before + BACKGROUND_NICENESS).min(19));
        assert_eq!(niceness(), before);
    }
}
