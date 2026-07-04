use std::str::FromStr;

use gayland::HotkeySpec;

use super::signals::CaptureControl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Stop,
    Pause,
    Resume,
    TogglePause,
    SaveReplayBuffer,
}

impl FromStr for HotkeyAction {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "stop" => Ok(Self::Stop),
            "pause" => Ok(Self::Pause),
            "resume" => Ok(Self::Resume),
            "toggle-pause" | "toggle_pause" | "togglepause" => Ok(Self::TogglePause),
            "save-replay-buffer" | "save_replay_buffer" | "savereplaybuffer" => {
                Ok(Self::SaveReplayBuffer)
            }
            other => Err(format!("unknown hotkey action '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    pub action: HotkeyAction,
    pub spec: HotkeySpec,
}

impl FromStr for HotkeyBinding {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (action, hotkey) = input
            .split_once('=')
            .ok_or_else(|| "hotkey must be formatted as action=chord".to_string())?;
        Ok(Self {
            action: action.parse()?,
            spec: hotkey
                .parse()
                .map_err(|e| format!("invalid hotkey '{hotkey}': {e}"))?,
        })
    }
}

impl std::fmt::Display for HotkeyAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotkeyAction::Stop => write!(f, "stop"),
            HotkeyAction::Pause => write!(f, "pause"),
            HotkeyAction::Resume => write!(f, "resume"),
            HotkeyAction::TogglePause => write!(f, "toggle-pause"),
            HotkeyAction::SaveReplayBuffer => write!(f, "save-replay-buffer"),
        }
    }
}

pub fn apply_action(action: HotkeyAction, control: &CaptureControl) {
    match action {
        HotkeyAction::Stop => control.request_stop(),
        HotkeyAction::Pause => control.request_pause(),
        HotkeyAction::Resume => control.request_resume(),
        HotkeyAction::TogglePause => {
            if control.paused.load(std::sync::atomic::Ordering::Relaxed) {
                control.request_resume();
            } else {
                control.request_pause();
            }
        }
        HotkeyAction::SaveReplayBuffer => {
            control.request_save_replay_buffer();
        }
    }
}
