//! AI generated tests
//!
//! TODO: Handmade `loom` based tests?
extern crate std;

use super::*;
pub use crate::rt::test_util::*;
use core::pin::pin;
use core::task::Poll;
use std::boxed::Box;

fn mutex<T>(v: T) -> Mutex<T> {
    Mutex::new(v)
}

fn ready<T>(p: Poll<T>) -> T {
    match p {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("expected Ready, got Pending"),
    }
}

#[test]
fn uncontended_lock_is_immediately_ready_and_mutable() {
    let m = mutex(1u32);
    let (_c, w) = counting();
    let mut l = pin!(m.lock());
    let mut g = ready(poll_with(l.as_mut(), &w));
    *g += 41;
    assert_eq!(*g, 42);
    drop(g);

    let mut l2 = pin!(m.lock());
    assert_eq!(*ready(poll_with(l2.as_mut(), &w)), 42);
}

#[test]
fn try_lock_semantics() {
    let m = mutex(());
    assert!(m.raw.try_lock());
    assert!(!m.raw.try_lock());
    unsafe { m.raw.unlock() };
    assert!(m.raw.try_lock());
}

#[test]
fn contended_lock_pends_then_wakes_on_unlock() {
    let m = mutex(0u32);
    let (_ca, wa) = counting();
    let (cb, wb) = counting();

    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));

    let mut b = pin!(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());
    assert_eq!(cb.count(), 0);

    drop(ga);
    assert_eq!(cb.count(), 1);
    let _gb = ready(poll_with(b.as_mut(), &wb));
}

#[test]
fn spurious_repoll_while_held_stays_pending_and_queued_once() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();
    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));

    let mut b = pin!(m.lock());
    for _ in 0..5 {
        assert!(poll_with(b.as_mut(), &wb).is_pending());
    }
    drop(ga);
    assert_eq!(cb.count(), 1, "queued exactly once, woken exactly once");
}

#[test]
fn waiters_acquire_in_fifo_order() {
    let m = mutex(std::vec::Vec::<u32>::new());
    let (_c0, w0) = counting();
    let (c1, w1) = counting();
    let (c2, w2) = counting();
    let (c3, w3) = counting();

    let mut l0 = pin!(m.lock());
    let g0 = ready(poll_with(l0.as_mut(), &w0));

    let mut l1 = pin!(m.lock());
    let mut l2 = pin!(m.lock());
    let mut l3 = pin!(m.lock());
    assert!(poll_with(l1.as_mut(), &w1).is_pending());
    assert!(poll_with(l2.as_mut(), &w2).is_pending());
    assert!(poll_with(l3.as_mut(), &w3).is_pending());

    drop(g0);
    assert_eq!((c1.count(), c2.count(), c3.count()), (1, 0, 0));
    let mut g1 = ready(poll_with(l1.as_mut(), &w1));
    g1.push(1);

    drop(g1);
    assert_eq!((c1.count(), c2.count(), c3.count()), (1, 1, 0));
    let mut g2 = ready(poll_with(l2.as_mut(), &w2));
    g2.push(2);

    drop(g2);
    assert_eq!((c1.count(), c2.count(), c3.count()), (1, 1, 1));
    let g3 = ready(poll_with(l3.as_mut(), &w3));
    assert_eq!(&**g3, &[1, 2][..]);
}

#[test]
fn dropping_a_pending_lock_removes_it_from_the_queue() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();
    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));

    let mut b = Box::pin(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());
    drop(b);

    assert!(!m.raw.wait_queue.lock().notify_one(), "queue must be empty");
    drop(ga);
    assert_eq!(cb.count(), 0);
}

#[test]
fn dropping_a_never_polled_lock_is_harmless() {
    let m = mutex(());
    drop(m.lock());
    let (_c, w) = counting();
    let mut l = pin!(m.lock());
    let _g = ready(poll_with(l.as_mut(), &w));
}

#[test]
fn dropping_a_completed_lock_future_does_not_wake_others() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();

    let mut a = Box::pin(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));

    let mut b = pin!(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());

    // the Lock future is done (guard taken); dropping it must not
    // forward a wakeup while the guard is still held
    drop(a);
    assert_eq!(cb.count(), 0);
    drop(ga);
    assert_eq!(cb.count(), 1);
}

