// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! What Auto-keywords does when analysis fails (tags_service.rs). A failure is either
//! systemic (the model or the ONNX runtime cannot load, the helper cannot start or keeps
//! dying: every photo would fail the same way) or about one photo (an unreadable picture).
//! Systemic failures back the worker off (5 s, 30 s, 5 min) and then block it until the user
//! presses Retry, so a missing runtime cannot spin on the same photos or respawn the helper in a
//! tight loop. A photo's own failure skips that photo, with a limited number of later tries,
//! and never holds up the others. Time is passed in (milliseconds), so the tests need no clock.
use std::{collections::HashMap, sync::Mutex};

/// Waits after the 1st, 2nd and 3rd systemic failure in a row.
pub const BACKOFF_MS: [u64; 3] = [5_000, 30_000, 300_000];
/// Systemic failures in a row that block the worker (the 4th, after all three waits).
pub const MAX_SYSTEMIC: u32 = 4;
/// Tries per photo and file version before it is left alone.
pub const MAX_PHOTO_TRIES: u32 = 3;
/// Wait before the 2nd, and the 3rd, try of a photo.
pub const PHOTO_RETRY_MS: [u64; 2] = [600_000, 3_600_000];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    /// The model's files cannot be loaded (also when the ONNX runtime is not found).
    Model,
    /// The helper process cannot start.
    Engine,
    /// This build of the engine has no CPU ONNX support.
    NoCpu,
    /// The helper stopped answering or died.
    Crash,
    Other,
}
impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::Model => "model",
            Reason::Engine => "engine",
            Reason::NoCpu => "nocpu",
            Reason::Crash => "crash",
            Reason::Other => "other",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Systemic(Reason),
    /// Only this photo is the problem.
    Photo,
    /// Not a fault: the helper's queue was full. Nothing is recorded.
    Transient,
}

/// Classifies an error message of the helper step (the engine's tags_analyze, engine/tags.c, or
/// the process around it).
pub fn classify_helper(msg: &str) -> Kind {
    let m = msg.to_ascii_lowercase();
    let has = |s: &str| m.contains(s);
    if has("worker is full") {
        Kind::Transient
    } else if has("model is unavailable") || has("unsupported ram++ graph") || has("wrong number of logits") {
        Kind::Systemic(Reason::Model)
    } else if has("built without") {
        Kind::Systemic(Reason::NoCpu)
    } else if has("cannot start the keyword engine") || has("exited at start") {
        Kind::Systemic(Reason::Engine)
    } else if has("engine write") || has("engine restarted") || has("timed out") || has("closed") || has("exited") || has("broken pipe") {
        Kind::Systemic(Reason::Crash)
    } else if has("invalid ram++ tensor") || has("invalid logit") || has("could not read ram++ tensor") {
        Kind::Photo
    } else {
        // an unknown failure of the model's own step is more likely to hit every photo
        Kind::Systemic(Reason::Other)
    }
}

pub enum Gate {
    Go,
    /// Waiting out a backoff.
    Wait,
    Blocked,
}

#[derive(Default)]
struct Inner {
    consecutive: u32,
    generation: u64,
    next_ms: u64,
    blocked: bool,
    reason: Option<Reason>,
    detail: String,
}

/// The worker's health: one per service.
#[derive(Default)]
pub struct Health {
    inner: Mutex<Inner>,
}

/// What the Preferences card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// "ok", "retrying" or "blocked"
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub retry_in_s: Option<u32>,
    /// The engine's own words, for the log and a tooltip.
    pub detail: String,
}

