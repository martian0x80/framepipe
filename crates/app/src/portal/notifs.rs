use ashpd::desktop::notification;

use crate::portal::portal_connection;

pub enum ProcessState {
    Running,
    Stopped(String),
    Preview,
    Paused
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
            ProcessState::Stopped(path) => format!("Screen recording has stopped, file saved to {}", path),
            ProcessState::Preview => "Preview started".into(),
            ProcessState::Paused => "Screen recording paused".into(),
        }
    }
}


pub async fn send_notification(state: &ProcessState, timeout: u32) -> eyre::Result<()> {
    let proxy = notification::NotificationProxy::with_connection(portal_connection().await?).await?;
    let state_code = state.to_string();
    let id = format!("framepipe-notification-{}", state_code);
    let title = "Framepipe";

    let notification = notification::Notification::new(title)
        .body(state.to_body().as_str())
        .priority(notification::Priority::High);
    proxy.add_notification(id.as_str(), notification).await?;
    tokio::time::sleep(std::time::Duration::from_secs(timeout as u64)).await;
    proxy.remove_notification(id.as_str()).await?;
    Ok(())
}


