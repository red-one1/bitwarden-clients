//! KeePass-style auto-type of a selected vault item into the previously active window.
//!
//! The renderer parses the item's sequence (`{USERNAME}{TAB}{PASSWORD}{ENTER}`, ...) and resolves
//! its placeholders into [`SequenceStep`]s. This module turns the steps into key strokes
//! ([`plan`], platform independent and unit tested) and sends them to the foreground window
//! (Windows only).

use anyhow::{anyhow, Result};

#[cfg(windows)]
mod windows;

/// Default pause between two key strokes, as in KeePass.
pub const DEFAULT_KEY_DELAY_MS: u32 = 10;

/// Upper bound for a single `{DELAY n}` or `{DELAY=n}`, so a typo can't hang the app.
pub const MAX_DELAY_MS: u32 = 60_000;

/// Modifier keys held while a key is pressed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
}

impl Modifiers {
    fn union(self, other: Modifiers) -> Modifiers {
        Modifiers {
            shift: self.shift || other.shift,
            ctrl: self.ctrl || other.ctrl,
            alt: self.alt || other.alt,
            win: self.win || other.win,
        }
    }

    fn is_empty(self) -> bool {
        self == Modifiers::default()
    }
}

/// One step of an auto-type sequence, already resolved by the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceStep {
    /// Literal text. `\t` is sent as Tab, `\n` as Enter, `\r` is skipped.
    Text(String),
    /// A virtual key (Windows `VK_*` code), e.g. `{ENTER}`, `{F5}` or `^{END}`.
    Key { vk: u16, modifiers: Modifiers },
    /// The key producing `ch` on the current keyboard layout, pressed with modifiers, e.g. `^a`.
    Char { ch: char, modifiers: Modifiers },
    /// `{DELAY n}`: pause once.
    Delay(u32),
    /// `{DELAY=n}`: pause between key strokes from here on.
    KeyDelay(u32),
}

/// A single key transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// A UTF-16 code unit sent as a Unicode packet (independent of layout and Caps Lock).
    Unicode { unit: u16, down: bool },
    /// A virtual key.
    Virtual { vk: u16, down: bool },
}

/// What the sender does, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Key events sent together in one `SendInput` call.
    Stroke(Vec<KeyEvent>),
    Sleep(u32),
}

pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;

/// Turns resolved steps into key strokes and pauses.
///
/// `char_to_key` maps a character to the virtual key and modifiers that produce it on the active
/// keyboard layout (Windows: `VkKeyScanW`); it is only used for [`SequenceStep::Char`] with
/// modifiers.
pub fn plan<F>(steps: &[SequenceStep], char_to_key: F) -> Result<Vec<Action>>
where
    F: Fn(char) -> Option<(u16, Modifiers)>,
{
    let mut actions = Vec::new();
    let mut key_delay = DEFAULT_KEY_DELAY_MS;

    for step in steps {
        match step {
            SequenceStep::Text(text) => {
                for unit in text.encode_utf16() {
                    let events = match unit {
                        0x0D => continue,
                        0x09 => press(VK_TAB, Modifiers::default()),
                        0x0A => press(VK_RETURN, Modifiers::default()),
                        _ => unicode(&[unit]),
                    };
                    push_stroke(&mut actions, events, key_delay);
                }
            }
            SequenceStep::Key { vk, modifiers } => {
                push_stroke(&mut actions, press(*vk, *modifiers), key_delay);
            }
            SequenceStep::Char { ch, modifiers } => {
                let events = if modifiers.is_empty() {
                    let mut buf = [0u16; 2];
                    unicode(ch.encode_utf16(&mut buf))
                } else {
                    let (vk, layout_modifiers) = char_to_key(*ch).ok_or_else(|| {
                        // The character may come from a password, so it is not in the message.
                        anyhow!(
                            "A character in the sequence can't be typed with modifier keys on \
                             this keyboard layout"
                        )
                    })?;
                    press(vk, modifiers.union(layout_modifiers))
                };
                push_stroke(&mut actions, events, key_delay);
            }
            SequenceStep::Delay(ms) => actions.push(Action::Sleep((*ms).min(MAX_DELAY_MS))),
            SequenceStep::KeyDelay(ms) => key_delay = (*ms).min(MAX_DELAY_MS),
        }
    }

    Ok(actions)
}

fn push_stroke(actions: &mut Vec<Action>, events: Vec<KeyEvent>, key_delay: u32) {
    actions.push(Action::Stroke(events));
    if key_delay > 0 {
        actions.push(Action::Sleep(key_delay));
    }
}

fn unicode(units: &[u16]) -> Vec<KeyEvent> {
    units
        .iter()
        .flat_map(|&unit| {
            [
                KeyEvent::Unicode { unit, down: true },
                KeyEvent::Unicode { unit, down: false },
            ]
        })
        .collect()
}

