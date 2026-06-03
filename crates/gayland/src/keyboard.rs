use std::collections::{BTreeSet, HashSet};

use crate::HotkeySpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub keycode: u32,
    pub state: KeyState,
    pub time_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub keycode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInfo {
    pub keycode: u32,
    pub name: Option<&'static str>,
    pub is_modifier: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyParseError {
    Empty,
    UnknownKey(String),
}

impl std::fmt::Display for HotkeyParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "hotkey is empty"),
            Self::UnknownKey(key) => write!(f, "unknown key '{key}'"),
        }
    }
}

impl std::error::Error for HotkeyParseError {}

#[derive(Default)]
pub struct KeyboardState {
    pressed: HashSet<u32>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: KeyEvent) {
        match event.state {
            KeyState::Pressed => {
                self.pressed.insert(event.keycode);
            }
            KeyState::Released => {
                self.pressed.remove(&event.keycode);
            }
        }
    }

    pub fn is_pressed(&self, keycode: u32) -> bool {
        self.pressed.contains(&keycode)
    }

    pub fn pressed_count(&self) -> usize {
        self.pressed.len()
    }
}

pub trait KeyboardEventSink: Send + Sync {
    fn on_key_event(&self, _event: KeyEvent) {}
}

pub fn key_info(keycode: u32) -> KeyInfo {
    KeyInfo {
        keycode,
        name: key_name(keycode),
        is_modifier: is_modifier_key(keycode),
    }
}

pub fn parse_hotkey(input: &str) -> Result<HotkeySpec, HotkeyParseError> {
    let mut keys = BTreeSet::new();
    for raw in input.split('+') {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let keycode = keycode_from_name(name)
            .ok_or_else(|| HotkeyParseError::UnknownKey(name.to_string()))?;
        keys.insert(keycode);
    }
    if keys.is_empty() {
        return Err(HotkeyParseError::Empty);
    }
    Ok(HotkeySpec { keys })
}

pub fn keycode_from_name(name: &str) -> Option<u32> {
    let key = normalize_key_name(name);
    match key.as_str() {
        "ctrl" | "control" | "leftctrl" | "leftcontrol" | "lctrl" => Some(KEY_LEFTCTRL),
        "rightctrl" | "rightcontrol" | "rctrl" => Some(KEY_RIGHTCTRL),
        "shift" | "leftshift" | "lshift" => Some(KEY_LEFTSHIFT),
        "rightshift" | "rshift" => Some(KEY_RIGHTSHIFT),
        "alt" | "leftalt" | "lalt" => Some(KEY_LEFTALT),
        "rightalt" | "ralt" | "altgr" => Some(KEY_RIGHTALT),
        "meta" | "super" | "win" | "logo" | "leftmeta" | "leftsuper" | "leftwin" => {
            Some(KEY_LEFTMETA)
        }
        "rightmeta" | "rightsuper" | "rightwin" => Some(KEY_RIGHTMETA),
        "capslock" | "caps" => Some(KEY_CAPSLOCK),
        "tab" => Some(KEY_TAB),
        "enter" | "return" => Some(KEY_ENTER),
        "esc" | "escape" => Some(KEY_ESC),
        "space" => Some(KEY_SPACE),
        "backspace" => Some(KEY_BACKSPACE),
        "delete" | "del" => Some(KEY_DELETE),
        "insert" | "ins" => Some(KEY_INSERT),
        "home" => Some(KEY_HOME),
        "end" => Some(KEY_END),
        "pageup" | "pgup" => Some(KEY_PAGEUP),
        "pagedown" | "pgdn" => Some(KEY_PAGEDOWN),
        "up" | "arrowup" => Some(KEY_UP),
        "down" | "arrowdown" => Some(KEY_DOWN),
        "left" | "arrowleft" => Some(KEY_LEFT),
        "right" | "arrowright" => Some(KEY_RIGHT),
        "minus" | "-" => Some(KEY_MINUS),
        "equal" | "equals" | "=" => Some(KEY_EQUAL),
        "leftbrace" | "leftbracket" | "[" => Some(KEY_LEFTBRACE),
        "rightbrace" | "rightbracket" | "]" => Some(KEY_RIGHTBRACE),
        "semicolon" | ";" => Some(KEY_SEMICOLON),
        "apostrophe" | "quote" | "'" => Some(KEY_APOSTROPHE),
        "grave" | "backtick" | "`" => Some(KEY_GRAVE),
        "backslash" | "\\" => Some(KEY_BACKSLASH),
        "comma" | "," => Some(KEY_COMMA),
        "dot" | "period" | "." => Some(KEY_DOT),
        "slash" | "/" => Some(KEY_SLASH),
        _ => keycode_from_simple_name(&key),
    }
}

