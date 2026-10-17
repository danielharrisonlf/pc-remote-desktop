//! Linux input simulation (`InputBackend`) for Ubuntu 22.04 / 24.04 LTS.
//!
//! Prioritizes direct X11 / XWayland simulation via `x11rb` (XTEST extension) and falls back
//! gracefully to `xdotool` (`libxdo`), `ydotool`, or `wtype` on Wayland without ever panicking.
use crate::{
    platform::{InputBackend, PlatformError},
    protocol::{DeckAction, KeyboardKey, PointerButton, PresentationCommand},
};
use std::{process::Command, sync::Mutex};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            ConnectionExt as XprotoExt, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT,
            KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
        },
        xtest::ConnectionExt as XtestExt,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt,
};

const NO_SYMBOL: u32 = 0;

pub struct LinuxInput;

static X11_LOCK: Mutex<()> = Mutex::new(());

/// Standard X11 keysyms for named protocol keys.
pub(crate) fn keysym_for(key: KeyboardKey) -> u32 {
    match key {
        KeyboardKey::Enter => 0xff0d,          // XK_Return
        KeyboardKey::Backspace => 0xff08,      // XK_BackSpace
        KeyboardKey::Tab => 0xff09,            // XK_Tab
        KeyboardKey::Escape => 0xff1b,         // XK_Escape
        KeyboardKey::Left => 0xff51,           // XK_Left
        KeyboardKey::Up => 0xff52,             // XK_Up
        KeyboardKey::Right => 0xff53,          // XK_Right
        KeyboardKey::Down => 0xff54,           // XK_Down
        KeyboardKey::Delete => 0xffff,         // XK_Delete
        KeyboardKey::Home => 0xff50,           // XK_Home
        KeyboardKey::End => 0xff57,            // XK_End
        KeyboardKey::Numpad0 => 0xffb0,        // XK_KP_0
        KeyboardKey::Numpad1 => 0xffb1,        // XK_KP_1
        KeyboardKey::Numpad2 => 0xffb2,        // XK_KP_2
        KeyboardKey::Numpad3 => 0xffb3,        // XK_KP_3
        KeyboardKey::Numpad4 => 0xffb4,        // XK_KP_4
        KeyboardKey::Numpad5 => 0xffb5,        // XK_KP_5
        KeyboardKey::Numpad6 => 0xffb6,        // XK_KP_6
        KeyboardKey::Numpad7 => 0xffb7,        // XK_KP_7
        KeyboardKey::Numpad8 => 0xffb8,        // XK_KP_8
        KeyboardKey::Numpad9 => 0xffb9,        // XK_KP_9
        KeyboardKey::NumpadDecimal => 0xffae,  // XK_KP_Decimal
        KeyboardKey::NumpadComma => 0xffac,    // XK_KP_Separator
        KeyboardKey::NumpadAdd => 0xffab,      // XK_KP_Add
        KeyboardKey::NumpadSubtract => 0xffad, // XK_KP_Subtract
        KeyboardKey::NumpadMultiply => 0xffaa, // XK_KP_Multiply
        KeyboardKey::NumpadDivide => 0xffaf,   // XK_KP_Divide
        KeyboardKey::NumpadEnter => 0xff8d,    // XK_KP_Enter
    }
}

/// Xdotool key names used as fallback when direct X11 XTEST is unavailable.
pub(crate) fn xdotool_key_name(key: KeyboardKey) -> &'static str {
    match key {
        KeyboardKey::Enter => "Return",
        KeyboardKey::Backspace => "BackSpace",
        KeyboardKey::Tab => "Tab",
        KeyboardKey::Escape => "Escape",
        KeyboardKey::Left => "Left",
        KeyboardKey::Up => "Up",
        KeyboardKey::Right => "Right",
        KeyboardKey::Down => "Down",
        KeyboardKey::Delete => "Delete",
        KeyboardKey::Home => "Home",
        KeyboardKey::End => "End",
        KeyboardKey::Numpad0 => "KP_0",
        KeyboardKey::Numpad1 => "KP_1",
        KeyboardKey::Numpad2 => "KP_2",
        KeyboardKey::Numpad3 => "KP_3",
        KeyboardKey::Numpad4 => "KP_4",
        KeyboardKey::Numpad5 => "KP_5",
        KeyboardKey::Numpad6 => "KP_6",
        KeyboardKey::Numpad7 => "KP_7",
        KeyboardKey::Numpad8 => "KP_8",
        KeyboardKey::Numpad9 => "KP_9",
        KeyboardKey::NumpadDecimal => "KP_Decimal",
        KeyboardKey::NumpadComma => "KP_Separator",
        KeyboardKey::NumpadAdd => "KP_Add",
        KeyboardKey::NumpadSubtract => "KP_Subtract",
        KeyboardKey::NumpadMultiply => "KP_Multiply",
        KeyboardKey::NumpadDivide => "KP_Divide",
        KeyboardKey::NumpadEnter => "KP_Enter",
    }
}

