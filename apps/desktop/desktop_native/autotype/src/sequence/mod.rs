//! KeePass-style auto-type of a selected vault item into the previously active window.
//!
//! The renderer parses the item's sequence (`{USERNAME}{TAB}{PASSWORD}{ENTER}`, ...) and resolves
//! its placeholders into [`SequenceStep`]s. This module turns the steps into key strokes
//! ([`plan`], platform independent and unit tested) and sends them to the foreground window
//! (Windows only).

use anyhow::{anyhow, Result};

#[cfg(windows)]
mod windows;

/// Default pause before every key event, as in KeePass 2.x (`SendInputEx.SendPriv`: a Windows time
/// slice boundary + 1, + 2 = 35 ms). Faster typing makes some apps drop or reorder keys, e.g. a Tab
/// whose focus change hasn't happened before the next character arrives.
pub const DEFAULT_KEY_DELAY_MS: u32 = 35;

/// KeePass waits the key delay once more after a Tab when the delay is below this.
const EXTRA_TAB_DELAY_BELOW_MS: u32 = 100;

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
/// Like KeePass, every key event (including each modifier press and release) is preceded by the
/// key delay, and a Tab is followed by one more key delay.
///
/// `char_to_key` maps a character to the virtual key and modifiers that produce it on the active
/// keyboard layout (Windows: `VkKeyScanW`); it is only used for [`SequenceStep::Char`] with
/// modifiers.
pub fn plan<F>(steps: &[SequenceStep], char_to_key: F) -> Result<Vec<Action>>
where
    F: Fn(char) -> Option<(u16, Modifiers)>,
{
    let mut planner = Planner {
        actions: Vec::new(),
        key_delay: DEFAULT_KEY_DELAY_MS,
        first: true,
    };

    for step in steps {
        match step {
            SequenceStep::Text(text) => {
                for unit in text.encode_utf16() {
                    match unit {
                        0x0D => {}
                        0x09 => planner.key(VK_TAB, Modifiers::default()),
                        0x0A => planner.key(VK_RETURN, Modifiers::default()),
                        _ => planner.stroke(unicode(&[unit])),
                    }
                }
            }
            SequenceStep::Key { vk, modifiers } => planner.key(*vk, *modifiers),
            SequenceStep::Char { ch, modifiers } => {
                if modifiers.is_empty() {
                    let mut buf = [0u16; 2];
                    planner.stroke(unicode(ch.encode_utf16(&mut buf)));
                } else {
                    let (vk, layout_modifiers) = char_to_key(*ch).ok_or_else(|| {
                        // The character may come from a password, so it is not in the message.
                        anyhow!(
                            "A character in the sequence can't be typed with modifier keys on \
                             this keyboard layout"
                        )
                    })?;
                    planner.key(vk, modifiers.union(layout_modifiers));
                }
            }
            SequenceStep::Delay(ms) => planner.actions.push(Action::Sleep((*ms).min(MAX_DELAY_MS))),
            SequenceStep::KeyDelay(ms) => planner.key_delay = (*ms).min(MAX_DELAY_MS),
        }
    }

    Ok(planner.actions)
}

struct Planner {
    actions: Vec<Action>,
    key_delay: u32,
    first: bool,
}

impl Planner {
    fn sleep(&mut self) {
        if self.key_delay > 0 {
            self.actions.push(Action::Sleep(self.key_delay));
        }
    }

    fn stroke(&mut self, events: Vec<KeyEvent>) {
        if !self.first {
            self.sleep();
        }
        self.first = false;
        self.actions.push(Action::Stroke(events));
    }

    /// Modifier downs, the key, then the modifiers released in reverse order, each a stroke.
    fn key(&mut self, vk: u16, modifiers: Modifiers) {
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

        for &m in &held {
            self.stroke(vec![KeyEvent::Virtual { vk: m, down: true }]);
        }
        self.stroke(vec![
            KeyEvent::Virtual { vk, down: true },
            KeyEvent::Virtual { vk, down: false },
        ]);
        for &m in held.iter().rev() {
            self.stroke(vec![KeyEvent::Virtual { vk: m, down: false }]);
        }
        if vk == VK_TAB && self.key_delay < EXTRA_TAB_DELAY_BELOW_MS {
            self.sleep();
        }
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

    const D: u32 = DEFAULT_KEY_DELAY_MS;

    fn no_layout(_: char) -> Option<(u16, Modifiers)> {
        None
    }

    fn uni(unit: u16) -> Action {
        Action::Stroke(unicode(&[unit]))
    }

    fn vk(vk: u16, down: bool) -> Action {
        Action::Stroke(vec![KeyEvent::Virtual { vk, down }])
    }

    fn tap(key: u16) -> Action {
        Action::Stroke(vec![
            KeyEvent::Virtual {
                vk: key,
                down: true,
            },
            KeyEvent::Virtual {
                vk: key,
                down: false,
            },
        ])
    }

    #[test]
    fn text_waits_before_each_key_and_once_more_after_tab() {
        let actions = plan(&[SequenceStep::Text("a\tb\r\n".into())], no_layout).unwrap();
        assert_eq!(
            actions,
            vec![
                uni('a' as u16),
                Action::Sleep(D),
                tap(VK_TAB),
                Action::Sleep(D),
                Action::Sleep(D),
                uni('b' as u16),
                Action::Sleep(D),
                tap(VK_RETURN),
            ]
        );
    }

    #[test]
    fn modifiers_are_separate_delayed_events_released_in_reverse_order() {
        let modifiers = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let steps = [SequenceStep::Key {
            vk: 0x23,
            modifiers,
        }];
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![
                vk(VK_CONTROL, true),
                Action::Sleep(D),
                vk(VK_SHIFT, true),
                Action::Sleep(D),
                tap(0x23),
                Action::Sleep(D),
                vk(VK_SHIFT, false),
                Action::Sleep(D),
                vk(VK_CONTROL, false),
            ]
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
            vec![
                vk(VK_MENU, true),
                vk(VK_SHIFT, true),
                tap(0x31),
                vk(VK_SHIFT, false),
                vk(VK_MENU, false),
            ]
        );
    }

    #[test]
    fn char_with_modifiers_fails_when_the_layout_has_no_key_for_it() {
        let modifiers = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let steps = [SequenceStep::Char {
            ch: 'é', modifiers
        }];
        assert!(plan(&steps, no_layout).is_err());
    }

    #[test]
    fn plain_char_outside_the_bmp_is_sent_as_a_surrogate_pair() {
        let steps = [SequenceStep::Char {
            ch: '😀',
            modifiers: Modifiers::default(),
        }];
        let mut expected = unicode(&[0xD83D]);
        expected.extend(unicode(&[0xDE00]));
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![Action::Stroke(expected)]
        );
    }

    #[test]
    fn delays_are_capped_and_key_delay_changes_later_pauses() {
        let steps = [
            SequenceStep::Text("x".into()),
            SequenceStep::Delay(MAX_DELAY_MS + 1),
            SequenceStep::KeyDelay(150),
            SequenceStep::Text("\ty".into()),
        ];
        assert_eq!(
            plan(&steps, no_layout).unwrap(),
            vec![
                uni('x' as u16),
                Action::Sleep(MAX_DELAY_MS),
                Action::Sleep(150),
                tap(VK_TAB),
                Action::Sleep(150),
                uni('y' as u16),
            ]
        );
    }
}
