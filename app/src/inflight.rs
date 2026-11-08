//! Work handed off to another thread that must finish before the app exits:
//! each job holds a guard, and quitting waits once for all of them.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

pub struct Inflight {
    count: Mutex<usize>,
    done: Condvar,
}

/// The app's jobs; tests make their own.
static JOBS: Inflight = Inflight::new();

/// Held by a job until it ends. Dropping it counts the job done, also when
/// the job panics or its task is cancelled.
pub struct Guard(&'static Inflight);

impl Inflight {
    pub const fn new() -> Self {
        Inflight {
            count: Mutex::new(0),
            done: Condvar::new(),
        }
    }

    pub fn track(&'static self) -> Guard {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count += 1;
        Guard(self)
    }

    /// Waits until no job is left or `timeout` passes; returns how many are left.
    pub fn wait(&self, timeout: Duration) -> usize {
        let count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        let (count, _) = self
            .done
            .wait_timeout_while(count, timeout, |count| *count > 0)
            .unwrap_or_else(PoisonError::into_inner);
        *count
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut count = self.0.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count -= 1;
        if *count == 0 {
            self.0.done.notify_all();
        }
    }
}

pub fn track() -> Guard {
    JOBS.track()
}

pub fn wait(timeout: Duration) -> usize {
    JOBS.wait(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_tracked_waits_for_nothing() {
        static T: Inflight = Inflight::new();
        assert_eq!(T.wait(Duration::ZERO), 0);
    }

    #[test]
    fn a_held_job_is_counted_after_the_timeout() {
        static T: Inflight = Inflight::new();
        let guard = T.track();
        assert_eq!(T.wait(Duration::from_millis(50)), 1);
        drop(guard);
    }

    #[test]
    fn a_job_ending_on_another_thread_ends_the_wait() {
        static T: Inflight = Inflight::new();
        let guard = T.track();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(guard);
        });
        let start = std::time::Instant::now();
        assert_eq!(T.wait(Duration::from_secs(5)), 0);
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