pub fn key_name(keycode: u32) -> Option<&'static str> {
    Some(match keycode {
        KEY_ESC => "Esc",
        KEY_1 => "1",
        KEY_2 => "2",
        KEY_3 => "3",
        KEY_4 => "4",
        KEY_5 => "5",
        KEY_6 => "6",
        KEY_7 => "7",
        KEY_8 => "8",
        KEY_9 => "9",
        KEY_0 => "0",
        KEY_MINUS => "Minus",
        KEY_EQUAL => "Equal",
        KEY_BACKSPACE => "Backspace",
        KEY_TAB => "Tab",
        KEY_Q => "Q",
        KEY_W => "W",
        KEY_E => "E",
        KEY_R => "R",
        KEY_T => "T",
        KEY_Y => "Y",
        KEY_U => "U",
        KEY_I => "I",
        KEY_O => "O",
        KEY_P => "P",
        KEY_LEFTBRACE => "LeftBracket",
        KEY_RIGHTBRACE => "RightBracket",
        KEY_ENTER => "Enter",
        KEY_LEFTCTRL => "LeftCtrl",
        KEY_A => "A",
        KEY_S => "S",
        KEY_D => "D",
        KEY_F => "F",
        KEY_G => "G",
        KEY_H => "H",
        KEY_J => "J",
        KEY_K => "K",
        KEY_L => "L",
        KEY_SEMICOLON => "Semicolon",
        KEY_APOSTROPHE => "Apostrophe",
        KEY_GRAVE => "Grave",
        KEY_LEFTSHIFT => "LeftShift",
        KEY_BACKSLASH => "Backslash",
        KEY_Z => "Z",
        KEY_X => "X",
        KEY_C => "C",
        KEY_V => "V",
        KEY_B => "B",
        KEY_N => "N",
        KEY_M => "M",
        KEY_COMMA => "Comma",
        KEY_DOT => "Dot",
        KEY_SLASH => "Slash",
        KEY_RIGHTSHIFT => "RightShift",
        KEY_LEFTALT => "LeftAlt",
        KEY_SPACE => "Space",
        KEY_CAPSLOCK => "CapsLock",
        KEY_F1 => "F1",
        KEY_F2 => "F2",
        KEY_F3 => "F3",
        KEY_F4 => "F4",
        KEY_F5 => "F5",
        KEY_F6 => "F6",
        KEY_F7 => "F7",
        KEY_F8 => "F8",
        KEY_F9 => "F9",
        KEY_F10 => "F10",
        KEY_F11 => "F11",
        KEY_F12 => "F12",
        KEY_RIGHTCTRL => "RightCtrl",
        KEY_RIGHTALT => "RightAlt",
        KEY_HOME => "Home",
        KEY_UP => "Up",
        KEY_PAGEUP => "PageUp",
        KEY_LEFT => "Left",
        KEY_RIGHT => "Right",
        KEY_END => "End",
        KEY_DOWN => "Down",
        KEY_PAGEDOWN => "PageDown",
        KEY_INSERT => "Insert",
        KEY_DELETE => "Delete",
        KEY_LEFTMETA => "LeftMeta",
        KEY_RIGHTMETA => "RightMeta",
        _ => return None,
    })
}

pub fn is_modifier_key(keycode: u32) -> bool {
    matches!(
        keycode,
        KEY_LEFTCTRL
            | KEY_RIGHTCTRL
            | KEY_LEFTSHIFT
            | KEY_RIGHTSHIFT
            | KEY_LEFTALT
            | KEY_RIGHTALT
            | KEY_LEFTMETA
            | KEY_RIGHTMETA
            | KEY_CAPSLOCK
    )
}

fn keycode_from_simple_name(key: &str) -> Option<u32> {
    if key.len() == 1 {
        return match key.as_bytes()[0] {
            b'a' => Some(KEY_A),
            b'b' => Some(KEY_B),
            b'c' => Some(KEY_C),
            b'd' => Some(KEY_D),
            b'e' => Some(KEY_E),
            b'f' => Some(KEY_F),
            b'g' => Some(KEY_G),
            b'h' => Some(KEY_H),
            b'i' => Some(KEY_I),
            b'j' => Some(KEY_J),
            b'k' => Some(KEY_K),
            b'l' => Some(KEY_L),
            b'm' => Some(KEY_M),
            b'n' => Some(KEY_N),
            b'o' => Some(KEY_O),
            b'p' => Some(KEY_P),
            b'q' => Some(KEY_Q),
            b'r' => Some(KEY_R),
            b's' => Some(KEY_S),
            b't' => Some(KEY_T),
            b'u' => Some(KEY_U),
            b'v' => Some(KEY_V),
            b'w' => Some(KEY_W),
            b'x' => Some(KEY_X),
            b'y' => Some(KEY_Y),
            b'z' => Some(KEY_Z),
            b'0' => Some(KEY_0),
            b'1' => Some(KEY_1),
            b'2' => Some(KEY_2),
            b'3' => Some(KEY_3),
            b'4' => Some(KEY_4),
            b'5' => Some(KEY_5),
            b'6' => Some(KEY_6),
            b'7' => Some(KEY_7),
            b'8' => Some(KEY_8),
            b'9' => Some(KEY_9),
            _ => None,
        };
    }

    if let Some(num) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return match num {
            1 => Some(KEY_F1),
            2 => Some(KEY_F2),
            3 => Some(KEY_F3),
            4 => Some(KEY_F4),
            5 => Some(KEY_F5),
            6 => Some(KEY_F6),
            7 => Some(KEY_F7),
            8 => Some(KEY_F8),
            9 => Some(KEY_F9),
            10 => Some(KEY_F10),
            11 => Some(KEY_F11),
            12 => Some(KEY_F12),
            _ => None,
        };
    }

    None
}

