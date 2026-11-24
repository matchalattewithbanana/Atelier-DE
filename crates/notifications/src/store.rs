use serde::Serialize;
use std::{collections::VecDeque, time::{Duration, Instant}};

const MAX_ACTIVE: usize = 128;
const MAX_HISTORY: usize = 100;

#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub transient: bool,
    pub closed_reason: Option<u32>,
    #[serde(skip)]
    deadline: Option<Instant>,
}

pub struct Store {
    instance: String,
    next_id: u32,
    active: Vec<Notice>,
    history: VecDeque<Notice>,
}

#[derive(Serialize)]
pub struct Snapshot<'a> {
    schema: u32,
    instance: &'a str,
    active: &'a [Notice],
    history: &'a VecDeque<Notice>,
}

impl Default for Store {
    fn default() -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        Self { instance: format!("{}-{stamp}", std::process::id()), next_id: 0, active: Vec::new(), history: VecDeque::new() }
    }
}

fn bounded(text: String, chars: usize) -> String { text.chars().take(chars).collect() }

impl Store {
    pub fn notify(&mut self, replaces: u32, app: String, summary: String, body: String, transient: bool, timeout: i32, now: Instant) -> Result<u32, &'static str> {
        let index = self.active.iter().position(|n| n.id == replaces);
        let id = if index.is_some() { replaces } else {
            if self.active.len() >= MAX_ACTIVE { return Err("Active notification capacity reached"); }
            self.next_id = self.next_id.checked_add(1).ok_or("Notification ID space exhausted")?;
            self.next_id
        };
        let deadline = match timeout {
            0 => None,
            value if value > 0 => Some(now + Duration::from_millis(value as u64)),
            _ => Some(now + Duration::from_secs(5)),
        };
        let notice = Notice { id, app: bounded(app, 256), summary: bounded(summary, 1024), body: bounded(body, 4096), transient, closed_reason: None, deadline };
        if let Some(index) = index { self.active[index] = notice; } else { self.active.push(notice); }
        Ok(id)
    }
    pub fn close(&mut self, id: u32, reason: u32) -> bool {
        let Some(index) = self.active.iter().position(|notice| notice.id == id) else { return false; };
        let mut notice = self.active.remove(index);
        notice.closed_reason = Some(reason);
        if !notice.transient {
            self.history.push_front(notice);
            self.history.truncate(MAX_HISTORY);
        }
        true
    }
    pub fn expire(&mut self, now: Instant) -> Vec<u32> {
        let expired: Vec<_> = self.active.iter().filter(|notice| notice.deadline.is_some_and(|deadline| deadline <= now)).map(|notice| notice.id).collect();
        for id in &expired { self.close(*id, 1); }
        expired
    }
    pub fn snapshot(&self) -> Snapshot<'_> { Snapshot { schema: 1, instance: &self.instance, active: &self.active, history: &self.history } }
    pub fn matches_instance(&self, instance: &str) -> bool { self.instance == instance }
    pub fn clear_history(&mut self) { self.history.clear(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add(store: &mut Store, replace: u32, transient: bool, timeout: i32, now: Instant) -> u32 {
        store.notify(replace, "app".into(), "title".into(), "body".into(), transient, timeout, now).unwrap()
    }
    #[test]
    fn replacement_expiry_and_persistent_active() {
        let mut store = Store::default(); let now = Instant::now();
        let id = add(&mut store, 0, false, 100, now);
        assert_eq!(add(&mut store, id, false, 500, now), id);
        let persistent = add(&mut store, 0, false, 0, now);
        assert!(store.expire(now + Duration::from_millis(150)).is_empty());
        assert_eq!(store.expire(now + Duration::from_secs(1)), vec![id]);
        assert_eq!(store.active[0].id, persistent);
        assert_eq!(store.history[0].closed_reason, Some(1));
    }
    #[test]
    fn transient_notifications_never_enter_history_and_history_is_bounded() {
        let mut store = Store::default(); let now = Instant::now();
        let id = add(&mut store, 0, true, 0, now); assert!(store.close(id, 3));
        assert!(store.history.is_empty());
        for _ in 0..MAX_HISTORY + 10 {
            let id = add(&mut store, 0, false, 0, now); assert!(store.close(id, 3));
        }
        assert_eq!(store.history.len(), MAX_HISTORY);
        store.clear_history(); assert!(store.history.is_empty());
    }
    #[test]
    fn rejects_over_capacity_and_unknown_close() {
        let mut store = Store::default(); let now = Instant::now();
        for _ in 0..MAX_ACTIVE { add(&mut store, 0, false, 0, now); }
        assert!(store.notify(0, "".into(), "".into(), "".into(), false, 0, now).is_err());
        assert!(!store.close(9999, 3));
    }
}
