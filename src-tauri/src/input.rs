//! Predefined, short Win32 input events. Never accepts virtual-key numbers or arbitrary flags.
use crate::protocol::{Action, KeyboardKey, PointerButton};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY,
    VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB,
    VK_UP,
};

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

fn events(action: &Action) -> Vec<INPUT> {
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
            let (vk, extended) = match named {
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
            };
            let flags = if extended {
                KEYEVENTF_EXTENDEDKEY
            } else {
                Default::default()
            };
            vec![key(vk, 0, flags), key(vk, 0, flags | KEYEVENTF_KEYUP)]
        }
        _ => Vec::new(),
    }
}

pub fn execute(action: &Action) -> Result<(), &'static str> {
    let v = events(action);
    if v.is_empty() {
        return Err("invalid_message");
    }
    // SendInput is intentionally called from the user's normal session. UIPI and
    // secure desktop can reject it; no elevation or accessibility bypass is used.
    let sent = unsafe { SendInput(&v, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != v.len() {
        // A partial batch could contain a down event. Release all supported keys
        // and mouse buttons so a failure cannot leave a held input behind.
        let mut releases = vec![
            mouse(0, 0, 0, MOUSEEVENTF_LEFTUP),
            mouse(0, 0, 0, MOUSEEVENTF_RIGHTUP),
        ];
        if let Action::KeyboardKey { key: named } = action {
            let pair = events(&Action::KeyboardKey { key: *named });
            releases.push(pair[1]);
        } else if let Action::KeyboardText { text } = action {
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
        return Err("unavailable");
    }
    Ok(())
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
        execute(&Action::PointerMove { dx: 16, dy: 0 }).unwrap();
        let mut after = POINT::default();
        unsafe {
            GetCursorPos(&mut after).unwrap();
        }
        assert_ne!(after.x, 100);
    }
}