/// Converts a Unicode scalar value to an X11 keysym (Latin-1 direct or `0x0100_0000 | codepoint`).
pub(crate) fn char_to_keysym(ch: char) -> u32 {
    match ch {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        c if (0x20..=0x7e).contains(&(c as u32)) || (0xa0..=0xff).contains(&(c as u32)) => c as u32,
        c => 0x0100_0000 | (c as u32),
    }
}

/// Maps `PointerButton` to X11 button number (1 = left, 3 = right).
pub(crate) fn x11_button(button: PointerButton) -> u8 {
    match button {
        PointerButton::Left => 1,
        PointerButton::Right => 3,
    }
}

/// Maps signed scroll steps to X11 wheel button (4 = up, 5 = down) and repeat count.
pub(crate) fn x11_scroll(steps: i8) -> (u8, u8) {
    if steps >= 0 {
        (4, steps.unsigned_abs())
    } else {
        (5, steps.unsigned_abs())
    }
}

fn with_x11<F>(f: F) -> Result<(), String>
where
    F: FnOnce(&RustConnection, usize) -> Result<(), String>,
{
    let _guard = X11_LOCK
        .lock()
        .map_err(|_| "X11 input mutex poisoned".to_string())?;
    let (conn, screen_num) =
        RustConnection::connect(None).map_err(|e| format!("X11 connect failed: {e}"))?;
    f(&conn, screen_num)?;
    conn.flush().map_err(|e| format!("X11 flush failed: {e}"))?;
    Ok(())
}

/// Finds an existing keycode for `keysym` in the X11 keymap, or binds a scratch keycode.
fn resolve_keycode(conn: &RustConnection, keysym: u32) -> Result<(u8, Option<u8>), String> {
    let setup = conn.setup();
    let min_kc = setup.min_keycode;
    let max_kc = setup.max_keycode;
    let count = max_kc.saturating_sub(min_kc).saturating_add(1);
    let reply = conn
        .get_keyboard_mapping(min_kc, count)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let per_kc = usize::from(reply.keysyms_per_keycode.max(1));

    // Look for an unshifted keysym match first.
    for (idx, chunk) in reply.keysyms.chunks(per_kc).enumerate() {
        let kc = min_kc.saturating_add(idx as u8);
        if chunk.first().copied() == Some(keysym) {
            return Ok((kc, None));
        }
    }

    // Pick the last keycode in the range (or an unused one) as a temporary scratch keycode.
    let mut scratch = max_kc;
    for (idx, chunk) in reply.keysyms.chunks(per_kc).enumerate().rev() {
        if chunk.iter().all(|&sym| sym == NO_SYMBOL) {
            scratch = min_kc.saturating_add(idx as u8);
            break;
        }
    }

    conn.change_keyboard_mapping(1, scratch, 1, &[keysym])
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
    conn.sync().map_err(|e| e.to_string())?;
    Ok((scratch, Some(scratch)))
}

fn unbind_scratch(conn: &RustConnection, scratch: Option<u8>) {
    if let Some(kc) = scratch {
        if let Ok(cookie) = conn.change_keyboard_mapping(1, kc, 1, &[NO_SYMBOL]) {
            let _ = cookie.check();
        }
        let _ = conn.sync();
    }
}