impl Health {
    pub fn gate(&self, now: u64) -> Gate {
        let i = self.inner.lock().unwrap();
        if i.blocked {
            Gate::Blocked
        } else if i.consecutive > 0 && now < i.next_ms {
            Gate::Wait
        } else {
            Gate::Go
        }
    }
    /// Taken when a photo's job starts; a failure from before the last change is not counted again
    /// (two photos in flight fail together for the same cause).
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }
    /// Records a systemic failure. Returns a line for the log only when the state changed (the
    /// first failure, or the block), so a retried failure does not repeat itself.
    pub fn systemic(&self, generation: u64, reason: Reason, detail: &str, now: u64) -> Option<String> {
        let mut i = self.inner.lock().unwrap();
        if generation != i.generation || i.blocked {
            return None;
        }
        i.generation += 1;
        i.consecutive += 1;
        i.reason = Some(reason);
        i.detail = detail.to_string();
        if i.consecutive >= MAX_SYSTEMIC {
            i.blocked = true;
            Some(format!("auto-keywords blocked after {} failures in a row ({}): {detail}", i.consecutive, reason.code()))
        } else {
            let wait = BACKOFF_MS[(i.consecutive as usize - 1).min(BACKOFF_MS.len() - 1)];
            i.next_ms = now + wait;
            (i.consecutive == 1).then(|| format!("auto-keywords cannot analyse ({}): {detail}; trying again in {} s", reason.code(), wait / 1000))
        }
    }
    /// A photo was analysed: the cause is gone. Returns a log line when it had been failing.
    pub fn ok(&self) -> Option<String> {
        let mut i = self.inner.lock().unwrap();
        if i.consecutive == 0 && !i.blocked {
            return None;
        }
        i.generation += 1;
        *i = Inner { generation: i.generation, ..Inner::default() };
        Some("auto-keywords analysis works again".into())
    }
    /// The user's Retry.
    pub fn retry(&self) {
        let mut i = self.inner.lock().unwrap();
        *i = Inner { generation: i.generation + 1, ..Inner::default() };
    }
    pub fn report(&self, now: u64) -> Report {
        let i = self.inner.lock().unwrap();
        if i.blocked {
            Report { state: "blocked", reason: i.reason.map(Reason::code), retry_in_s: None, detail: i.detail.clone() }
        } else if i.consecutive > 0 {
            let left = i.next_ms.saturating_sub(now).div_ceil(1000) as u32;
            Report { state: "retrying", reason: i.reason.map(Reason::code), retry_in_s: Some(left), detail: i.detail.clone() }
        } else {
            Report { state: "ok", reason: None, retry_in_s: None, detail: String::new() }
        }
    }
}

struct Entry {
    fingerprint: String,
    tries: u32,
    retry_at: u64,
}

/// Photos that failed on their own, by key (the path), with the file version that failed.
#[derive(Default)]
pub struct Ledger {
    map: Mutex<HashMap<String, Entry>>,
}

