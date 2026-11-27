//! How busy the computer is: GPU and CPU use, and free memory, sampled
//! every couple of seconds on a thread of its own (some of it means asking
//! another program), for `load_guard` to read at no cost.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How often the system is sampled.
const EVERY: Duration = Duration::from_secs(2);
/// Stored for a figure that can't be told.
const UNKNOWN: u32 = u32::MAX;

/// The latest sample: percentages (0-100) of the GPU and CPU in use and of
/// memory free, each `None` where it can't be told.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sample {
    pub gpu_busy: Option<f32>,
    pub cpu_busy: Option<f32>,
    pub memory_free: Option<f32>,
}

#[derive(Default)]
pub struct Usage {
    gpu: AtomicU32,
    cpu: AtomicU32,
    memory: AtomicU32,
    /// How many samples there have been, to tell a new one.
    count: AtomicU64,
}

fn store(slot: &AtomicU32, value: Option<f32>) {
    slot.store(value.map_or(UNKNOWN, |v| (v.clamp(0.0, 100.0) * 10.0) as u32), Ordering::Relaxed);
}

fn load(slot: &AtomicU32) -> Option<f32> {
    let value = slot.load(Ordering::Relaxed);
    (value != UNKNOWN).then(|| value as f32 / 10.0)
}

impl Usage {
    /// The latest sample and its number (0: none yet).
    pub fn latest(&self) -> (u64, Sample) {
        let count = self.count.load(Ordering::Acquire);
        let sample = Sample { gpu_busy: load(&self.gpu), cpu_busy: load(&self.cpu), memory_free: load(&self.memory) };
        (count, sample)
    }
}

/// Starts sampling; `None` if no thread could be started for it.
pub fn start() -> Option<Arc<Usage>> {
    let usage = Arc::new(Usage::default());
    let shared = usage.clone();
    let started = crate::platform::spawn("openbar5000-resources", move || {
        let mut cpu = CpuMeter::default();
        let mut gpu = GpuMeter::new();
        loop {
            store(&shared.cpu, cpu.busy());
            store(&shared.gpu, gpu.busy());
            store(&shared.memory, free_memory_share().map(|share| share as f32 * 100.0));
            shared.count.fetch_add(1, Ordering::Release);
            std::thread::sleep(EVERY);
        }
    });
    started.then_some(usage)
}

/// The share of the CPU in use since the last call, from the system's
/// counters of time spent busy and in all.
#[derive(Default)]
struct CpuMeter {
    last: Option<(u64, u64)>,
}

impl CpuMeter {
    fn busy(&mut self) -> Option<f32> {
        let now = cpu_times()?;
        let last = self.last.replace(now)?;
        let (busy, total) = (now.0.saturating_sub(last.0), now.1.saturating_sub(last.1));
        (total > 0).then(|| busy as f32 / total as f32 * 100.0)
    }
}

/// (time busy, time in all), in the system's units, counted since boot.
#[cfg(target_os = "linux")]
fn cpu_times() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal ...
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some((total.saturating_sub(idle), total))
}

#[cfg(windows)]
fn cpu_times() -> Option<(u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetSystemTimes;
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: plain call into three FILETIMEs.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let value = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    // Kernel time includes the idle time.
    let total = value(kernel) + value(user);
    Some((total.saturating_sub(value(idle)), total))
}

#[cfg(target_os = "macos")]
fn cpu_times() -> Option<(u64, u64)> {
    // mach's processor load counters: user, system, idle, nice ticks.
    extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }
    const HOST_CPU_LOAD_INFO: i32 = 3;
    let mut ticks = [0u32; 4];
    let mut count = 4u32;
    // SAFETY: a buffer of the size `count` says.
    let ok = unsafe { host_statistics(mach_host_self(), HOST_CPU_LOAD_INFO, ticks.as_mut_ptr().cast(), &mut count) } == 0;
    if !ok {
        return None;
    }
    let [user, system, idle, nice] = ticks.map(u64::from);
    let total = user + system + idle + nice;
    Some((total - idle, total))
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn cpu_times() -> Option<(u64, u64)> {
    None
}

/// The GPU's use, however the system tells it.
struct GpuMeter {
    #[cfg(windows)]
    counters: Option<windows_gpu::Counters>,
    /// Whether asking the NVIDIA tool is worth it (it's there).
    #[cfg(target_os = "linux")]
    nvidia: bool,
}

impl GpuMeter {
    fn new() -> Self {
        Self {
            #[cfg(windows)]
            counters: windows_gpu::Counters::open(),
            #[cfg(target_os = "linux")]
            nvidia: true,
        }
    }

