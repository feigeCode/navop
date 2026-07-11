use super::{ActiveTurn, Session};
use crate::ids::TurnId;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct TurnState {
    active: Option<ActiveTurn>,
    cancelled: HashSet<TurnId>,
}

impl Session {
    pub fn set_active_turn(&self, turn: ActiveTurn) {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .active = Some(turn);
    }

    pub fn active_turn_id(&self) -> Option<TurnId> {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .active
            .as_ref()
            .map(|turn| turn.turn_id.clone())
    }

    pub fn clear_active_turn_if(&self, turn_id: &TurnId) -> bool {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        if turns
            .active
            .as_ref()
            .is_some_and(|turn| &turn.turn_id == turn_id)
        {
            turns.active = None;
            return true;
        }
        false
    }

    pub fn is_busy(&self) -> bool {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .active
            .is_some()
    }

    pub fn cancel_and_detach_active_turn(&self) -> Option<TurnId> {
        let mut turns = self.turns.lock().expect("session turn lock poisoned");
        let turn = turns.active.take()?;
        turn.cancel();
        let turn_id = turn.turn_id;
        turns.cancelled.insert(turn_id.clone());
        Some(turn_id)
    }

    pub fn cancel_active_turn(&self) {
        let _ = self.cancel_and_detach_active_turn();
    }

    pub fn is_turn_cancelled(&self, turn_id: &TurnId) -> bool {
        self.turns
            .lock()
            .expect("session turn lock poisoned")
            .cancelled
            .contains(turn_id)
    }

    pub(super) fn with_writable_turn<R>(
        &self,
        turn_id: &TurnId,
        operation: impl FnOnce() -> R,
    ) -> Option<R> {
        let turns = self.turns.lock().expect("session turn lock poisoned");
        if turns.cancelled.contains(turn_id) {
            return None;
        }
        Some(operation())
    }
}