impl Ledger {
    /// Records a failure; returns the tries so far.
    pub fn fail(&self, key: &str, fingerprint: &str, now: u64) -> u32 {
        let mut map = self.map.lock().unwrap();
        let e = map.entry(key.to_string()).or_insert(Entry { fingerprint: fingerprint.into(), tries: 0, retry_at: 0 });
        if e.fingerprint != fingerprint {
            *e = Entry { fingerprint: fingerprint.into(), tries: 0, retry_at: 0 };
        }
        e.tries += 1;
        e.retry_at = match PHOTO_RETRY_MS.get(e.tries as usize - 1) {
            Some(wait) if e.tries < MAX_PHOTO_TRIES => now + wait,
            _ => u64::MAX,
        };
        e.tries
    }
    /// Whether the photo is left alone now: given up on, or waiting for its next try. A changed
    /// file is a new photo.
    pub fn skipped(&self, key: &str, fingerprint: &str, now: u64) -> bool {
        self.map.lock().unwrap().get(key).is_some_and(|e| e.fingerprint == fingerprint && now < e.retry_at)
    }
    pub fn clear(&self, key: &str) {
        self.map.lock().unwrap().remove(key);
    }
    pub fn reset(&self) {
        self.map.lock().unwrap().clear();
    }
    /// Photos currently left alone.
    pub fn count(&self, now: u64) -> u32 {
        self.map.lock().unwrap().values().filter(|e| now < e.retry_at).count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemic_and_per_photo_messages_are_told_apart() {
        for m in [
            "RAM++ model is unavailable; verify the Auto-keywords model",
            "This engine was built without CPU ONNX support",
            "cannot start the keyword engine: the engine exited at start (exit status: 1)",
            "engine write: Broken pipe (os error 32)",
            "CPU RAM++ inference failed",
        ] {
            assert!(matches!(classify_helper(m), Kind::Systemic(_)), "{m}");
        }
        assert_eq!(classify_helper("RAM++ model is unavailable"), Kind::Systemic(Reason::Model));
        assert_eq!(classify_helper("built without CPU ONNX support"), Kind::Systemic(Reason::NoCpu));
        assert_eq!(classify_helper("Invalid RAM++ tensor values"), Kind::Photo);
        assert_eq!(classify_helper("RAM++ worker is full or stopping"), Kind::Transient);
    }

    #[test]
    fn systemic_failures_back_off_then_block_and_log_twice() {
        let h = Health::default();
        let mut now = 1_000;
        let mut logs = 0;
        for (n, wait) in BACKOFF_MS.iter().enumerate() {
            assert!(matches!(h.gate(now), Gate::Go), "attempt {n}");
            logs += h.systemic(h.generation(), Reason::Model, "no runtime", now).is_some() as u32;
            assert!(matches!(h.gate(now), Gate::Wait));
            assert!(matches!(h.gate(now + wait - 1), Gate::Wait), "still waiting just before {wait} ms");
            let r = h.report(now);
            assert_eq!((r.state, r.reason, r.retry_in_s), ("retrying", Some("model"), Some((*wait / 1000) as u32)));
            now += wait;
        }
        assert!(matches!(h.gate(now), Gate::Go));
        logs += h.systemic(h.generation(), Reason::Model, "no runtime", now).is_some() as u32;
        assert!(matches!(h.gate(now + 10_000_000), Gate::Blocked), "blocked for good, whatever the time");
        assert_eq!(h.report(now).state, "blocked");
        assert_eq!(logs, 2, "one line for the first failure, one for the block");
        // nothing more is counted or logged while blocked
        assert!(h.systemic(h.generation(), Reason::Crash, "x", now).is_none());
        // Retry and a later success both clear it
        h.retry();
        assert!(matches!(h.gate(now), Gate::Go));
        assert_eq!(h.report(now).state, "ok");
    }

    #[test]
    fn a_failure_that_started_before_the_last_one_is_not_counted_twice() {
        let h = Health::default();
        let (a, b) = (h.generation(), h.generation());
        assert!(h.systemic(a, Reason::Model, "x", 0).is_some());
        assert!(h.systemic(b, Reason::Model, "x", 0).is_none());
        assert_eq!(h.report(0).retry_in_s, Some(5), "still the first backoff");
    }

    #[test]
    fn a_success_resets_the_count() {
        let h = Health::default();
        h.systemic(h.generation(), Reason::Crash, "x", 0);
        assert!(h.ok().is_some());
        assert!(h.ok().is_none());
        h.systemic(h.generation(), Reason::Crash, "x", 0);
        assert_eq!(h.report(0).retry_in_s, Some(5), "back to the first wait");
    }

    #[test]
    fn a_photo_is_tried_a_few_times_then_left_alone_until_its_file_changes() {
        let l = Ledger::default();
        let mut now = 0;
        assert!(!l.skipped("/a.jpg", "v1", now));
        for tries in 1..=MAX_PHOTO_TRIES {
            assert_eq!(l.fail("/a.jpg", "v1", now), tries);
            assert!(l.skipped("/a.jpg", "v1", now), "left alone right after a failure");
            if tries < MAX_PHOTO_TRIES {
                now += PHOTO_RETRY_MS[tries as usize - 1];
                assert!(!l.skipped("/a.jpg", "v1", now), "tried again after the wait");
            }
        }
        assert!(l.skipped("/a.jpg", "v1", u64::MAX - 1), "given up on");
        assert_eq!(l.count(now), 1);
        assert!(!l.skipped("/a.jpg", "v2", now), "an edited file is a new photo");
        l.clear("/a.jpg");
        assert_eq!(l.count(now), 0);
    }
}
