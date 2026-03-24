use std::iter::repeat_with;
use zbus::{Connection, proxy, zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type}};
use serde::{Serialize, Deserialize};

#[derive(SerializeDict, DeserializeDict, Type, Debug)]
#[zvariant(signature = "a{sv}")]
pub struct CreateSessionOptions {
    pub handle_token: String,
    pub session_handle_token: String,
}

#[derive(Type, Debug, Serialize, Deserialize)]
#[zvariant(signature = "(o)")]
pub struct CreateSessionResponse {
    pub session_handle: OwnedObjectPath,
}

#[derive(Type, Debug, SerializeDict, DeserializeDict)]
#[zvariant(signature = "a{sv}")]
pub struct SelectSourcesOptions {
    pub handle_token: String,
    #[zvariant(rename = "types")]
    pub source_type: u32,
    pub multiple: bool,
    pub cursor_mode: u32,
    pub restore_token: String,
    pub persist_mode: u32,
}

#[derive(Type, Debug, Serialize, Deserialize)]
#[zvariant(signature = "(o)")]
pub struct SelectSourcesResponse {
    pub handle: OwnedObjectPath,
}

#[proxy(
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop",
    interface = "org.freedesktop.portal.ScreenCast"
)]
trait XDGPortalScreenCast {
    fn create_session(&self, options: CreateSessionOptions) -> zbus::Result<CreateSessionResponse>;
    fn select_sources(&self, session_handle: OwnedObjectPath, options: SelectSourcesOptions) -> zbus::Result<SelectSourcesResponse>;
}


fn get_random_token() -> String {
    let token: String = repeat_with(fastrand::alphanumeric).take(16).collect();
    token
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to create session: {source}")]
    CreateSessionFailed {
        #[source]
        source: zbus::Error,
    },
    #[error("Failed to select sources: {source}")]
    SelectSourcesFailed {
        #[source]
        source: zbus::Error,
    },
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();

    let connection = Connection::session().await.unwrap();
    let proxy = XDGPortalScreenCastProxy::new(&connection).await.unwrap();

    let handle_token = get_random_token();
    let session_handle_token = get_random_token();

    let options = CreateSessionOptions {
        handle_token: handle_token.clone(),
        session_handle_token: session_handle_token.clone(),
    };

    log::debug!("Creating session with options: {:?}", options);

    let response = proxy.create_session(options).await.map_err(|e| Error::CreateSessionFailed { source: e })?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    log::info!("Session created with session handle (o): {}", response.session_handle);

    let select_sources_handle_token = get_random_token();

    let select_sources_options = SelectSourcesOptions {
        handle_token: handle_token.clone(),
        source_type: 1,
        multiple: false,
        cursor_mode: 1,
        restore_token: String::new(),
        persist_mode: 0,
    };

    log::debug!("Selecting sources with options: {:?}", select_sources_options);

    let response = proxy.select_sources(response.session_handle, select_sources_options).await.map_err(|e| Error::SelectSourcesFailed { source: e })?;

    log::info!("Sources selected with handle (o): {}", response.handle);

    Ok(())
}
