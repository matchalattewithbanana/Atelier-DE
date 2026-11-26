use serde::Serialize;
use atelier_config::notification_preferences::Preferences;
use std::{collections::VecDeque, time::{Duration, Instant}};

const MAX_ACTIVE: usize = 128;
const MAX_HISTORY: usize = 100;
const MAX_ACTIONS: usize = 8;
const MAX_POPUPS: usize = 3;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Action { pub key: String, pub label: String }

pub struct Details {
    pub actions: Vec<Action>,
    pub progress: Option<u8>,
    pub icon: String,
    pub urgency: u8,
    pub owner: Option<String>,
}
impl Default for Details {
    fn default()->Self{Self{actions:vec![],progress:None,icon:String::new(),urgency:1,owner:None}}
}
impl Details {
    pub fn parse(actions: Vec<String>, progress: Option<i32>, icon: String, owner: Option<String>) -> Self {
        let mut valid: Vec<Action> = Vec::new();
        if actions.len() % 2 == 0 && owner.is_some() {
            for pair in actions.chunks_exact(2).take(MAX_ACTIONS) {
                let key = &pair[0];
                if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) || valid.iter().any(|a| a.key == *key) { continue; }
                let label = bounded(pair[1].clone(), 128);
                if !label.trim().is_empty() { valid.push(Action { key: key.clone(), label }); }
            }
        }
        Self { actions: valid, progress: progress.and_then(|v| u8::try_from(v).ok()).filter(|v| *v <= 100), icon: bounded(icon, 256), urgency:1, owner }
    }
}


#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    pub id: u32,
    pub revision: u64,
    pub actions: Vec<Action>,
    pub progress: Option<u8>,
    pub icon: String,
    pub urgency: u8,
    #[serde(skip)]
    owner: Option<String>,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub transient: bool,
    pub popup_allowed: bool,
    pub missed: bool,
    #[serde(skip)]
    history_allowed: bool,
    pub closed_reason: Option<u32>,
    #[serde(skip)]
    deadline: Option<Instant>,
}

pub struct Store {
    instance: String,
    dnd: bool,
    paused_until: Option<Instant>,
    preferences: Preferences,
    next_id: u32,
    revision: u64,
    active: Vec<Notice>,
    history: VecDeque<Notice>,
}

#[derive(Serialize)]
pub struct Snapshot<'a> {
    schema: u32,
    instance: &'a str,
    dnd: bool,
    manual_dnd: bool,
    pause_remaining_seconds: u32,
    active: &'a [Notice],
    history: &'a VecDeque<Notice>,
}

#[derive(Serialize)]
pub struct PopupSnapshot<'a> {
    schema: u32,
    instance: &'a str,
    last_id: u32,
    latest_revision: u64,
    dnd: bool,
    manual_dnd: bool,
    pause_remaining_seconds: u32,
    notice: Option<&'a Notice>,
    cards: Vec<&'a Notice>,
    queued: usize,
}

impl Default for Store {
    fn default() -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        Self { instance: format!("{}-{stamp}", std::process::id()), dnd: false, paused_until: None, preferences:Preferences::default(), next_id: 0, revision: 0, active: Vec::new(), history: VecDeque::new() }
    }
}

fn bounded(text: String, chars: usize) -> String { text.chars().take(chars).collect() }

