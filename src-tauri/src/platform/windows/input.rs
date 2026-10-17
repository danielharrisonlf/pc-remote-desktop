//! Windows Win32 `SendInput` implementation of `InputBackend`.
use crate::{
    platform::{InputBackend, PlatformError},
    protocol::{Action, DeckAction, KeyboardKey, PointerButton, PresentationCommand},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY,
    VK_ADD, VK_BACK, VK_BROWSER_HOME, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_F11, VK_F4, VK_F5, VK_HOME, VK_LAUNCH_APP2, VK_LEFT, VK_LWIN, VK_MENU,
    VK_MULTIPLY, VK_NEXT, VK_NUMPAD0, VK_NUMPAD1, VK_NUMPAD2, VK_NUMPAD3, VK_NUMPAD4, VK_NUMPAD5,
    VK_NUMPAD6, VK_NUMPAD7, VK_NUMPAD8, VK_NUMPAD9, VK_OEM_PERIOD, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SEPARATOR, VK_SHIFT, VK_SUBTRACT, VK_TAB, VK_UP,
};

pub struct WindowsInput;

fn mouse(
    dx: i32,
    dy: i32,
    data: u32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn key(
    vk: VIRTUAL_KEY,
    scan: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn vk_for(named: KeyboardKey) -> (VIRTUAL_KEY, bool) {
    match named {
        KeyboardKey::Enter => (VK_RETURN, false),
        KeyboardKey::Backspace => (VK_BACK, false),
        KeyboardKey::Tab => (VK_TAB, false),
        KeyboardKey::Escape => (VK_ESCAPE, false),
        KeyboardKey::Left => (VK_LEFT, true),
        KeyboardKey::Right => (VK_RIGHT, true),
        KeyboardKey::Up => (VK_UP, true),
        KeyboardKey::Down => (VK_DOWN, true),
        KeyboardKey::Delete => (VK_DELETE, true),
        KeyboardKey::Home => (VK_HOME, true),
        KeyboardKey::End => (VK_END, true),
        KeyboardKey::Numpad0 => (VK_NUMPAD0, false),
        KeyboardKey::Numpad1 => (VK_NUMPAD1, false),
        KeyboardKey::Numpad2 => (VK_NUMPAD2, false),
        KeyboardKey::Numpad3 => (VK_NUMPAD3, false),
        KeyboardKey::Numpad4 => (VK_NUMPAD4, false),
        KeyboardKey::Numpad5 => (VK_NUMPAD5, false),
        KeyboardKey::Numpad6 => (VK_NUMPAD6, false),
        KeyboardKey::Numpad7 => (VK_NUMPAD7, false),
        KeyboardKey::Numpad8 => (VK_NUMPAD8, false),
        KeyboardKey::Numpad9 => (VK_NUMPAD9, false),
        KeyboardKey::NumpadDecimal => (VK_DECIMAL, false),
        KeyboardKey::NumpadComma => (VK_SEPARATOR, false),
        KeyboardKey::NumpadAdd => (VK_ADD, false),
        KeyboardKey::NumpadSubtract => (VK_SUBTRACT, false),
        KeyboardKey::NumpadMultiply => (VK_MULTIPLY, false),
        KeyboardKey::NumpadDivide => (VK_DIVIDE, true),
        KeyboardKey::NumpadEnter => (VK_RETURN, true),
    }
}

fn tap(vk: VIRTUAL_KEY, extended: bool) -> Vec<INPUT> {
    let flags = if extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        Default::default()
    };
    vec![key(vk, 0, flags), key(vk, 0, flags | KEYEVENTF_KEYUP)]
}

fn chord(modifiers: &[(VIRTUAL_KEY, bool)], main_vk: VIRTUAL_KEY, main_ext: bool) -> Vec<INPUT> {
    let mut v = Vec::with_capacity((modifiers.len() + 1) * 2);
    for &(vk, ext) in modifiers {
        let flags = if ext {
            KEYEVENTF_EXTENDEDKEY
        } else {
            Default::default()
        };
        v.push(key(vk, 0, flags));
    }
    let main_flags = if main_ext {
        KEYEVENTF_EXTENDEDKEY
    } else {
        Default::default()
    };
    v.push(key(main_vk, 0, main_flags));
    v.push(key(main_vk, 0, main_flags | KEYEVENTF_KEYUP));
    for &(vk, ext) in modifiers.iter().rev() {
        let flags = if ext {
            KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        } else {
            KEYEVENTF_KEYUP
        };
        v.push(key(vk, 0, flags));
    }
    v
}

pub(crate) fn presentation_events(command: PresentationCommand) -> Vec<INPUT> {
    match command {
        PresentationCommand::Start => tap(VK_F5, false),
        PresentationCommand::StartCurrent => chord(&[(VK_SHIFT, false)], VK_F5, false),
        PresentationCommand::Stop => tap(VK_ESCAPE, false),
        PresentationCommand::Next => tap(VK_NEXT, true),
        PresentationCommand::Previous => tap(VK_PRIOR, true),
        PresentationCommand::Blackout => tap(VIRTUAL_KEY(0x42), false), // 'B'
        PresentationCommand::Whiteout => tap(VIRTUAL_KEY(0x57), false), // 'W'
    }
}

pub(crate) fn deck_events(action: DeckAction) -> Vec<INPUT> {
    match action {
        DeckAction::ShowDesktop => chord(&[(VK_LWIN, true)], VIRTUAL_KEY(0x44), false), // Win+D
        DeckAction::TaskView => chord(&[(VK_LWIN, true)], VK_TAB, false),               // Win+Tab
        DeckAction::SwitchWindow => chord(&[(VK_MENU, false)], VK_TAB, false),          // Alt+Tab
        DeckAction::CloseWindow => chord(&[(VK_MENU, false)], VK_F4, false),            // Alt+F4
        DeckAction::Screenshot => {
            chord(&[(VK_LWIN, true), (VK_SHIFT, false)], VIRTUAL_KEY(0x53), false) // Win+Shift+S
        }
        DeckAction::Fullscreen => tap(VK_F11, false),
        DeckAction::Calculator => tap(VK_LAUNCH_APP2, true),
        DeckAction::Browser => tap(VK_BROWSER_HOME, true),
        DeckAction::TaskManager => {
            chord(&[(VK_CONTROL, false), (VK_SHIFT, false)], VK_ESCAPE, false)
        }
        DeckAction::EmojiPicker => chord(&[(VK_LWIN, true)], VK_OEM_PERIOD, false),
        DeckAction::LockPc | DeckAction::Downloads => Vec::new(),
    }
}

pub(crate) fn events(action: &Action) -> Vec<INPUT> {
    match action {
        Action::PointerMove { dx, dy } => {
            vec![mouse(i32::from(*dx), i32::from(*dy), 0, MOUSEEVENTF_MOVE)]
        }
        Action::PointerClick { button, count } => {
            let (down, up) = match button {
                PointerButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                PointerButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            };
            let mut v = Vec::with_capacity(usize::from(*count) * 2);
            for _ in 0..*count {
                v.push(mouse(0, 0, 0, down));
                v.push(mouse(0, 0, 0, up));
            }
            v
        }
        Action::PointerScroll { steps } => vec![mouse(
            0,
            0,
            (i32::from(*steps) * 120) as u32,
            MOUSEEVENTF_WHEEL,
        )],
        Action::KeyboardText { text } => {
            let mut v = Vec::with_capacity(text.len() * 2);
            for unit in text.encode_utf16() {
                v.push(key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
                v.push(key(
                    VIRTUAL_KEY(0),
                    unit,
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                ));
            }
            v
        }
        Action::KeyboardKey { key: named } => {
            let (vk, extended) = vk_for(*named);
            tap(vk, extended)
        }
        Action::PresentationControl { command } => presentation_events(*command),
        Action::DeckTrigger { id } => deck_events(*id),
        _ => Vec::new(),
    }
}

fn send_batch(v: &[INPUT], action: Option<&Action>) -> Result<(), PlatformError> {
    if v.is_empty() {
        return Err(PlatformError::InvalidMessage);
    }
    // SendInput is intentionally called from the user's normal session. UIPI and
    // secure desktop can reject it; no elevation or accessibility bypass is used.
    let sent = unsafe { SendInput(v, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != v.len() {
        // A partial batch could contain a down event. Release all supported keys
        // and mouse buttons so a failure cannot leave a held input behind.
        let mut releases = vec![
            mouse(0, 0, 0, MOUSEEVENTF_LEFTUP),
            mouse(0, 0, 0, MOUSEEVENTF_RIGHTUP),
            key(VK_SHIFT, 0, KEYEVENTF_KEYUP),
            key(VK_CONTROL, 0, KEYEVENTF_KEYUP),
            key(VK_MENU, 0, KEYEVENTF_KEYUP),
            key(VK_LWIN, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
        ];
        if let Some(Action::KeyboardKey { key: named }) = action {
            let pair = events(&Action::KeyboardKey { key: *named });
            releases.push(pair[1]);
        } else if let Some(Action::KeyboardText { text }) = action {
            for unit in text.encode_utf16() {
                releases.push(key(
                    VIRTUAL_KEY(0),
                    unit,
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                ));
            }
        }
        unsafe {
            SendInput(&releases, std::mem::size_of::<INPUT>() as i32);
        }
        return Err(PlatformError::Unavailable("unavailable".into()));
    }
    Ok(())
}

fn open_downloads_folder() -> Result<(), PlatformError> {
    let base = crate::platform::system()
        .downloads_dir()
        .ok_or_else(|| PlatformError::Unavailable("unavailable".into()))?;
    let folder = base.join(crate::files::FOLDER_NAME);
    let _ = std::fs::create_dir_all(&folder);
    crate::platform::system()
        .open_folder(&folder)
        .map_err(|()| PlatformError::Unavailable("unavailable".into()))
}

impl InputBackend for WindowsInput {
    fn move_pointer(&self, dx: i16, dy: i16) -> Result<(), PlatformError> {
        let action = Action::PointerMove { dx, dy };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn click_pointer(&self, button: PointerButton, count: u8) -> Result<(), PlatformError> {
        let action = Action::PointerClick { button, count };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn scroll_pointer(&self, steps: i8) -> Result<(), PlatformError> {
        let action = Action::PointerScroll { steps };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn key_down(&self, named: KeyboardKey) -> Result<(), PlatformError> {
        let (vk, extended) = vk_for(named);
        let flags = if extended {
            KEYEVENTF_EXTENDEDKEY
        } else {
            Default::default()
        };
        send_batch(&[key(vk, 0, flags)], None)
    }

    fn key_up(&self, named: KeyboardKey) -> Result<(), PlatformError> {
        let (vk, extended) = vk_for(named);
        let flags = if extended {
            KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        } else {
            KEYEVENTF_KEYUP
        };
        send_batch(&[key(vk, 0, flags)], None)
    }

    fn press_key(&self, named: KeyboardKey) -> Result<(), PlatformError> {
        let action = Action::KeyboardKey { key: named };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn type_unicode(&self, text: &str) -> Result<(), PlatformError> {
        let action = Action::KeyboardText {
            text: text.to_string(),
        };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn presentation_control(&self, command: PresentationCommand) -> Result<(), PlatformError> {
        let action = Action::PresentationControl { command };
        let v = events(&action);
        send_batch(&v, Some(&action))
    }

    fn deck_trigger(&self, action: DeckAction) -> Result<(), PlatformError> {
        match action {
            DeckAction::LockPc => crate::platform::power().lock_screen(),
            DeckAction::Downloads => open_downloads_folder(),
            other => {
                let act = Action::DeckTrigger { id: other };
                let v = events(&act);
                send_batch(&v, Some(&act))
            }
        }
    }

    fn execute(&self, action: &Action) -> Result<(), PlatformError> {
        match action {
            Action::DeckTrigger { id } => self.deck_trigger(*id),
            Action::PresentationControl { command } => self.presentation_control(*command),
            _ => {
                let v = events(action);
                send_batch(&v, Some(action))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_surrogates_and_release_are_bounded() {
        let v = events(&Action::KeyboardText {
            text: "á🦊".into()
        });
        assert_eq!(v.len(), 6);
        for pair in v.chunks_exact(2) {
            unsafe {
                assert_eq!(pair[0].Anonymous.ki.wScan, pair[1].Anonymous.ki.wScan);
                assert_eq!(
                    pair[1].Anonymous.ki.dwFlags,
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                );
            }
        }
    }

    #[test]
    fn click_is_down_up_and_scroll_is_signed() {
        let v = events(&Action::PointerClick {
            button: PointerButton::Left,
            count: 2,
        });
        assert_eq!(v.len(), 4);
        unsafe {
            assert_eq!(v[0].Anonymous.mi.dwFlags, MOUSEEVENTF_LEFTDOWN);
            assert_eq!(v[3].Anonymous.mi.dwFlags, MOUSEEVENTF_LEFTUP);
        }
        let v = events(&Action::PointerScroll { steps: -1 });
        unsafe {
            assert_eq!(v[0].Anonymous.mi.mouseData as i32, -120);
        }
    }

    #[test]
    fn numpad_presentation_and_deck_chords_balance_down_and_up() {
        let num = events(&Action::KeyboardKey {
            key: KeyboardKey::NumpadEnter,
        });
        assert_eq!(num.len(), 2);
        unsafe {
            assert_eq!(num[0].Anonymous.ki.wVk, VK_RETURN);
            assert_eq!(num[0].Anonymous.ki.dwFlags, KEYEVENTF_EXTENDEDKEY);
            assert_eq!(
                num[1].Anonymous.ki.dwFlags,
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
            );
        }

        let start_current = events(&Action::PresentationControl {
            command: PresentationCommand::StartCurrent,
        });
        assert_eq!(start_current.len(), 4);

        let snap = events(&Action::DeckTrigger {
            id: DeckAction::Screenshot,
        });
        assert_eq!(snap.len(), 6);
    }

    #[test]
    #[ignore = "move temporariamente o cursor real do Windows"]
    fn windows_pointer_smoke_restores_cursor() {
        use windows::Win32::{
            Foundation::POINT,
            UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos},
        };
        let mut before = POINT::default();
        unsafe {
            GetCursorPos(&mut before).unwrap();
        }
        struct Restore(POINT);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    let _ = SetCursorPos(self.0.x, self.0.y);
                }
            }
        }
        let _restore = Restore(before);
        unsafe {
            SetCursorPos(100, 100).unwrap();
        }
        WindowsInput
            .execute(&Action::PointerMove { dx: 16, dy: 0 })
            .unwrap();
        let mut after = POINT::default();
        unsafe {
            GetCursorPos(&mut after).unwrap();
        }
        assert_ne!(after.x, 100);
    }
}
