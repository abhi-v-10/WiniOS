//! Session-only sharing control. Contains no clipboard data or credentials.
use tokio::sync::watch;

#[derive(Clone, Copy, Default)]
struct State {
    paused: bool,
    pause_generation: u64,
}

#[derive(Clone)]
pub struct SharingControl(watch::Sender<State>);

impl Default for SharingControl {
    fn default() -> Self {
        Self(watch::channel(State::default()).0)
    }
}
impl SharingControl {
    pub fn is_paused(&self) -> bool {
        self.0.borrow().paused
    }
    pub fn set_paused(&self, paused: bool) {
        self.0.send_modify(|state| {
            if paused && !state.paused {
                state.pause_generation = state.pause_generation.wrapping_add(1);
            }
            state.paused = paused;
        });
    }
    pub fn generation(&self) -> u64 {
        self.0.borrow().pause_generation
    }
    /// Also detects a rapid pause/resume while an admitted request is in flight.
    pub async fn paused_since(&self, generation: u64) {
        let mut receiver = self.0.subscribe();
        loop {
            if receiver.borrow_and_update().pause_generation != generation {
                return;
            }
            if receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pause_generation_survives_rapid_resume_without_persisting_state() {
        let control = SharingControl::default();
        let generation = control.generation();
        assert!(!control.is_paused());
        control.set_paused(true);
        control.set_paused(false);
        assert!(!control.is_paused());
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            control.paused_since(generation),
        )
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                control.paused_since(control.generation())
            )
            .await
            .is_err()
        );
    }
}
