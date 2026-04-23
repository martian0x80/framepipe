pub mod pipewire;
pub mod notifs;


// reuse didnt work out
async fn portal_connection() -> eyre::Result<zbus::Connection> {
    // let conn = CONNECTION
    //     .get_or_try_init(|| async {
    //         log::debug!("establishing new zbus connection to portal");
    //         zbus::Connection::session()
    //             .await
    //             .map_err(|e| eyre::eyre!("failed to establish zbus session connection: {e}"))
    //     })
    //     .await?;
    zbus::Connection::session()
        .await
        .map_err(|e| eyre::eyre!("failed to establish zbus session connection: {e}"))

    // Ok(conn.clone())
}