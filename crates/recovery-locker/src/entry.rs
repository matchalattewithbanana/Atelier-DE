//! Password-entry and one-shot authentication state, independent of Wayland.
//! No protocol unlock authority is created here. The eventual event loop
//! must supply readiness from the confirmed lock, mapped root and exact focus.
use std::{
    io,
    sync::{atomic::{compiler_fence, Ordering}, mpsc::{self, Receiver, TryRecvError}},
    thread,
    time::{Duration, Instant},
};

const CAPACITY: usize = 4096;
const ATTEMPT_LIMIT: Duration = Duration::from_secs(30);

fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

// Fixed allocation: no growing buffer, Clone, Debug, or plaintext accessor
// outside this module. Moving an attempt transfers its existing allocation.
struct Password {
    bytes: Box<[u8; CAPACITY]>,
    len: usize,
}
impl Default for Password {
    fn default() -> Self {
        Self { bytes: Box::new([0; CAPACITY]), len: 0 }
    }
}
impl Password {
    fn push(&mut self, ch: char) -> bool {
        if ch.is_control() || self.len + ch.len_utf8() > CAPACITY {
            return false;
        }
        let end = self.len + ch.len_utf8();
        ch.encode_utf8(&mut self.bytes[self.len..end]);
        self.len = end;
        true
    }
    fn backspace(&mut self) -> bool {
        if self.len == 0 { return false; }
        let mut start = self.len - 1;
        while start > 0 && self.bytes[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        erase(&mut self.bytes[start..self.len]);
        self.len = start;
        true
    }
    fn clear(&mut self) {
        erase(&mut self.bytes[..]);
        self.len = 0;
    }
}
impl Drop for Password {
    fn drop(&mut self) { self.clear(); }
}

struct Pending {
    result: Receiver<bool>,
    deadline: Instant,
    valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Poll {
    Idle,
    Busy,
    RetryLater,
    // Consumed once; not cached or retained across future events.
    Verified,
}

#[derive(Default)]
pub(super) struct Entry {
    password: Password,
    ready: bool,
    finished: bool,
    pending: Option<Pending>,
    failures: u8,
    retry_after: Option<Instant>,
}

impl Entry {
    /// False on focus loss, protocol failure, root loss or unconfirmed lock.
    /// Re-entering focus cannot resurrect an attempt that was invalidated.
    pub(super) fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if !ready { self.cancel(); }
    }

    pub(super) fn cancel(&mut self) {
        self.password.clear();
        if let Some(pending) = &mut self.pending {
            pending.valid = false;
        }
    }

    fn can_edit(&self, now: Instant) -> bool {
        self.ready && !self.finished && self.pending.is_none() &&
            self.retry_after.is_none_or(|until| now >= until)
    }

    pub(super) fn insert(&mut self, ch: char, now: Instant) -> bool {
        self.can_edit(now) && self.password.push(ch)
    }

    pub(super) fn backspace(&mut self, now: Instant) -> bool {
        self.can_edit(now) && self.password.backspace()
    }

    /// Length is not exposed. The renderer can show a generic entry indicator
    /// rather than encode the exact password length in the lock surface.
    pub(super) fn has_input(&self) -> bool { self.password.len != 0 }

    /// Transfer the one existing buffer to a dedicated worker thread. The
    /// injected operation is an internal boundary for the future trusted
    /// launcher; no public transport or arbitrary command selection exists.
    /// It must independently enforce its own process deadline.
    pub(super) fn submit(
        &mut self,
        now: Instant,
        authenticate: impl FnOnce(&mut [u8]) -> bool + Send + 'static,
    ) -> io::Result<bool> {
        if !self.can_edit(now) || self.password.len == 0 {
            return Ok(false);
        }
        let mut secret = std::mem::take(&mut self.password);
        let (tx, rx) = mpsc::sync_channel(1);
        let spawn = thread::Builder::new().name("atelier-recovery-auth".into()).spawn(move || {
            let len = secret.len;
            let decision = authenticate(&mut secret.bytes[..len]);
            // Erase even if the injected implementation did not do so.
            drop(secret);
            let _ = tx.send(decision);
        });
        if let Err(error) = spawn {
            self.record_failure(now);
            return Err(error);
        }
        self.pending = Some(Pending {
            result: rx, deadline: now + ATTEMPT_LIMIT, valid: true,
        });
        Ok(true)
    }

    fn record_failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        // 1, 2, 4, 8 seconds, bounded. Not a persistent PAM rate limiter.
        let seconds = 1u64 << (self.failures - 1).min(3);
        self.retry_after = Some(now + Duration::from_secs(seconds));
    }