impl Store {
    pub fn notify(&mut self, replaces: u32, app: String, summary: String, body: String, transient: bool, timeout: i32, now: Instant) -> Result<u32, &'static str> {
        self.notify_with(replaces, app, summary, body, transient, timeout, now, Details::default())
    }
    pub fn notify_with(&mut self, replaces: u32, app: String, summary: String, body: String, transient: bool, timeout: i32, now: Instant, details: Details) -> Result<u32, &'static str> {
        let app=bounded(app,256);
        let rule=self.preferences.app(&app);
        let suppressed = self.suppressed(now) || !rule.popups;
        let index = self.active.iter().position(|n| n.id == replaces && ((n.actions.is_empty() && details.actions.is_empty()) || (n.owner.is_some() && n.owner == details.owner)));
        let revision = self.revision.checked_add(1).ok_or("Notification revision space exhausted")?;
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
        self.revision = revision;
        let notice = Notice { id, revision, actions: details.actions, progress: details.progress, icon: details.icon, urgency:details.urgency, owner: details.owner, app, summary: bounded(summary, 1024), body: bounded(body, 4096), transient, popup_allowed: !suppressed, missed: suppressed, history_allowed:rule.history, closed_reason: None, deadline };
        if let Some(index) = index { self.active[index] = notice; } else { self.active.push(notice); }
        Ok(id)
    }
    pub fn close(&mut self, id: u32, reason: u32) -> bool {
        let Some(index) = self.active.iter().position(|notice| notice.id == id) else { return false; };
        let mut notice = self.active.remove(index);
        notice.closed_reason = Some(reason);
        notice.actions.clear();
        notice.owner = None;
        if !notice.transient && notice.history_allowed {
            self.history.push_front(notice);
            self.history.truncate(MAX_HISTORY);
        }
        true
    }
    pub fn invoke(&mut self, instance: &str, id: u32, revision: u64, key: &str, now: Instant) -> Option<String> {
        if !self.matches_instance(instance) { return None; }
        let notice = self.active.iter_mut().find(|n| n.id == id && n.revision == revision && n.deadline.is_none_or(|d| d > now))?;
        if !notice.actions.iter().any(|a| a.key == key) { return None; }
        let owner = notice.owner.clone()?;
        // Each displayed revision can yield at most one user action.
        notice.actions.clear();
        Some(owner)
    }
    pub fn close_from(&mut self, id: u32, owner: Option<&str>) -> bool {
        if self.active.iter().any(|n| n.id == id && !n.actions.is_empty() && n.owner.as_deref() != owner) { return false; }
        self.close(id, 3)
    }
    pub fn expire(&mut self, now: Instant) -> Vec<u32> {
        let expired: Vec<_> = self.active.iter().filter(|notice| notice.deadline.is_some_and(|deadline| deadline <= now)).map(|notice| notice.id).collect();
        for id in &expired { self.close(*id, 1); }
        expired
    }
    pub fn apply_preferences(&mut self, next: Preferences) {
        for notice in &mut self.active {
            let rule=next.app(&notice.app);
            if !rule.popups {notice.popup_allowed=false;}
            // Do not resurrect a previously excluded notice or its history.
            if !rule.history {notice.history_allowed=false;}
        }
        self.preferences=next;
    }
    fn suppressed(&self, now: Instant) -> bool {
        self.dnd || self.paused_until.is_some_and(|until| now < until)
    }
    fn remaining(&self, now: Instant) -> u32 {
        self.paused_until.and_then(|until| until.checked_duration_since(now))
            .map_or(0, |left| left.as_secs().min(u64::from(u32::MAX - 1)) as u32 + u32::from(left.subsec_nanos() != 0))
    }
    pub fn snapshot(&self) -> Snapshot<'_> {
        let now=Instant::now();
        Snapshot { schema: 1, instance: &self.instance, dnd: self.suppressed(now), manual_dnd: self.dnd,
            pause_remaining_seconds: self.remaining(now), active: &self.active, history: &self.history }
    }
    fn hide_popups(&mut self) {
        for notice in &mut self.active { notice.popup_allowed = false; }
    }
    pub fn set_dnd(&mut self, enabled: bool) {
        self.dnd = enabled;
        if enabled { self.hide_popups(); }
    }
    /// Zero cancels only the temporary pause; manual DND is independent.
    pub fn pause(&mut self, seconds: u32, now: Instant) -> Result<(), &'static str> {
        if seconds > 24 * 60 * 60 { return Err("Pause exceeds 24 hours"); }
        self.paused_until = if seconds == 0 { None } else {
            Some(now.checked_add(Duration::from_secs(u64::from(seconds))).ok_or("Pause deadline overflow")?)
        };
        if seconds != 0 { self.hide_popups(); }
        Ok(())
    }
    pub fn popup_snapshot(&self) -> PopupSnapshot<'_> { self.popup_at(Instant::now()) }
    fn popup_at(&self, now: Instant) -> PopupSnapshot<'_> {
        let dnd=self.suppressed(now);
        let notice = if dnd { None } else { self.active.iter().filter(|n| n.popup_allowed).max_by_key(|n| n.id) };
        let mut eligible: Vec<_> = if dnd { Vec::new() } else { self.active.iter().filter(|n| n.popup_allowed).collect() };
        eligible.sort_unstable_by_key(|n| std::cmp::Reverse(n.revision));
        let queued = eligible.len().saturating_sub(MAX_POPUPS);
        eligible.truncate(MAX_POPUPS);
        PopupSnapshot { schema: 1, instance: &self.instance, last_id: self.next_id, latest_revision: self.revision,
            dnd, manual_dnd:self.dnd, pause_remaining_seconds:self.remaining(now), notice, cards:eligible, queued }
    }
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
    fn dnd_does_not_replay_suppressed_notices() {
        let mut store = Store::default(); let now = Instant::now();
        add(&mut store, 0, false, 0, now);
        assert!(store.popup_snapshot().notice.is_some());
        store.set_dnd(true);
        add(&mut store, 0, false, 0, now);
        assert!(store.popup_snapshot().notice.is_none());
        store.set_dnd(false);
        assert!(store.popup_snapshot().notice.is_none());
        let id = add(&mut store, 0, false, 0, now);
        assert_eq!(store.popup_snapshot().notice.unwrap().id, id);
    }
    #[test]
    fn progress_replacement_stack_and_dnd() {
        let mut store = Store::default(); let now = Instant::now();
        let first = add(&mut store, 0, false, 0, now);
        for _ in 0..4 { add(&mut store, 0, false, 0, now); }
        let details = Details::parse(vec![], Some(72), "package".into(), None);
        assert_eq!(store.notify_with(first, "app".into(), "updated".into(), "".into(), false, 0, now, details).unwrap(), first);
        let popup = store.popup_snapshot();
        assert_eq!(popup.cards.len(), 3);
        assert_eq!(popup.queued, 2);
        assert_eq!(popup.cards[0].id, first);
        assert_eq!(popup.cards[0].progress, Some(72));
        store.set_dnd(true);
        assert!(store.popup_snapshot().cards.is_empty());
        store.set_dnd(false);
        assert!(store.popup_snapshot().cards.is_empty());
    }
    #[test]
    fn actions_are_revision_bound_one_shot_and_owner_scoped() {
        let mut store = Store::default(); let now = Instant::now();
        let details = || Details::parse(vec!["view".into(),"View".into(),"view".into(),"Duplicate".into()], Some(101), "".into(), Some(":1.2".into()));
        let id = store.notify_with(0,"app".into(),"a".into(),"".into(),false,0,now,details()).unwrap();
        let revision = store.active[0].revision;
        let instance = store.instance.clone();
        assert_eq!(store.active[0].actions.len(),1);
        assert_eq!(store.active[0].progress,None);
        assert!(!store.close_from(id,Some(":1.3")));
        let other = add(&mut store,id,false,0,now);
        assert_ne!(other,id);
        assert_eq!(store.notify_with(id,"app".into(),"b".into(),"".into(),false,0,now,details()).unwrap(),id);
        assert!(store.invoke(&instance,id,revision,"view",now).is_none());
        let revision = store.active[0].revision;
        assert!(store.invoke("stale",id,revision,"view",now).is_none());
        assert!(store.invoke(&instance,id,revision,"wrong",now).is_none());
        assert_eq!(store.invoke(&instance,id,revision,"view",now).as_deref(),Some(":1.2"));
        assert!(store.invoke(&instance,id,revision,"view",now).is_none());
        assert!(store.close_from(id,Some(":1.2")));
        assert!(store.history[0].actions.is_empty());
    }
    #[test]
    fn temporary_pause_expires_without_replay_and_preserves_manual_dnd() {
        let mut store=Store::default();let now=Instant::now();
        let old=add(&mut store,0,false,0,now);
        store.pause(60,now).unwrap();
        assert_eq!(store.popup_at(now).pause_remaining_seconds,60);
        let missed=add(&mut store,0,false,0,now);
        assert!(store.popup_at(now).dnd);
        assert!(!store.active.iter().find(|n|n.id==old).unwrap().missed);
        assert!(store.active.iter().find(|n|n.id==missed).unwrap().missed);
        let later=now+Duration::from_secs(60);
        assert!(!store.popup_at(later).dnd);
        assert!(store.popup_at(later).cards.is_empty());
        let fresh=add(&mut store,0,false,0,later);
        assert_eq!(store.popup_at(later).cards[0].id,fresh);
        store.set_dnd(true);
        store.pause(10,later).unwrap();
        store.pause(0,later).unwrap();
        assert!(store.popup_at(later).dnd);
        assert!(store.popup_at(later).manual_dnd);
        store.set_dnd(false);
        assert!(!store.popup_at(later).dnd);
        assert!(store.pause(86401,later).is_err());
        assert!(store.close(missed,2));
        assert!(store.history[0].missed);
    }
    #[test]
    fn application_policy_suppresses_and_excludes_history_without_replay(){
        let mut store=Store::default();let now=Instant::now();
        let old=add(&mut store,0,false,0,now);
        let muted=Preferences::decode(b"[applications.app]\npopups=false\nhistory=false\n").unwrap();
        store.apply_preferences(muted);
        assert!(store.popup_at(now).cards.is_empty());
        let id=add(&mut store,0,false,0,now);
        assert!(store.active.iter().find(|n|n.id==id).unwrap().missed);
        let other=store.notify(0,"other".into(),"other".into(),"".into(),false,0,now).unwrap();
        assert_eq!(store.popup_at(now).cards[0].id,other);
        store.apply_preferences(Preferences::default());
        assert_eq!(store.popup_at(now).cards.len(),1);
        assert!(store.close(old,2));assert!(store.close(id,2));assert!(store.history.is_empty());
        let fresh=add(&mut store,0,false,0,now);
        assert!(store.popup_at(now).cards.iter().any(|n|n.id==fresh));
        assert!(store.close(fresh,2));assert_eq!(store.history.len(),1);
    }
    #[test]
    fn rejects_over_capacity_and_unknown_close() {
        let mut store = Store::default(); let now = Instant::now();
        for _ in 0..MAX_ACTIVE { add(&mut store, 0, false, 0, now); }
        assert!(store.notify(0, "".into(), "".into(), "".into(), false, 0, now).is_err());
        assert!(!store.close(9999, 3));
    }
}
