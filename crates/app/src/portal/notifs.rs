use crate::portal::portal_connection;
use crate::utils::types::ProcessState;
use ashpd::desktop::notification;

pub async fn send_notification(state: &ProcessState, timeout: u32) -> eyre::Result<()> {
    let proxy =
        notification::NotificationProxy::with_connection(portal_connection().await?).await?;
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