fn normalize_key_name(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .collect()
}

pub const KEY_ESC: u32 = 1;
pub const KEY_1: u32 = 2;
pub const KEY_2: u32 = 3;
pub const KEY_3: u32 = 4;
pub const KEY_4: u32 = 5;
pub const KEY_5: u32 = 6;
pub const KEY_6: u32 = 7;
pub const KEY_7: u32 = 8;
pub const KEY_8: u32 = 9;
pub const KEY_9: u32 = 10;
pub const KEY_0: u32 = 11;
pub const KEY_MINUS: u32 = 12;
pub const KEY_EQUAL: u32 = 13;
pub const KEY_BACKSPACE: u32 = 14;
pub const KEY_TAB: u32 = 15;
pub const KEY_Q: u32 = 16;
pub const KEY_W: u32 = 17;
pub const KEY_E: u32 = 18;
pub const KEY_R: u32 = 19;
pub const KEY_T: u32 = 20;
pub const KEY_Y: u32 = 21;
pub const KEY_U: u32 = 22;
pub const KEY_I: u32 = 23;
pub const KEY_O: u32 = 24;
pub const KEY_P: u32 = 25;
pub const KEY_LEFTBRACE: u32 = 26;
pub const KEY_RIGHTBRACE: u32 = 27;
pub const KEY_ENTER: u32 = 28;
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_A: u32 = 30;
pub const KEY_S: u32 = 31;
pub const KEY_D: u32 = 32;
pub const KEY_F: u32 = 33;
pub const KEY_G: u32 = 34;
pub const KEY_H: u32 = 35;
pub const KEY_J: u32 = 36;
pub const KEY_K: u32 = 37;
pub const KEY_L: u32 = 38;
pub const KEY_SEMICOLON: u32 = 39;
pub const KEY_APOSTROPHE: u32 = 40;
pub const KEY_GRAVE: u32 = 41;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_BACKSLASH: u32 = 43;
pub const KEY_Z: u32 = 44;
pub const KEY_X: u32 = 45;
pub const KEY_C: u32 = 46;
pub const KEY_V: u32 = 47;
pub const KEY_B: u32 = 48;
pub const KEY_N: u32 = 49;
pub const KEY_M: u32 = 50;
pub const KEY_COMMA: u32 = 51;
pub const KEY_DOT: u32 = 52;
pub const KEY_SLASH: u32 = 53;
pub const KEY_RIGHTSHIFT: u32 = 54;
pub const KEY_LEFTALT: u32 = 56;
pub const KEY_SPACE: u32 = 57;
pub const KEY_CAPSLOCK: u32 = 58;
pub const KEY_F1: u32 = 59;
pub const KEY_F2: u32 = 60;
pub const KEY_F3: u32 = 61;
pub const KEY_F4: u32 = 62;
pub const KEY_F5: u32 = 63;
pub const KEY_F6: u32 = 64;
pub const KEY_F7: u32 = 65;
pub const KEY_F8: u32 = 66;
pub const KEY_F9: u32 = 67;
pub const KEY_F10: u32 = 68;
pub const KEY_F11: u32 = 87;
pub const KEY_F12: u32 = 88;
pub const KEY_RIGHTCTRL: u32 = 97;
pub const KEY_RIGHTALT: u32 = 100;
pub const KEY_HOME: u32 = 102;
pub const KEY_UP: u32 = 103;
pub const KEY_PAGEUP: u32 = 104;
pub const KEY_LEFT: u32 = 105;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_END: u32 = 107;
pub const KEY_DOWN: u32 = 108;
pub const KEY_PAGEDOWN: u32 = 109;
pub const KEY_INSERT: u32 = 110;
pub const KEY_DELETE: u32 = 111;
pub const KEY_LEFTMETA: u32 = 125;
pub const KEY_RIGHTMETA: u32 = 126;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_hotkey_names() {
        let spec = parse_hotkey("Ctrl+Shift+R").unwrap();
        assert_eq!(
            spec.keys,
            BTreeSet::from([KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_R])
        );
    }

    #[test]
    fn parses_right_side_modifiers() {
        let spec = parse_hotkey("RightCtrl+AltGr+F12").unwrap();
        assert_eq!(
            spec.keys,
            BTreeSet::from([KEY_RIGHTCTRL, KEY_RIGHTALT, KEY_F12])
        );
    }
}
