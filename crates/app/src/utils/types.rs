#[derive(Debug)]
pub enum ProcessState {
    Running,
    Stopped(Option<String>),
    Preview,
    Paused,
}

impl std::fmt::Display for ProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessState::Running => write!(f, "running"),
            ProcessState::Stopped(_) => write!(f, "stopped"),
            ProcessState::Preview => write!(f, "preview"),
            ProcessState::Paused => write!(f, "paused"),
        }
    }
}

impl ProcessState {
    pub fn to_body(&self) -> String {
        match self {
            ProcessState::Running => "Screen recording is about to start".into(),
            ProcessState::Stopped(path) => match path {
                Some(path) => format!("Screen recording has stopped, file saved to {}", path),
                None => "Screen recording has stopped".into(),
            },
            ProcessState::Preview => "Preview started".into(),
            ProcessState::Paused => "Screen recording paused".into(),
        }
    }
}
