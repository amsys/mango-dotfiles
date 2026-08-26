//! Delta-only ironvar tracking. ironbar's IPC has no batch command and each
//! `var set` is a full connect+write+read+close (see IRONBAR.md Spike
//! findings S4), so writing a value ironbar already holds is pure waste —
//! `set()` is the only writer and it compares against `live` first.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

// ponytail: fixed cooldown, not exponential — a destroyed-output workspace
// class fails forever until the output returns, so backoff shape doesn't
// matter here. Revisit if a transient-failure case needs faster recovery.
const RETRY_COOLDOWN: Duration = Duration::from_secs(5);

pub struct Vars {
    live: HashMap<Box<str>, Box<str>>,
    dirty: BTreeMap<Box<str>, Box<str>>,
    /// Keys that just failed an IPC send, and when they become eligible for
    /// another attempt. Without this, a permanently-failing key (e.g. a
    /// workspace class for an output mango has destroyed) retries every
    /// flush forever and starves every other dirty key behind it in `dirty`
    /// — a `Module not found` reject on one output can lock the whole bar's
    /// UI to stale state.
    cooldown: HashMap<Box<str>, Instant>,
}

impl Vars {
    pub fn new() -> Self {
        Self {
            live: HashMap::new(),
            dirty: BTreeMap::new(),
            cooldown: HashMap::new(),
        }
    }

    /// Queues `key=value` if it differs from what ironbar last acknowledged.
    /// Returns true if this made something dirty (caller arms the debounce).
    pub fn set(&mut self, key: &str, value: impl Into<Box<str>>) -> bool {
        let value = value.into();
        if self.live.get(key).map(|v| &**v) == Some(&*value) {
            return false;
        }
        self.dirty.insert(key.into(), value);
        true
    }

    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// The value ironbar last acknowledged for `key`, or `None` if we have
    /// never sent it (or it was wiped by `mark_all_dirty`). Used by the
    /// `@class/` flush path (mango.rs) to know which CSS class to remove
    /// before adding the new one.
    pub fn live_value(&self, key: &str) -> Option<&str> {
        self.live.get(key).map(|v| &**v)
    }

    /// Takes the next dirty pair that isn't cooling down from a prior
    /// failure, without removing it from `dirty` — the caller (ipc::flush)
    /// removes on ACK and leaves it on failure, so a send that never got a
    /// response naturally replays once its cooldown elapses. Skipping past a
    /// cooling-down key (rather than stopping at it) is what lets sibling
    /// keys still flush every cycle.
    pub fn peek_dirty(&self) -> Option<(&str, &str)> {
        let now = Instant::now();
        self.dirty
            .iter()
            .find(|(k, _)| self.cooldown.get(*k).is_none_or(|&until| now >= until))
            .map(|(k, v)| (&**k, &**v))
    }

    pub fn ack(&mut self, key: &str) {
        if let Some((k, v)) = self.dirty.remove_entry(key) {
            self.cooldown.remove(&k);
            self.live.insert(k, v);
        }
    }

    /// Records a send failure for `key` so `peek_dirty` skips it until
    /// `RETRY_COOLDOWN` elapses, instead of handing it back every flush.
    pub fn back_off(&mut self, key: &str) {
        self.cooldown.insert(key.into(), Instant::now() + RETRY_COOLDOWN);
    }

    /// Called when the ironbar socket's inode changes (restart detected) —
    /// everything we believe ironbar holds is now wrong, since a fresh
    /// ironbar starts from its config's `ironvar_defaults`, not our state.
    pub fn mark_all_dirty(&mut self) {
        self.dirty.extend(self.live.drain());
    }
}

/// Escapes a string for embedding in a JSON string literal. ironbar's ironvar
/// keys/values are user- or system-derived text (window titles, SSIDs) that
/// can contain `"`, `\`, and control characters.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_skips_unchanged_value() {
        let mut v = Vars::new();
        assert!(v.set("k", "a"));
        v.ack("k");
        assert!(!v.set("k", "a"));
        assert!(v.set("k", "b"));
    }

    #[test]
    fn mark_all_dirty_requeues_live_values() {
        let mut v = Vars::new();
        v.set("k", "a");
        v.ack("k");
        assert!(!v.has_dirty());
        v.mark_all_dirty();
        assert!(v.has_dirty());
        assert_eq!(v.peek_dirty(), Some(("k", "a")));
    }

    #[test]
    fn back_off_skips_the_failing_key_but_not_its_siblings() {
        // Regression test for the 2026-08-26 freeze: a permanently-failing
        // key (workspace class for a destroyed output) must not block every
        // other dirty key behind it in `dirty` forever.
        let mut v = Vars::new();
        v.set("a", "1");
        v.set("b", "2");
        assert_eq!(v.peek_dirty(), Some(("a", "1")));

        v.back_off("a");
        assert_eq!(
            v.peek_dirty(),
            Some(("b", "2")),
            "cooling-down key must be skipped, not returned again"
        );

        v.ack("b");
        assert_eq!(
            v.peek_dirty(),
            None,
            "the cooling-down key must stay hidden until its cooldown elapses"
        );
    }

    #[test]
    fn json_escape_covers_quote_backslash_and_control_chars() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(json_escape("caf\u{e9}"), "caf\u{e9}");
    }
}
