//! Scripted keyboard input: keys a test scenario presses, merged into the
//! frame's `InputState` after the platform snapshot.
//!
//! `Engine::frame` replaces `Engine::input` with the platform's snapshot at
//! the top of every frame, so a key written straight into `engine.input` by
//! the test runner (which runs after every update closure) was overwritten
//! before anything could read it.  Keys queued here are applied right after
//! that snapshot instead, so a press issued on frame N is seen by every
//! update closure (the guest) on frame N + 1.

use std::collections::BTreeSet;

use classic_platform::InputState;

/// One-shot key presses plus a persistent held set, applied to the frame's
/// input by [`ScriptedInput::apply`].
#[derive(Debug, Default)]
pub struct ScriptedInput {
    /// Keys reported `pressed` on the next frame only.
    pressed: Vec<String>,
    /// Keys reported `down` on every frame until released.
    held: BTreeSet<String>,
}

impl ScriptedInput {
    /// Press `key`: `pressed` on the next frame, `down` until [`Self::release`].
    pub fn press(&mut self, key: &str) {
        self.pressed.push(key.to_string());
        self.held.insert(key.to_string());
    }

    /// Release `key` from the held set (from the next frame on).
    pub fn release(&mut self, key: &str) {
        self.held.remove(key);
    }

    /// Whether a scripted key is currently held.
    pub fn is_held(&self, key: &str) -> bool {
        self.held.contains(key)
    }

    /// Merge into this frame's input: the held set as `down`, the queued
    /// presses as `pressed` (and `down`), consuming the queue.
    pub fn apply(&mut self, input: &mut InputState) {
        for key in &self.held {
            input.keys_down.insert(key.clone(), true);
        }
        for key in self.pressed.drain(..) {
            input.keys_down.insert(key.clone(), true);
            input.keys_pressed.insert(key, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_is_one_shot_and_hold_persists_until_release() {
        let mut s = ScriptedInput::default();
        s.press("KeyN");
        let mut f1 = InputState::new();
        s.apply(&mut f1);
        assert!(f1.was_key_pressed("KeyN") && f1.is_key_down("KeyN"));
        let mut f2 = InputState::new();
        s.apply(&mut f2);
        assert!(!f2.was_key_pressed("KeyN"), "pressed is one frame only");
        assert!(f2.is_key_down("KeyN"), "still held");
        s.release("KeyN");
        let mut f3 = InputState::new();
        s.apply(&mut f3);
        assert!(!f3.is_key_down("KeyN") && !f3.was_key_pressed("KeyN"));
    }

    #[test]
    fn a_press_after_release_in_the_same_frame_still_fires_once() {
        // A tap (press then release before the next frame) is still seen.
        let mut s = ScriptedInput::default();
        s.press("Space");
        s.release("Space");
        let mut f1 = InputState::new();
        s.apply(&mut f1);
        assert!(f1.was_key_pressed("Space"));
        let mut f2 = InputState::new();
        s.apply(&mut f2);
        assert!(!f2.is_key_down("Space"));
    }
}
