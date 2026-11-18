//! Windows side of [`super`]: focusing the previous window and sending key strokes.
//!
//! Modelled on KeePass (`NativeMethods.LoseFocus`, `EnsureForegroundWindow`, `SiEngineWin`): the
//! target is the next real top-level window below this app in the Z order, and modifier keys still
//! held from the in-app shortcut are released before typing.

use std::{
    ffi::c_void,
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
    System::Threading::GetCurrentProcessId,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0,
            INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowTextLengthW,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE,
            GW_HWNDNEXT, WS_EX_TOOLWINDOW,
        },
    },
};

use super::{Action, KeyEvent, Modifiers};

/// KeePass waits this long after switching windows before it types (`TargetActivationDelay`).
const TARGET_ACTIVATION_DELAY: Duration = Duration::from_millis(100);
const FOREGROUND_TIMEOUT: Duration = Duration::from_secs(1);
/// How long to wait for the user to let go of the in-app shortcut (Ctrl+Shift+V) before typing.
const KEY_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause after the keys were released (or force-released) before the first key is sent.
const SETTLE_DELAY: Duration = Duration::from_millis(50);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Windows that are never a typing target: the taskbars and the desktop.
const SKIPPED_CLASSES: [&str; 4] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Progman",
    "WorkerW",
];

/// Left/right Shift, Ctrl, Alt, then the Windows keys.
const MODIFIER_VKS: [u16; 8] = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C];

pub(super) fn activate_previous_window() -> Result<()> {
    let own_pid = unsafe { GetCurrentProcessId() };
    let start = unsafe { GetForegroundWindow() };
    if start.0.is_null() {
        return Err(anyhow!("No window is in the foreground"));
    }

    let mut hwnd = start;
    let target = loop {
        hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }
            .map_err(|_| anyhow!("No window to auto-type into was found"))?;
        if hwnd.0.is_null() {
            return Err(anyhow!("No window to auto-type into was found"));
        }
        if is_target_candidate(hwnd, own_pid) {
            break hwnd;
        }
    };

    if !unsafe { SetForegroundWindow(target) }.as_bool() {
        return Err(anyhow!("Windows refused to switch to the target window"));
    }

    let deadline = Instant::now() + FOREGROUND_TIMEOUT;
    loop {
        let current = unsafe { GetForegroundWindow() };
        // Some apps hand the focus on to another of their windows, so any other window counts.
        if current == target || (!current.0.is_null() && current != start) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("The target window did not come to the foreground"));
        }
        sleep(POLL_INTERVAL);
    }

    sleep(TARGET_ACTIVATION_DELAY);
    debug!("Previous window activated for auto-type.");
    Ok(())
}

fn is_target_candidate(hwnd: HWND, own_pid: u32) -> bool {
    // SAFETY (each call): these only query the given window; a stale handle yields 0/false.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() || is_cloaked(hwnd) {
            return false;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own_pid {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class);
        let class = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
        !SKIPPED_CLASSES.contains(&class.as_str())
    }
}

/// A cloaked window is composed but not shown (e.g. a suspended packaged app).
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: the out-buffer is `cloaked`, whose size is passed explicitly.
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast::<c_void>(),
            std::mem::size_of_val(&cloaked) as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

/// The virtual key and modifiers producing `ch` on the active keyboard layout.
pub(super) fn char_to_key(ch: char) -> Option<(u16, Modifiers)> {
    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    if units.len() != 1 {
        return None;
    }
    // SAFETY: plain lookup in the current thread's keyboard layout.
    let scan = unsafe { VkKeyScanW(units[0]) };
    if scan == -1 {
        return None;
    }
    let [vk, state] = (scan as u16).to_le_bytes();
    Some((
        u16::from(vk),
        Modifiers {
            shift: state & 1 != 0,
            ctrl: state & 2 != 0,
            alt: state & 4 != 0,
            win: false,
        },
    ))
}