fn run_cmd(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

impl InputBackend for LinuxInput {
    fn move_pointer(&self, dx: i16, dy: i16) -> Result<(), PlatformError> {
        let x11_res = with_x11(|conn, _| {
            conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 1, 0, x11rb::NONE, dx, dy, 0)
                .map_err(|e| e.to_string())?;
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        let dx_s = dx.to_string();
        let dy_s = dy.to_string();
        if run_cmd("xdotool", &["mousemove_relative", "--", &dx_s, &dy_s])
            || run_cmd("ydotool", &["mousemove", "--", &dx_s, &dy_s])
        {
            return Ok(());
        }

        tracing::warn!(
            "Linux move_pointer failed (X11: {:?}, no working Wayland/xdotool fallback)",
            x11_res.err()
        );
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn click_pointer(&self, button: PointerButton, count: u8) -> Result<(), PlatformError> {
        let btn = x11_button(button);
        let x11_res = with_x11(|conn, _| {
            for _ in 0..count {
                conn.xtest_fake_input(BUTTON_PRESS_EVENT, btn, 0, x11rb::NONE, 0, 0, 0)
                    .map_err(|e| e.to_string())?;
                conn.xtest_fake_input(BUTTON_RELEASE_EVENT, btn, 0, x11rb::NONE, 0, 0, 0)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        let btn_s = btn.to_string();
        let count_s = count.to_string();
        if run_cmd("xdotool", &["click", "--repeat", &count_s, &btn_s]) {
            return Ok(());
        }

        tracing::warn!(
            "Linux click_pointer failed (X11: {:?}, xdotool unavailable)",
            x11_res.err()
        );
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn scroll_pointer(&self, steps: i8) -> Result<(), PlatformError> {
        if steps == 0 {
            return Ok(());
        }
        let (btn, repeat) = x11_scroll(steps);
        let x11_res = with_x11(|conn, _| {
            for _ in 0..repeat {
                conn.xtest_fake_input(BUTTON_PRESS_EVENT, btn, 0, x11rb::NONE, 0, 0, 0)
                    .map_err(|e| e.to_string())?;
                conn.xtest_fake_input(BUTTON_RELEASE_EVENT, btn, 0, x11rb::NONE, 0, 0, 0)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        let btn_s = btn.to_string();
        let repeat_s = repeat.to_string();
        if run_cmd("xdotool", &["click", "--repeat", &repeat_s, &btn_s]) {
            return Ok(());
        }

        tracing::warn!(
            "Linux scroll_pointer failed (X11: {:?}, xdotool unavailable)",
            x11_res.err()
        );
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn key_down(&self, key: KeyboardKey) -> Result<(), PlatformError> {
        let sym = keysym_for(key);
        let x11_res = with_x11(|conn, _| {
            let (kc, _) = resolve_keycode(conn, sym)?;
            conn.xtest_fake_input(KEY_PRESS_EVENT, kc, 0, x11rb::NONE, 0, 0, 0)
                .map_err(|e| e.to_string())?;
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        if run_cmd("xdotool", &["keydown", xdotool_key_name(key)]) {
            return Ok(());
        }

        tracing::warn!("Linux key_down failed: {:?}", x11_res.err());
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn key_up(&self, key: KeyboardKey) -> Result<(), PlatformError> {
        let sym = keysym_for(key);
        let x11_res = with_x11(|conn, _| {
            let (kc, scratch) = resolve_keycode(conn, sym)?;
            let res = conn
                .xtest_fake_input(KEY_RELEASE_EVENT, kc, 0, x11rb::NONE, 0, 0, 0)
                .map_err(|e| e.to_string());
            unbind_scratch(conn, scratch);
            res.map(|_| ())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        if run_cmd("xdotool", &["keyup", xdotool_key_name(key)]) {
            return Ok(());
        }

        tracing::warn!("Linux key_up failed: {:?}", x11_res.err());
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn press_key(&self, key: KeyboardKey) -> Result<(), PlatformError> {
        let sym = keysym_for(key);
        let x11_res = with_x11(|conn, _| {
            let (kc, scratch) = resolve_keycode(conn, sym)?;
            let press = conn.xtest_fake_input(KEY_PRESS_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
            let release = conn.xtest_fake_input(KEY_RELEASE_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
            let _ = conn.sync();
            unbind_scratch(conn, scratch);
            press.map_err(|e| e.to_string())?;
            release.map_err(|e| e.to_string())?;
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        let name = xdotool_key_name(key);
        if run_cmd("xdotool", &["key", "--clearmodifiers", name]) || run_cmd("wtype", &["-k", name])
        {
            return Ok(());
        }

        tracing::warn!("Linux press_key failed: {:?}", x11_res.err());
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn type_unicode(&self, text: &str) -> Result<(), PlatformError> {
        if text.is_empty() {
            return Err(PlatformError::InvalidMessage);
        }

        let x11_res = with_x11(|conn, _| {
            for ch in text.chars() {
                let sym = char_to_keysym(ch);
                let (kc, scratch) = resolve_keycode(conn, sym)?;
                let press = conn.xtest_fake_input(KEY_PRESS_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
                let release = conn.xtest_fake_input(KEY_RELEASE_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
                let _ = conn.sync();
                unbind_scratch(conn, scratch);
                press.map_err(|e| e.to_string())?;
                release.map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        if x11_res.is_ok() {
            return Ok(());
        }

        if run_cmd(
            "xdotool",
            &["type", "--clearmodifiers", "--delay", "0", "--", text],
        ) || run_cmd("wtype", &["--", text])
        {
            return Ok(());
        }

        tracing::warn!("Linux type_unicode failed: {:?}", x11_res.err());
        Err(PlatformError::Unavailable("unavailable".into()))
    }

    fn presentation_control(&self, command: PresentationCommand) -> Result<(), PlatformError> {
        match command {
            PresentationCommand::Start => press_keysym_chord(&[], 0xffbe, "F5"),
            PresentationCommand::StartCurrent => press_keysym_chord(&[0xffe1], 0xffbe, "shift+F5"),
            PresentationCommand::Stop => press_keysym_chord(&[], 0xff1b, "Escape"),
            PresentationCommand::Next => press_keysym_chord(&[], 0xff56, "Next"),
            PresentationCommand::Previous => press_keysym_chord(&[], 0xff55, "Prior"),
            PresentationCommand::Blackout => press_keysym_chord(&[], 0x0062, "b"),
            PresentationCommand::Whiteout => press_keysym_chord(&[], 0x0077, "w"),
        }
    }

    fn deck_trigger(&self, action: DeckAction) -> Result<(), PlatformError> {
        match action {
            DeckAction::ShowDesktop => press_keysym_chord(&[0xffeb], 0x0064, "super+d"),
            DeckAction::TaskView => press_keysym_chord(&[], 0xffeb, "Super_L"),
            DeckAction::SwitchWindow => press_keysym_chord(&[0xffe9], 0xff09, "alt+Tab"),
            DeckAction::CloseWindow => press_keysym_chord(&[0xffe9], 0xffbd, "alt+F4"),
            DeckAction::Screenshot => {
                if press_keysym_chord(&[], 0xff61, "Print").is_ok()
                    || spawn_detached("gnome-screenshot", &["-i"])
                    || spawn_detached("spectacle", &[])
                {
                    Ok(())
                } else {
                    Err(PlatformError::Unavailable("unavailable".into()))
                }
            }
            DeckAction::Fullscreen => press_keysym_chord(&[], 0xffc8, "F11"),
            DeckAction::LockPc => crate::platform::power().lock_screen(),
            DeckAction::Calculator => {
                if spawn_detached("gnome-calculator", &[])
                    || spawn_detached("kcalc", &[])
                    || spawn_detached("galculator", &[])
                    || press_keysym_chord(&[], 0x1008_ff1d, "XF86Calculator").is_ok()
                {
                    Ok(())
                } else {
                    Err(PlatformError::Unavailable("unavailable".into()))
                }
            }
            DeckAction::Downloads => {
                let sys = crate::platform::system();
                let base = sys
                    .downloads_dir()
                    .ok_or_else(|| PlatformError::Unavailable("unavailable".into()))?;
                let folder = base.join(crate::files::FOLDER_NAME);
                let target = if folder.exists() { folder } else { base };
                sys.open_folder(&target)
                    .map_err(|()| PlatformError::Unavailable("unavailable".into()))
            }
            DeckAction::Browser => {
                if press_keysym_chord(&[], 0x1008_ff18, "XF86HomePage").is_ok() {
                    Ok(())
                } else {
                    crate::platform::system()
                        .open_url("https://www.google.com")
                        .map_err(|()| PlatformError::Unavailable("unavailable".into()))
                }
            }
            DeckAction::TaskManager => {
                if spawn_detached("gnome-system-monitor", &[])
                    || spawn_detached("plasma-systemmonitor", &[])
                    || spawn_detached("ksysguard", &[])
                    || press_keysym_chord(&[0xffe3], 0xff1b, "ctrl+Escape").is_ok()
                {
                    Ok(())
                } else {
                    Err(PlatformError::Unavailable("unavailable".into()))
                }
            }
            DeckAction::EmojiPicker => press_keysym_chord(&[0xffe3], 0x002e, "ctrl+period"),
        }
    }
}

fn spawn_detached(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).spawn().is_ok()
}

fn press_keysym_chord(
    modifiers: &[u32],
    main_sym: u32,
    xdotool_chord: &str,
) -> Result<(), PlatformError> {
    let x11_res = with_x11(|conn, _| {
        let mut mod_kcs = Vec::with_capacity(modifiers.len());
        for &mod_sym in modifiers {
            let (kc, _) = resolve_keycode(conn, mod_sym)?;
            mod_kcs.push(kc);
        }
        let (main_kc, scratch) = resolve_keycode(conn, main_sym)?;
        for &kc in &mod_kcs {
            let _ = conn.xtest_fake_input(KEY_PRESS_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
        }
        let press = conn.xtest_fake_input(KEY_PRESS_EVENT, main_kc, 0, x11rb::NONE, 0, 0, 0);
        let release = conn.xtest_fake_input(KEY_RELEASE_EVENT, main_kc, 0, x11rb::NONE, 0, 0, 0);
        for &kc in mod_kcs.iter().rev() {
            let _ = conn.xtest_fake_input(KEY_RELEASE_EVENT, kc, 0, x11rb::NONE, 0, 0, 0);
        }
        let _ = conn.sync();
        unbind_scratch(conn, scratch);
        press.map_err(|e| e.to_string())?;
        release.map_err(|e| e.to_string())?;
        Ok(())
    });
    if x11_res.is_ok() {
        return Ok(());
    }

    if run_cmd("xdotool", &["key", "--clearmodifiers", xdotool_chord]) {
        return Ok(());
    }

    tracing::warn!(
        "Linux press_keysym_chord({xdotool_chord}) failed: {:?}",
        x11_res.err()
    );
    Err(PlatformError::Unavailable("unavailable".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_keysyms_and_unicode_mapping_are_consistent() {
        assert_eq!(keysym_for(KeyboardKey::Enter), 0xff0d);
        assert_eq!(keysym_for(KeyboardKey::Backspace), 0xff08);
        assert_eq!(keysym_for(KeyboardKey::Escape), 0xff1b);
        assert_eq!(keysym_for(KeyboardKey::Numpad0), 0xffb0);
        assert_eq!(keysym_for(KeyboardKey::Numpad9), 0xffb9);
        assert_eq!(keysym_for(KeyboardKey::NumpadEnter), 0xff8d);
        assert_eq!(xdotool_key_name(KeyboardKey::NumpadComma), "KP_Separator");
        assert_eq!(char_to_keysym('a'), b'a' as u32);
        assert_eq!(char_to_keysym('á'), 0x00e1);
        assert_eq!(char_to_keysym('🦊'), 0x0100_0000 | ('🦊' as u32));
    }

    #[test]
    fn x11_mouse_buttons_and_signed_scroll_mapping() {
        assert_eq!(x11_button(PointerButton::Left), 1);
        assert_eq!(x11_button(PointerButton::Right), 3);
        assert_eq!(x11_scroll(3), (4, 3));
        assert_eq!(x11_scroll(-2), (5, 2));
    }
}