#[test]
fn dropping_a_notified_lock_forwards_the_wakeup() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();
    let (cc, wc) = counting();

    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));

    let mut b = Box::pin(m.lock());
    let mut c = pin!(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());
    assert!(poll_with(c.as_mut(), &wc).is_pending());

    drop(ga); // b notified
    assert_eq!((cb.count(), cc.count()), (1, 0));

    drop(b); // b never took the lock: c must be woken, not stranded
    assert_eq!(cc.count(), 1);
    let _gc = ready(poll_with(c.as_mut(), &wc));
}

#[test]
fn barging_task_steals_lock_and_notified_waiter_requeues() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();
    let (_cd, wd) = counting();

    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));
    let mut b = pin!(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());

    drop(ga);
    assert_eq!(cb.count(), 1);

    // newcomer grabs the lock via the fast path before b polls
    let mut d = pin!(m.lock());
    let gd = ready(poll_with(d.as_mut(), &wd));

    // b was notified but the lock is taken: no panic, re-queue
    assert!(poll_with(b.as_mut(), &wb).is_pending());

    drop(gd);
    assert_eq!(cb.count(), 2, "b must be woken again");
    let _gb = ready(poll_with(b.as_mut(), &wb));
}

/// Same as above but with a second waiter behind b. b should keep its
/// place. Expected to FAIL until `enqueue` puts `Notified` at the front.
#[test]
fn barged_waiter_keeps_its_place_in_line() {
    let m = mutex(());
    let (_ca, wa) = counting();
    let (cb, wb) = counting();
    let (cc, wc) = counting();
    let (_cd, wd) = counting();

    let mut a = pin!(m.lock());
    let ga = ready(poll_with(a.as_mut(), &wa));
    let mut b = pin!(m.lock());
    let mut c = pin!(m.lock());
    assert!(poll_with(b.as_mut(), &wb).is_pending());
    assert!(poll_with(c.as_mut(), &wc).is_pending());

    drop(ga); // b notified
    let mut d = pin!(m.lock());
    let gd = ready(poll_with(d.as_mut(), &wd)); // steals
    assert!(poll_with(b.as_mut(), &wb).is_pending()); // b re-queues

    drop(gd);
    assert_eq!(cb.count(), 2, "b was first, it must be woken first");
    assert_eq!(cc.count(), 0);
}

#[test]
fn guard_is_released_on_drop_even_across_many_cycles() {
    let m = mutex(0u64);
    let (_c, w) = counting();
    for i in 0..1000 {
        let mut l = pin!(m.lock());
        let mut g = ready(poll_with(l.as_mut(), &w));
        *g += 1;
        assert_eq!(*g, i + 1);
    }
}

#[test]
#[should_panic(expected = "poll() called after Ready")]
fn polling_after_completion_panics() {
    let m = mutex(());
    let (_c, w) = counting();
    let mut l = pin!(m.lock());
    let _g = ready(poll_with(l.as_mut(), &w));
    let _ = poll_with(l.as_mut(), &w);
}

#[test]
fn mutual_exclusion_under_thread_contention() {
    const THREADS: usize = 4;
    const ITERS: u64 = 5_000;
    let m = mutex(0u64);

    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                for _ in 0..ITERS {
                    let mut g = block_on(m.lock());
                    // non-atomic read-modify-write with a yield in the
                    // middle: lost updates mean broken exclusion
                    let v = *g;
                    if v % 7 == 0 {
                        std::thread::yield_now();
                    }
                    *g = v + 1;
                }
            });
        }
    });

    let (_c, w) = counting();
    let mut l = pin!(m.lock());
    assert_eq!(*ready(poll_with(l.as_mut(), &w)), THREADS as u64 * ITERS);
    // and nothing is left behind in the queue
    drop(l);
    assert!(!m.raw.wait_queue.lock().notify_one());
}

#[test]
fn cancelled_lockers_do_not_strand_waiters_under_contention() {
    use core::sync::atomic::AtomicBool;
    let m = mutex(0u64);
    let done = AtomicBool::new(false);

    std::thread::scope(|s| {
        // churn: poll once then drop, repeatedly (random cancel points)
        for _ in 0..3 {
            s.spawn(|| {
                let (_c, w) = counting();
                while !done.load(Ordering::Relaxed) {
                    let mut l = Box::pin(m.lock());
                    if let Poll::Ready(mut g) = poll_with(l.as_mut(), &w) {
                        *g += 1;
                    }
                }
            });
        }
        // a real locker must always make progress
        let h = s.spawn(|| {
            for _ in 0..2_000 {
                let mut g = block_on(m.lock());
                *g += 1;
            }
        });
        h.join().unwrap();
        done.store(true, Ordering::Relaxed);
    });
}
