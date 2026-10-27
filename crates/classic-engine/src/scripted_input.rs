//! Scripted keyboard input: keys a test scenario presses, merged into the
//! frame's `InputState` after the platform snapshot.
//!
//! `Engine::frame` replaces `Engine::input` with the platform's snapshot at
//! the top of every frame, so a key written straight into `engine.input` by
//! the test runner (which runs after every update closure) was overwritten
//! before anything could read it.  Keys queued here are applied right after
//! that snapshot instead, so a press issued on frame N is seen by every
//! update closure (the guest) on frame N + 1.
//!
//! Mouse clicks go the same way ([`ScriptedInput::click`]): the button is
//! pressed on the next frame and released on the one after, at a pointer
//! position that then stays put, so a scripted click runs the same input
//! path a player's does (RTS point-select on release, the guest's click
//! handling on press).

use std::collections::{BTreeSet, VecDeque};

use glam::Vec2;

use classic_platform::InputState;

/// One-shot key presses plus a persistent held set, applied to the frame's
/// input by [`ScriptedInput::apply`].
#[derive(Debug, Default)]
pub struct ScriptedInput {
    /// Keys reported `pressed` on the next frame only.
    pressed: Vec<String>,
    /// Keys reported `down` on every frame until released.
    held: BTreeSet<String>,
    /// Queued clicks `(position, button, released yet)`, one at a time.
    clicks: VecDeque<(Vec2, usize, bool)>,
    /// The scripted pointer position, held once a click has set it.
    pointer: Option<Vec2>,
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

    /// Click mouse `button` at screen pixel `pos`: pressed on the next frame,
    /// released on the one after (clicks queue behind each other).
    pub fn click(&mut self, pos: Vec2, button: usize) {
        if button < 3 {
            self.clicks.push_back((pos, button, false));
        }
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
        if let Some((pos, b, released)) = self.clicks.front_mut() {
            self.pointer = Some(*pos);
            if *released {
                input.mouse_down[*b] = false;
                input.mouse_released[*b] = true;
                self.clicks.pop_front();
            } else {
                input.mouse_down[*b] = true;
                input.mouse_pressed[*b] = true;
                input.frame_had_click |= *b == 0;
                *released = true;
            }
        }
        if let Some(p) = self.pointer {
            input.mouse_pos = p;
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
    fn a_click_presses_then_releases_and_keeps_the_pointer() {
        let mut s = ScriptedInput::default();
        s.click(Vec2::new(30.0, 40.0), 0);
        s.click(Vec2::new(5.0, 6.0), 2);
        let mut f = [InputState::new(), InputState::new(), InputState::new()];
        let mut f4 = InputState::new();
        for i in &mut f {
            s.apply(i);
        }
        s.apply(&mut f4);
        assert!(f[0].was_mouse_pressed(0) && f[0].is_mouse_down(0) && f[0].frame_had_click);
        assert!(f[1].was_mouse_released(0) && !f[1].is_mouse_down(0));
        assert_eq!(f[1].mouse_pos, Vec2::new(30.0, 40.0));
        assert!(f[2].was_mouse_pressed(2) && !f[2].frame_had_click);
        assert!(f4.was_mouse_released(2));
        let mut f5 = InputState::new();
        s.apply(&mut f5);
        assert!(!f5.was_mouse_released(2) && f5.mouse_pos == Vec2::new(5.0, 6.0));
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
