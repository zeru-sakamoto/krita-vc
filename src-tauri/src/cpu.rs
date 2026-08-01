//! CPU headroom: how much of the machine the engine is allowed to take.
//!
//! Everything else in this crate is tuned for throughput. That's the wrong default on the
//! 2-4 core laptops artists actually use: rayon's *global* pool is sized to `num_cpus`, so a
//! commit or a diff pins every logical core at normal priority — while Krita (which is often
//! the very thing that triggered the commit, via the plugin) and the browser starve.
//!
//! Two knobs, both applied here so no call site has to think about it:
//!
//! - **Thread count** — we build our own pool rather than `build_global()`, because the global
//!   pool can only be initialized once and the budget is a live setting.
//! - **Priority** — worker threads are born below-normal, once, in `start_handler`. This does
//!   most of the real work: the OS scheduler then always preempts us in favour of Krita.
//!
//! `commands::run` installs this pool around every Tauri command, and nested `par_iter`s
//! inherit the installing pool — so the whole engine is covered by that one wrap.

use std::sync::{Arc, OnceLock, RwLock};

/// Percent of logical cores the engine may use. Priority lowering carries most of the load;
/// this is the belt to that suspenders, which is why the default isn't more aggressive.
pub const DEFAULT_BUDGET: u8 = 75;

/// The live pool plus the budget it was built for, so `set_budget` can no-op on a repeat.
type Current = Option<(u8, Arc<rayon::ThreadPool>)>;

fn slot() -> &'static RwLock<Current> {
    static POOL: OnceLock<RwLock<Current>> = OnceLock::new();
    POOL.get_or_init(|| RwLock::new(None))
}

/// Logical cores for `percent`, never zero.
fn threads_for(percent: u8) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores * percent as usize / 100).max(1)
}

#[cfg(windows)]
fn lower_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // Best-effort: a failure here costs headroom, not correctness.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

// Upgrade path if this ever ships beyond Windows: `libc::nice(5)` on the worker thread
// (per-thread on Linux, per-process on macOS — which is why it isn't a one-liner).
#[cfg(not(windows))]
fn lower_priority() {}

/// Drop the **whole process** below normal priority.
///
/// For the `kvc` CLI, which the Krita plugin spawns *inside Krita's own process tree* while
/// Krita is painting — so unlike the desktop app it has no idle moment to work in, and its
/// main thread does real engine work too (not just the rayon workers `lower_priority` covers).
/// The desktop app deliberately does not call this: its UI thread should stay responsive.
#[cfg(windows)]
pub fn lower_process_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS,
    };
    // Best-effort, same as lower_priority: losing this costs headroom, not correctness.
    unsafe {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

#[cfg(not(windows))]
pub fn lower_process_priority() {}

/// How many heavy operations may run at once.
///
/// Cores are bounded by the pool above; **memory is not**. Every concurrent heavy op carries
/// its own `RESTORE_CHUNK_BUDGET` (64 MB) of in-flight decode buffers, and the frontend can
/// start them faster than they finish: cancelling a diff in the UI only stops JS from
/// *listening*, so clicking quickly through history used to stack unbounded backend work.
/// Two permits covers the normal "one view in flight, one incoming" case with no added latency.
const HEAVY_PERMITS: usize = 2;

/// Acquire a slot for a heavy operation, waiting if `HEAVY_PERMITS` are already out.
/// Dropping the returned permit releases it. Always taken *outside* `RepoLock`, so there is
/// no lock-order hazard.
pub async fn heavy_permit() -> tokio::sync::OwnedSemaphorePermit {
    static SEM: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(HEAVY_PERMITS)))
        .clone()
        .acquire_owned()
        .await
        .expect("heavy semaphore never closed")
}

fn build(percent: u8) -> Arc<rayon::ThreadPool> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads_for(percent))
        .thread_name(|i| format!("kvc-worker-{i}"))
        .start_handler(|_| lower_priority())
        .build()
        .expect("rayon pool");
    Arc::new(pool)
}

/// The current pool, building it at the default budget on first use.
fn pool() -> Arc<rayon::ThreadPool> {
    if let Some((_, p)) = slot().read().unwrap().as_ref() {
        return p.clone();
    }
    let mut guard = slot().write().unwrap();
    // Another thread may have won the race between the read and the write.
    if let Some((_, p)) = guard.as_ref() {
        return p.clone();
    }
    let p = build(DEFAULT_BUDGET);
    *guard = Some((DEFAULT_BUDGET, p.clone()));
    p
}

/// Resize the pool. In-flight work keeps running on the old pool, which drops with its last
/// `Arc` — so this is safe to call mid-operation and takes effect on the next one.
pub fn set_budget(percent: u8) {
    let percent = percent.clamp(10, 100);
    let mut guard = slot().write().unwrap();
    if guard.as_ref().is_some_and(|(p, _)| *p == percent) {
        return; // Rebuilding an identical pool would just churn threads.
    }
    *guard = Some((percent, build(percent)));
}

/// Run `f` on the budgeted pool. Nested `par_iter`s inside `f` inherit it.
pub fn install<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    pool().install(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::prelude::*;

    #[test]
    fn budget_bounds_thread_count() {
        assert_eq!(threads_for(0), 1, "never zero threads");
        assert!(threads_for(100) >= threads_for(50));
        assert!(threads_for(100) >= 1);
    }

    #[test]
    fn install_runs_nested_parallel_work_on_our_pool() {
        set_budget(50);
        // Nested par_iter — the shape the diff and commit paths actually use. If `install`
        // didn't cover nesting, the inner iterator would land on rayon's global pool.
        let sum: usize = install(|| {
            (0..8usize)
                .into_par_iter()
                .map(|i| {
                    (0..8usize)
                        .into_par_iter()
                        .map(|j| {
                            assert!(
                                rayon::current_thread_index().is_some(),
                                "must run on a rayon worker"
                            );
                            i * j
                        })
                        .sum::<usize>()
                })
                .sum()
        });
        assert_eq!(sum, (0..8).map(|i: usize| i * 28).sum::<usize>());

        // Resizing mid-life must not poison the pool.
        set_budget(100);
        assert_eq!(install(|| (0..100u32).into_par_iter().sum::<u32>()), 4950);
    }
}
