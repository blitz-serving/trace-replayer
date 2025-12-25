pub mod apis;
pub mod dataset;
pub mod requester;
pub mod token_sampler;

use core::hint::spin_loop;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::yield_now;

use tracing::{instrument, Level};

pub fn timeout_secs_upon_slo(output_length: u64, ttft_slo: f32, tpot_slo: f32) -> u64 {
    15.max((ttft_slo + tpot_slo * output_length as f32) as u64)
}

/// Light weighted spinlock, for extremely short critical section
/// do not abuse it
pub struct SpinLock {
    flag: AtomicBool, // false: unlocked, true: locked
}

unsafe impl Send for SpinLock {}
unsafe impl Sync for SpinLock {}

#[allow(unused)]
impl SpinLock {
    pub const fn new() -> Self {
        Self { flag: AtomicBool::new(false) }
    }

    /// acuire the lock in blocking manner (spin)
    pub fn lock(&self) {
        // test-and-test-and-set + exponential backoff
        let mut spins = 0u32;
        loop {
            // fast path: first "read" to see if it might be unlocked (avoid bus contention from frequent writes)
            while self.flag.load(Ordering::Relaxed) {
                // spin-wait
                spins = backoff(spins);
            }

            // real attempt: try to acquire the lock via CAS
            match self.flag.compare_exchange(
                false,
                true,
                Ordering::Acquire, // Successfully acquired, establish Acquire barrier
                Ordering::Relaxed, // failure can use Relaxed
            ) {
                Ok(_) => break,
                Err(_) => {
                    // if failed, continue backoff
                    spins = backoff(spins);
                }
            }
        }
    }

    #[inline]
    fn unlock(&self) {
        // release the lock: Release ensures that writes to data are visible to subsequent acquirers
        self.flag.store(false, Ordering::Release);
    }
}
/// Exponential backoff: busy-spin for the first few attempts, then proactively yield the CPU time slice.
#[inline]
fn backoff(spins: u32) -> u32 {
    // 64 times before: CPU hint
    if spins < 64 {
        spin_loop();
        spins + 1
    } else {
        // yeild CPU from time to time, avoid starvation
        if spins & 0xF == 0 {
            // yield every 16 spins
            yield_now();
        } else {
            spin_loop();
        }
        spins.saturating_add(1)
    }
}

unsafe impl Send for SpinRwLock {}
unsafe impl Sync for SpinRwLock {}

pub struct SpinRwLock {
    state: AtomicUsize,
}

const USIZE_BITS: u32 = (core::mem::size_of::<usize>() * 8) as u32;
const WRITER_BIT: usize = 1usize << (USIZE_BITS - 1);
const WAITER_BIT: usize = 1usize << (USIZE_BITS - 2);
const READER_MASK: usize = !(WRITER_BIT | WAITER_BIT);

impl SpinRwLock {
    pub const fn new() -> Self {
        Self { state: AtomicUsize::new(0) }
    }

    /// Get read lock, while writer is priorized
    #[instrument(skip_all, level = Level::DEBUG, target = "spin_rwlck::read")]
    pub fn read_lock(&self) {
        let mut spins = 0u32;
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s & (WRITER_BIT | WAITER_BIT) != 0 {
                // can't acquire lock due to writer
                spins += 1;
                backoff(spins);
                continue;
            }
            if self
                .state
                .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                // acquire read lock, add reader counter
                return;
            }
            // can't acquire lock due to writer
            spins += 1;
            backoff(spins);
        }
    }

    pub fn read_unlock(&self) {
        // sub reader counter
        let prev = self.state.fetch_sub(1, Ordering::Release);
        debug_assert!(prev & READER_MASK >= 1);
    }

    /// Get write lock, while writer is priorized
    #[instrument(skip_all, level = Level::DEBUG, target = "spin_rwlck::write")]
    pub fn write_lock(&self) {
        let mut spins = 0u32;
        // mark self as waiter
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s & WAITER_BIT == 0 {
                if self
                    .state
                    .compare_exchange_weak(s, s | WAITER_BIT, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    // self is the waiter now
                    break;
                }
            } else {
                // other writer is the waiter, wait for write lock
                spins += 1;
                backoff(spins);
            }
        }

        let mut spins = 0u32;
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s & READER_MASK == 0 && s & WRITER_BIT == 0 {
                // precond: self is the write lock waiter
                // no readers hold lock, no writer holds lock
                if self
                    .state
                    .compare_exchange(WAITER_BIT, WRITER_BIT, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    // acquire writer lock, set writer bit
                    return;
                }
            }
            spins += 1;
            backoff(spins);
        }
    }

    pub fn write_unlock(&self) {
        // clear writer bit
        let prev = self.state.swap(0, Ordering::Release);
        debug_assert!(prev & WRITER_BIT != 0);
    }
}
