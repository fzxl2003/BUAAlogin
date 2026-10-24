use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Cancellation {
    pub(crate) generation: Arc<AtomicU64>,
    pub(crate) active: u64,
    deadline: Option<Instant>,
}
impl Cancellation {
    pub fn new(generation: Arc<AtomicU64>, active: u64) -> Self {
        Self {
            generation,
            active,
            deadline: None,
        }
    }
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;
        Self {
            deadline: Some(self.deadline.map_or(deadline, |d| d.min(deadline))),
            ..self.clone()
        }
    }
    pub fn check(&self) -> Result<(), String> {
        if self.generation.load(Ordering::SeqCst) != self.active {
            Err("已取消".into())
        } else if self.deadline.is_some_and(|d| Instant::now() >= d) {
            Err("操作超时".into())
        } else {
            Ok(())
        }
    }
    pub fn remaining(&self) -> Result<Option<Duration>, String> {
        self.check()?;
        Ok(self
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now())))
    }
    pub fn wait(&self, duration: Duration) -> Result<(), String> {
        let end = Instant::now() + duration;
        loop {
            self.check()?;
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(50)));
        }
    }
}
