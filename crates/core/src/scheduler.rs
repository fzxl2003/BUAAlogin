//! Clock-independent control transitions; caller supplies monotonic timestamps.
use crate::interfaces::Interface;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
#[derive(Default, Debug)]
pub(crate) struct Changes {
    all: bool,
    names: BTreeSet<String>,
}
impl Changes {
    fn merge(&mut self, names: Vec<String>) {
        if names.is_empty() {
            self.all = true;
        }
        self.names.extend(names);
    }
    pub fn affects(&self, interface: &Interface) -> bool {
        self.all
            || self.names.contains(&interface.name)
            || self.names.contains(&format!("#{}", interface.index))
    }
}
#[derive(Default)]
pub(crate) struct Controls {
    pub suspended: bool,
    pub deadline: Option<Instant>,
    pending: Changes,
}
impl Controls {
    pub fn network(&mut self, now: Instant, names: Vec<String>) {
        if self.suspended {
            return;
        }
        self.pending.merge(names);
        self.deadline = Some(now + Duration::from_secs(2));
    }
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.deadline = None;
        self.pending = Changes::default();
    }
    pub fn resume(&mut self, now: Instant) {
        self.suspended = false;
        self.network(now, vec![]);
    }
    pub fn refresh(&mut self, now: Instant) -> Option<Changes> {
        if self.suspended || self.deadline.is_none_or(|d| d > now) {
            return None;
        }
        self.deadline = None;
        Some(std::mem::take(&mut self.pending))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn iface(name: &str, index: u32) -> Interface {
        Interface {
            name: name.into(),
            index,
            address: "10.1.2.3".parse().unwrap(),
        }
    }
    #[test]
    fn notifications_merge_and_debounce_with_virtual_time() {
        let now = Instant::now();
        let mut c = Controls::default();
        c.network(now, vec!["en0".into()]);
        c.network(now + Duration::from_secs(1), vec!["#3".into()]);
        assert!(c.refresh(now + Duration::from_secs(2)).is_none());
        let changes = c.refresh(now + Duration::from_secs(3)).unwrap();
        assert!(changes.affects(&iface("en0", 2)));
        assert!(changes.affects(&iface("en1", 3)));
        assert!(!changes.affects(&iface("en2", 4)));
        assert!(c.refresh(now + Duration::from_secs(4)).is_none());
    }
    #[test]
    fn sleep_cancels_pending_work_and_resume_checks_once() {
        let now = Instant::now();
        let mut c = Controls::default();
        c.network(now, vec![]);
        c.suspend();
        c.network(now + Duration::from_secs(3), vec![]);
        assert!(c.deadline.is_none());
        assert!(c.refresh(now + Duration::from_secs(3600)).is_none());
        c.resume(now + Duration::from_secs(3600));
        assert!(c.refresh(now + Duration::from_secs(3601)).is_none());
        assert!(c
            .refresh(now + Duration::from_secs(3602))
            .unwrap()
            .affects(&iface("en0", 2)));
        assert!(c.refresh(now + Duration::from_secs(7200)).is_none());
    }
}