pub(super) fn send(actions: &[Action]) -> Result<()> {
    wait_for_key_release();

    // Modifiers this sequence pressed and hasn't released yet (e.g. inside `^{END}`).
    let mut pressed_by_us = 0usize;
    for action in actions {
        match action {
            Action::Stroke(events) => {
                if pressed_by_us == 0 {
                    release_stray_modifiers();
                }
                for event in events {
                    if let KeyEvent::Virtual { vk, down } = *event {
                        if is_modifier(vk) {
                            if down {
                                pressed_by_us += 1;
                            } else {
                                pressed_by_us = pressed_by_us.saturating_sub(1);
                            }
                        }
                    }
                }
                let inputs: Vec<INPUT> = events.iter().map(to_input).collect();
                send_inputs(&inputs)?;
            }
            Action::Sleep(ms) => sleep(Duration::from_millis(u64::from(*ms))),
        }
    }
    Ok(())
}

fn is_down(vk: u16) -> bool {
    // SAFETY: plain query of the asynchronous key state.
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Waits until the user has let go of every key (the shortcut's Ctrl, Shift and V), otherwise the
/// first keys would arrive as Ctrl+Shift+<key> (a Tab becomes Shift+Tab or Ctrl+Tab). Modifiers
/// still down after the timeout are released.
fn wait_for_key_release() {
    // Keyboard keys only; 0x01-0x07 are mouse buttons.
    let any_key_down = || (0x08u16..=0xFE).any(is_down);

    let start = Instant::now();
    let mut waited = false;
    while any_key_down() && start.elapsed() < KEY_RELEASE_TIMEOUT {
        waited = true;
        sleep(POLL_INTERVAL);
    }
    if waited {
        info!(
            waited_ms = start.elapsed().as_millis() as u64,
            "Auto-type waited for keys to be released."
        );
    }
    let released = release_stray_modifiers();
    if waited || released > 0 {
        sleep(SETTLE_DELAY);
    }
}

/// Releases modifier keys that are down although this sequence didn't press them (still held
/// from the shortcut, or pressed by the user while typing), like KeePass does before sending.
fn release_stray_modifiers() -> usize {
    let held: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .filter(|&&vk| is_down(vk))
        .map(|&vk| to_input(&KeyEvent::Virtual { vk, down: false }))
        .collect();
    if !held.is_empty() {
        warn!(
            count = held.len(),
            "Auto-type released modifier keys that were still held."
        );
        if let Err(e) = send_inputs(&held) {
            warn!(error = %e, "Could not release modifier keys.");
        }
        sleep(POLL_INTERVAL);
    }
    held.len()
}

fn is_extended(vk: u16) -> bool {
    matches!(
        vk,
        0x21..=0x28 | 0x2C | 0x2D | 0x2E | 0x5B | 0x5C | 0x5D | 0x6F | 0x90 | 0xA3 | 0xA5
    )
}

fn to_input(event: &KeyEvent) -> INPUT {
    let (w_vk, w_scan, mut flags) = match *event {
        KeyEvent::Unicode { unit, .. } => (VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE),
        KeyEvent::Virtual { vk, .. } => {
            // SAFETY: plain lookup in the current keyboard layout.
            let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
            let flags = if is_extended(vk) {
                KEYEVENTF_EXTENDEDKEY
            } else {
                KEYBD_EVENT_FLAGS(0)
            };
            (VIRTUAL_KEY(vk), u16::try_from(scan).unwrap_or(0), flags)
        }
    };
    let down = match *event {
        KeyEvent::Unicode { down, .. } | KeyEvent::Virtual { down, .. } => down,
    };
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: w_vk,
                wScan: w_scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT]) -> Result<()> {
    let size = i32::try_from(std::mem::size_of::<INPUT>()).expect("INPUT size fits in i32");
    // SAFETY: `inputs` is a valid slice of keyboard INPUT structs.
    let sent = unsafe { SendInput(inputs, size) };
    if sent as usize != inputs.len() {
        return Err(anyhow!(
            "Windows blocked the simulated key presses (sent {sent} of {}). Auto-type can't type \
             into apps running as administrator unless Bitwarden does too.",
            inputs.len()
        ));
    }
    Ok(())
}
