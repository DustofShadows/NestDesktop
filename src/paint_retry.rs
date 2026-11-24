//! Bounded recovery for an initial native-surface presentation failure.
//!
//! The caller starts a timer only while this queue is nonempty. A failed
//! initial presentation requests work; at most three subsequent attempts are
//! allowed. Repeated native paint messages must not replenish that budget.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
    rc::Rc,
};

pub(crate) const MAX_RETRIES: u8 = 3;

#[derive(Default, Debug)]
pub(crate) struct RetryQueue {
    pending: BTreeMap<isize, u8>,
}

impl RetryQueue {
    /// Queue a window once, without resetting an existing retry budget.
    pub(crate) fn request(&mut self, id: isize) -> bool {
        if id == 0 || self.pending.contains_key(&id) {
            return false;
        }
        self.pending.insert(id, 0);
        true
    }

    /// A stable snapshot permits completion/removal while iterating attempts.
    pub(crate) fn pending(&self) -> Vec<isize> {
        self.pending.keys().copied().collect()
    }

    pub(crate) fn succeeded(&mut self, id: isize) -> bool {
        self.remove(id)
    }

    /// Forget destroyed/stale windows without treating them as paint failures.
    pub(crate) fn remove(&mut self, id: isize) -> bool {
        self.pending.remove(&id).is_some()
    }

    /// Record one failed retry. False means no further retry is scheduled.
    /// Call only after a retry attempt, not for the original presentation.
    pub(crate) fn failed(&mut self, id: isize) -> bool {
        let Some(attempts) = self.pending.get_mut(&id) else {
            return false;
        };
        *attempts += 1;
        if *attempts < MAX_RETRIES {
            true
        } else {
            self.pending.remove(&id);
            false
        }
    }

    /// Rebuilding the native windows starts a fresh lifecycle and budget.
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

thread_local! {
    static RENDERING: RefCell<HashMap<isize, usize>> = RefCell::new(HashMap::new());
}

pub(crate) fn is_rendering(id: isize) -> bool {
    RENDERING.with(|rendering| rendering.borrow().contains_key(&id))
}

/// Prevent reentrant native painting from feeding back into the retry queue.
/// Keep the guard on the rendering thread and hold it through presentation.
#[must_use = "hold the painting guard until surface presentation has finished"]
pub(crate) struct Painting {
    id: isize,
    _same_thread: PhantomData<Rc<()>>,
}

impl Painting {
    pub(crate) fn begin(id: isize) -> Self {
        RENDERING.with(|rendering| {
            *rendering.borrow_mut().entry(id).or_default() += 1;
        });
        Self {
            id,
            _same_thread: PhantomData,
        }
    }
}

impl Drop for Painting {
    fn drop(&mut self) {
        let _ = RENDERING.try_with(|rendering| {
            let mut rendering = rendering.borrow_mut();
            if let Some(depth) = rendering.get_mut(&self.id) {
                *depth -= 1;
                if *depth == 0 {
                    rendering.remove(&self.id);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_cancels_pending_work_and_leaves_no_timer_reason() {
        let mut queue = RetryQueue::default();
        assert!(queue.is_empty());
        for id in [10, 20, 30] {
            assert!(queue.request(id));
        }
        let snapshot = queue.pending();
        assert_eq!(snapshot, [10, 20, 30]);
        for id in snapshot {
            assert!(queue.succeeded(id));
        }
        assert!(queue.pending().is_empty());
        assert!(queue.is_empty());
        assert!(!queue.failed(10));
    }

    #[test]
    fn one_failed_retry_can_recover_without_disturbing_other_windows() {
        let mut queue = RetryQueue::default();
        queue.request(10);
        queue.request(20);
        assert!(queue.failed(10));
        assert!(!queue.request(10));
        assert_eq!(queue.pending(), [10, 20]);
        assert!(queue.succeeded(10));
        assert_eq!(queue.pending(), [20]);
        assert!(queue.succeeded(20));
        assert!(queue.is_empty());
    }

    #[test]
    fn repeated_requests_do_not_replenish_the_three_retry_budget() {
        let mut queue = RetryQueue::default();
        assert!(!queue.request(0));
        queue.request(10);
        for _ in 1..MAX_RETRIES {
            assert!(queue.failed(10));
            assert!(!queue.request(10));
        }
        assert!(!queue.failed(10));
        assert!(queue.is_empty());
        assert!(!queue.failed(10));
        assert!(queue.is_empty());
    }

    #[test]
    fn rebuild_and_stale_window_removal_discard_old_lifecycles() {
        let mut queue = RetryQueue::default();
        queue.request(10);
        queue.request(20);
        assert!(queue.failed(10));
        assert!(queue.remove(20));
        assert!(!queue.remove(20));
        assert_eq!(queue.pending(), [10]);
        queue.clear();
        assert!(queue.is_empty());
        // A reused HWND after rebuild receives a fresh, complete budget.
        assert!(queue.request(10));
        assert!(queue.failed(10));
        assert!(queue.failed(10));
        assert!(!queue.failed(10));
        assert!(queue.is_empty());
    }

    #[test]
    fn rendering_guards_are_nested_per_window_and_release_on_unwind() {
        assert!(!is_rendering(10));
        let first = Painting::begin(10);
        assert!(is_rendering(10));
        assert!(!is_rendering(20));
        {
            let _nested = Painting::begin(10);
            let _other = Painting::begin(20);
            assert!(is_rendering(10));
            assert!(is_rendering(20));
        }
        assert!(is_rendering(10));
        assert!(!is_rendering(20));
        drop(first);
        assert!(!is_rendering(10));
        let _ = std::panic::catch_unwind(|| {
            let _guard = Painting::begin(10);
            panic!("synthetic draw failure");
        });
        assert!(!is_rendering(10));
    }
}