/// Modifier key-downs, the key down and up, then the modifiers released in reverse order.
fn press(vk: u16, modifiers: Modifiers) -> Vec<KeyEvent> {
    let held: Vec<u16> = [
        (modifiers.ctrl, VK_CONTROL),
        (modifiers.alt, VK_MENU),
        (modifiers.shift, VK_SHIFT),
        (modifiers.win, VK_LWIN),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, key)| *key)
    .collect();

    let mut events = Vec::with_capacity(held.len() * 2 + 2);
    events.extend(held.iter().map(|&vk| KeyEvent::Virtual { vk, down: true }));
    events.push(KeyEvent::Virtual { vk, down: true });
    events.push(KeyEvent::Virtual { vk, down: false });
    events.extend(
        held.iter()
            .rev()
            .map(|&vk| KeyEvent::Virtual { vk, down: false }),
    );
    events
}

/// Gives the focus to the window below this app's window, like KeePass's "Perform Auto-Type" on
/// a selected entry. Returns once that window is in the foreground.
pub fn activate_previous_window() -> Result<()> {
    #[cfg(windows)]
    {
        windows::activate_previous_window()
    }
    #[cfg(not(windows))]
    {
        Err(anyhow!(
            "Auto-type of the selected item is only supported on Windows"
        ))
    }
}

/// Types the steps into the foreground window.
pub fn type_sequence(steps: &[SequenceStep]) -> Result<()> {
    #[cfg(windows)]
    {
        let actions = plan(steps, windows::char_to_key)?;
        windows::send(&actions)
    }
    #[cfg(not(windows))]
    {
        let _ = steps;
        Err(anyhow!(
            "Auto-type of the selected item is only supported on Windows"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_layout(_: char) -> Option<(u16, Modifiers)> {
        None
    }

    fn uni(unit: u16) -> Vec<KeyEvent> {
        unicode(&[unit])
    }

    fn vk(vk: u16, down: bool) -> KeyEvent {
        KeyEvent::Virtual { vk, down }
    }

    #[test]
    fn text_is_typed_as_unicode_with_tab_and_enter_as_keys() {
        let actions = plan(&[SequenceStep::Text("a\tb\r\n".into())], no_layout).unwrap();
        assert_eq!(
            actions,
            vec![
                Action::Stroke(uni('a' as u16)),
                Action::Sleep(DEFAULT_KEY_DELAY_MS),
                Action::Stroke(vec![vk(VK_TAB, true), vk(VK_TAB, false)]),
                Action::Sleep(DEFAULT_KEY_DELAY_MS),
                Action::Stroke(uni('b' as u16)),
                Action::Sleep(DEFAULT_KEY_DELAY_MS),
                Action::Stroke(vec![vk(VK_RETURN, true), vk(VK_RETURN, false)]),
                Action::Sleep(DEFAULT_KEY_DELAY_MS),
            ]
        );
    }

    #[test]
    fn key_with_modifiers_presses_and_releases_them_in_reverse_order() {
        let modifiers = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let steps = [
            SequenceStep::KeyDelay(0),
            SequenceStep::Key {
                vk: 0x23,
                modifiers,
            },
        ];
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![Action::Stroke(vec![
                vk(VK_CONTROL, true),
                vk(VK_SHIFT, true),
                vk(0x23, true),
                vk(0x23, false),
                vk(VK_SHIFT, false),
                vk(VK_CONTROL, false),
            ])]
        );
    }

    #[test]
    fn char_with_modifiers_uses_the_layout_and_merges_its_shift_state() {
        let layout = |c: char| {
            (c == '!').then_some((
                0x31,
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ))
        };
        let modifiers = Modifiers {
            alt: true,
            ..Default::default()
        };
        let steps = [
            SequenceStep::KeyDelay(0),
            SequenceStep::Char { ch: '!', modifiers },
        ];
        assert_eq!(
            plan(&steps, layout).unwrap(),
            vec![Action::Stroke(vec![
                vk(VK_MENU, true),
                vk(VK_SHIFT, true),
                vk(0x31, true),
                vk(0x31, false),
                vk(VK_SHIFT, false),
                vk(VK_MENU, false),
            ])]
        );
    }

    #[test]
    fn char_with_modifiers_fails_when_the_layout_has_no_key_for_it() {
        let modifiers = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert!(plan(
            &[SequenceStep::Char {
                ch: 'é', modifiers
            }],
            no_layout
        )
        .is_err());
    }

    #[test]
    fn plain_char_outside_the_bmp_is_sent_as_a_surrogate_pair() {
        let steps = [
            SequenceStep::KeyDelay(0),
            SequenceStep::Char {
                ch: '😀',
                modifiers: Modifiers::default(),
            },
        ];
        let mut expected = uni(0xD83D);
        expected.extend(uni(0xDE00));
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![Action::Stroke(expected)]
        );
    }

    #[test]
    fn delays_are_capped_and_key_delay_applies_to_later_strokes() {
        let steps = [
            SequenceStep::Delay(MAX_DELAY_MS + 1),
            SequenceStep::KeyDelay(50),
            SequenceStep::Text("x".into()),
        ];
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![
                Action::Sleep(MAX_DELAY_MS),
                Action::Stroke(uni('x' as u16)),
                Action::Sleep(50),
            ]
        );
    }
}