    /// Nonblocking. After invalidation or timeout, keep the single-attempt
    /// slot occupied until its thread resolves; never start overlapping PAM
    /// calls. A late success or disconnected result channel is not approval.
    pub(super) fn poll(&mut self, now: Instant) -> Poll {
        if let Some(pending) = &mut self.pending {
            if now >= pending.deadline { pending.valid = false; }
            let result = match pending.result.try_recv() {
                Ok(decision) => decision && pending.valid && self.ready,
                Err(TryRecvError::Disconnected) => false,
                Err(TryRecvError::Empty) => return Poll::Busy,
            };
            self.pending = None;
            if result {
                self.finished = true;
                return Poll::Verified;
            }
            self.record_failure(now);
        }
        if self.retry_after.is_some_and(|until| now < until) {
            Poll::RetryLater
        } else {
            Poll::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(entry: &mut Entry, now: Instant) -> Poll {
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            let result = entry.poll(now);
            if result != Poll::Busy { return result; }
            assert!(Instant::now() < end, "synthetic worker did not resolve");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn bounded_unicode_buffer_erases_removed_bytes() {
        let mut p = Password::default();
        assert!(p.push('a'));
        assert!(p.push('界'));
        assert_eq!(&p.bytes[..4], "a界".as_bytes());
        assert!(p.backspace());
        assert_eq!(&p.bytes[..4], &[b'a', 0, 0, 0]);
        assert!(!p.push('\0'));
        assert!(!p.push('\n'));
        for _ in 1..CAPACITY { assert!(p.push('x')); }
        assert!(!p.push('z'));
        assert!(!p.push('界'));
        p.clear();
        assert_eq!(p.len, 0);
        assert!(p.bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn input_requires_ready_and_focus_loss_clears_it() {
        let now = Instant::now();
        let mut entry = Entry::default();
        assert!(!entry.insert('x', now));
        assert!(!entry.submit(now, |_| panic!("not ready")).unwrap());
        entry.set_ready(true);
        assert!(entry.insert('x', now));
        assert!(entry.has_input());
        entry.set_ready(false);
        assert!(!entry.has_input());
        entry.set_ready(true);
        assert!(!entry.submit(now, |_| panic!("empty")).unwrap());
    }

    #[test]
    fn success_is_consumed_once_and_only_one_attempt_can_run() {
        let now = Instant::now();
        let mut entry = Entry::default();
        entry.set_ready(true);
        assert!(entry.insert('界', now));
        assert!(entry.submit(now, |p| p == "界".as_bytes()).unwrap());
        assert!(!entry.has_input());
        assert!(!entry.insert('x', now));
        assert!(!entry.submit(now, |_| panic!("overlapping attempt")).unwrap());
        assert_eq!(collect(&mut entry, now), Poll::Verified);
        assert_eq!(entry.poll(now), Poll::Idle);
        assert!(!entry.insert('x', now));
    }

    #[test]
    fn focus_loss_or_cancel_invalidates_inflight_success() {
        for focus_loss in [false, true] {
            let now = Instant::now();
            let mut entry = Entry::default();
            entry.set_ready(true);
            entry.insert('x', now);
            let (tx, rx) = mpsc::channel();
            entry.submit(now, move |_| rx.recv().unwrap()).unwrap();
            if focus_loss {
                entry.set_ready(false);
                entry.set_ready(true);
            } else {
                entry.cancel();
            }
            assert_eq!(entry.poll(now), Poll::Busy);
            assert!(!entry.submit(now, |_| panic!("overlapping attempt")).unwrap());
            tx.send(true).unwrap();
            assert_eq!(collect(&mut entry, now), Poll::RetryLater);
            assert!(!entry.insert('x', now));
            assert!(entry.insert('x', now + Duration::from_secs(1)));
        }
    }

    #[test]
    fn deadline_and_worker_failure_never_verify() {
        let now = Instant::now();
        let mut entry = Entry::default();
        entry.set_ready(true);
        entry.insert('x', now);
        let (tx, rx) = mpsc::channel();
        entry.submit(now, move |_| rx.recv().unwrap()).unwrap();
        let expired = now + ATTEMPT_LIMIT;
        assert_eq!(entry.poll(expired), Poll::Busy);
        tx.send(true).unwrap();
        assert_eq!(collect(&mut entry, expired), Poll::RetryLater);
        let next = expired + Duration::from_secs(1);
        assert!(entry.insert('x', next));
        entry.submit(next, |_| panic!("synthetic worker failure")).unwrap();
        assert_eq!(collect(&mut entry, next), Poll::RetryLater);
    }

    #[test]
    fn retry_delay_is_bounded() {
        let mut entry = Entry::default();
        entry.set_ready(true);
        let mut now = Instant::now();
        for seconds in [1, 2, 4, 8, 8] {
            assert!(entry.insert('x', now));
            assert!(entry.submit(now, |_| false).unwrap());
            assert_eq!(collect(&mut entry, now), Poll::RetryLater);
            assert!(!entry.insert('x', now + Duration::from_secs(seconds) - Duration::from_millis(1)));
            now += Duration::from_secs(seconds);
            assert_eq!(entry.poll(now), Poll::Idle);
        }
    }
}
