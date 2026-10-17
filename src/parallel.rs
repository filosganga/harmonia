//! Running independent jobs on every processor core at once.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many jobs run at once: one per core.
pub fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// `f` of every item, several at a time, in the items' order. Put the slowest items first: each
/// worker takes the next item when it's done, so a long one left for the end keeps the rest waiting.
///
/// `scope` lets the workers borrow `items` and `f`; they're all finished when it returns. `next` is
/// a counter all the workers share, and fetch_add hands each one a different item number.
pub fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..workers().min(items.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    done.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}