    /// The busiest GPU's use (0-100).
    #[cfg(target_os = "linux")]
    fn busy(&mut self) -> Option<f32> {
        // AMD: the driver's own figure, one file per card.
        let amd = std::fs::read_dir("/sys/class/drm")
            .ok()?
            .flatten()
            .filter_map(|card| std::fs::read_to_string(card.path().join("device/gpu_busy_percent")).ok())
            .filter_map(|text| text.trim().parse::<f32>().ok())
            .reduce(f32::max);
        if amd.is_some() {
            return amd;
        }
        // NVIDIA: its tool, if installed (once not, never asked again).
        if self.nvidia {
            let asked = std::process::Command::new("nvidia-smi")
                .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
                .output();
            match asked {
                Ok(out) if out.status.success() => {
                    return String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .filter_map(|line| line.trim().parse::<f32>().ok())
                        .reduce(f32::max);
                }
                _ => self.nvidia = false,
            }
        }
        None
    }

    #[cfg(windows)]
    fn busy(&mut self) -> Option<f32> {
        self.counters.as_mut()?.busy()
    }

    /// macOS: the accelerator's own "Device Utilization %", from `ioreg`.
    #[cfg(target_os = "macos")]
    fn busy(&mut self) -> Option<f32> {
        let out = std::process::Command::new("ioreg").args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.match_indices("\"Device Utilization %\"=")
            .filter_map(|(at, key)| {
                let digits: String = text[at + key.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<f32>().ok()
            })
            .reduce(f32::max)
    }

    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    fn busy(&mut self) -> Option<f32> {
        None
    }
}

/// Windows' GPU performance counters: every process's use of the 3D engines,
/// added up.
#[cfg(windows)]
mod windows_gpu {
    use windows::core::w;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    };

    pub struct Counters {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    // The handles are only ever used on the sampling thread they're made on.
    unsafe impl Send for Counters {}

    impl Counters {
        pub fn open() -> Option<Self> {
            let mut query = PDH_HQUERY::default();
            let mut counter = PDH_HCOUNTER::default();
            // SAFETY: plain calls, given places for the handles.
            unsafe {
                if PdhOpenQueryW(None, 0, &mut query) != 0 {
                    return None;
                }
                if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut counter) != 0 {
                    PdhCloseQuery(query);
                    return None;
                }
                // The first collection only starts the measuring.
                PdhCollectQueryData(query);
            }
            Some(Self { query, counter })
        }

        pub fn busy(&mut self) -> Option<f32> {
            // SAFETY: the handles are open; the buffer is sized as asked.
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let (mut size, mut count) = (0u32, 0u32);
                if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None) != PDH_MORE_DATA {
                    return None;
                }
                let items = (size as usize).div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
                let mut buffer = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); items.max(1)];
                if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(buffer.as_mut_ptr())) != 0 {
                    return None;
                }
                let total: f64 = buffer[..count as usize].iter().map(|item| item.FmtValue.Anonymous.doubleValue).sum();
                Some(total.min(100.0) as f32)
            }
        }
    }

    impl Drop for Counters {
        fn drop(&mut self) {
            // SAFETY: closing the query this opened.
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}

/// The share of the system's memory that's free (0 to 1), if it can be told.
#[cfg(target_os = "linux")]
pub fn free_memory_share() -> Option<f64> {
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
pub fn free_memory_share() -> Option<f64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: a plain call, given a struct with its size set.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    (status.ullTotalPhys > 0).then(|| status.ullAvailPhys as f64 / status.ullTotalPhys as f64)
}

/// macOS keeps its own measure of memory pressure: the share left, 0-100.
#[cfg(target_os = "macos")]
pub fn free_memory_share() -> Option<f64> {
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
pub fn free_memory_share() -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_read_where_they_can_and_stay_in_range() {
        let mut cpu = CpuMeter::default();
        assert_eq!(cpu.busy(), None, "the first reading only starts the meter");
        std::thread::sleep(Duration::from_millis(50));
        if let Some(busy) = cpu.busy() {
            assert!((0.0..=100.0).contains(&busy));
        }
        if let Some(busy) = GpuMeter::new().busy() {
            assert!((0.0..=100.0).contains(&busy));
        }
        if let Some(free) = free_memory_share() {
            assert!((0.0..=1.0).contains(&free));
        }
    }

    #[test]
    fn unknown_figures_stay_unknown() {
        let usage = Usage::default();
        store(&usage.gpu, None);
        store(&usage.cpu, Some(42.5));
        store(&usage.memory, Some(150.0));
        let (_, sample) = usage.latest();
        assert_eq!(sample.gpu_busy, None);
        assert_eq!(sample.cpu_busy, Some(42.5));
        assert_eq!(sample.memory_free, Some(100.0));
    }
}
