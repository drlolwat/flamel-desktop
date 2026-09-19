#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;
mod remote;
mod sso;
mod transport;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub struct Host {
    state: Arc<flamel::AppState>,
    runtime: Mutex<Option<Arc<flamel::native::Runtime>>>,
    remote: Mutex<Option<Arc<remote::Remote>>>,
    cloud: Mutex<Option<Arc<sso::Sso>>>,
}
impl Host {
    async fn get(&self) -> Result<Arc<flamel::native::Runtime>, String> {
        let mut runtime = self.runtime.lock().await;
        if let Some(value) = runtime.as_ref() {
            return Ok(value.clone());
        }
        let value = Arc::new(flamel::native::Runtime::open(self.state.clone(), None).await?);
        *runtime = Some(value.clone());
        Ok(value)
    }
}
pub enum Target {
    Local(Arc<flamel::native::Runtime>),
    Remote(Arc<remote::Remote>),
}
impl Target {
    async fn request(
        &self,
        input: flamel_protocol::Request,
        body: axum::body::Body,
    ) -> Result<axum::response::Response, String> {
        match self {
            Self::Local(value) => value.request(input, body).await,
            Self::Remote(value) => value.request(input, body).await,
        }
    }
    async fn socket(&self, path: &str) -> Result<flamel::native_socket::Socket, String> {
        match self {
            Self::Local(value) => value.socket(path),
            Self::Remote(value) => value.socket(path).await,
        }
    }
}
impl Host {
    async fn target(&self) -> Result<Target, String> {
        if let Some(value) = self.remote.lock().await.as_ref() {
            return Ok(Target::Remote(value.clone()));
        }
        self.get().await.map(Target::Local)
    }
}
#[tauri::command]
async fn cloud_login(
    host: tauri::State<'_, Host>,
    remember: bool,
) -> Result<(), String> {
    let cloud = sso::Sso::login(remember).await?;
    *host.state.service_access.write().await = Some(cloud.clone());
    *host.cloud.lock().await = Some(cloud);
    Ok(())
}
#[tauri::command]
async fn cloud_signed_in(host: tauri::State<'_, Host>) -> Result<bool, String> {
    Ok(host.cloud.lock().await.is_some())
}
#[tauri::command]
async fn cloud_servers(host: tauri::State<'_, Host>) -> Result<Vec<sso::Server>, String> {
    let cloud = host.cloud.lock().await.clone().ok_or("Sign in to Flamel first")?;
    cloud.servers().await
}
#[tauri::command]
async fn cloud_resume(host: tauri::State<'_, Host>) -> Result<Vec<sso::Server>, String> {
    let cloud = sso::Sso::restore().await?;
    let servers = cloud.servers().await?;
    *host.state.service_access.write().await = Some(cloud.clone());
    *host.cloud.lock().await = Some(cloud);
    Ok(servers)
}
#[tauri::command]
async fn cloud_connect(
    host: tauri::State<'_, Host>,
    transport: tauri::State<'_, transport::Transport>,
    server: String,
) -> Result<String, String> {
    let cloud = host
        .cloud
        .lock()
        .await
        .clone()
        .ok_or("Sign in to Flamel first")?;
    let remote = remote::Remote::from_cloud(cloud, server).await?;
    let base = remote.base.clone();
    transport.cancel_all().await;
    *host.remote.lock().await = Some(remote);
    Ok(base)
}
#[tauri::command]
async fn remote_resume(
    host: tauri::State<'_, Host>,
    transport: tauri::State<'_, transport::Transport>,
    url: String,
) -> Result<String, String> {
    let remote = remote::Remote::restore(&url).await?;
    let base = remote.base.clone();
    transport.cancel_all().await;
    *host.remote.lock().await = Some(remote);
    Ok(base)
}
#[tauri::command]
async fn remote_login(
    host: tauri::State<'_, Host>,
    transport: tauri::State<'_, transport::Transport>,
    url: String,
    email: String,
    password: String,
    remember: bool,
) -> Result<String, String> {
    let remote = remote::Remote::login(&url, &email, &password, remember).await?;
    let base = remote.base.clone();
    transport.cancel_all().await;
    *host.remote.lock().await = Some(remote);
    Ok(base)
}
#[tauri::command]
async fn remote_logout(
    host: tauri::State<'_, Host>,
    transport: tauri::State<'_, transport::Transport>,
) -> Result<(), String> {
    let mut remote = host.remote.lock().await;
    if let Some(value) = remote.as_ref() {
        value.logout().await?;
    }
    let mut cloud = host.cloud.lock().await;
    if let Some(value) = cloud.as_ref() {
        value.logout().await?;
    }
    *cloud = None;
    *host.state.service_access.write().await = None;
    transport.cancel_all().await;
    *remote = None;
    Ok(())
}
#[tauri::command]
async fn runtime_profiles(
    host: tauri::State<'_, Host>,
) -> Result<Vec<flamel::identity::Profile>, String> {
    flamel::identity::profiles(&host.state.db)
}
#[tauri::command]
async fn runtime_select_profile(
    host: tauri::State<'_, Host>,
    profile: String,
) -> Result<(), String> {
    let mut runtime = host.runtime.lock().await;
    if runtime.is_some() {
        return Err("Restart Desktop to switch local profiles".into());
    }
    *runtime = Some(Arc::new(
        flamel::native::Runtime::open(host.state.clone(), Some(&profile)).await?,
    ));
    Ok(())
}
#[tauri::command]
async fn cmd_start_local_server(
    host: tauri::State<'_, Host>,
    transport: tauri::State<'_, transport::Transport>,
) -> Result<String, String> {
    transport.cancel_all().await;
    *host.remote.lock().await = None;
    host.get().await?;
    Ok("http://flamel.local.invalid".into())
}
#[tauri::command]
async fn cmd_get_local_api_base() -> String {
    "http://flamel.local.invalid".into()
}
fn main() {
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("native runtime"),
    );
    let _guard = rt.enter();
    let state = flamel::AppState::new_with_mode(&flamel::db::default_db_path(), 0, true);
    let shutdown = state.clone();
    tauri::Builder::default()
        .manage(Host {
            state,
            runtime: Mutex::new(None),
            remote: Mutex::new(None),
            cloud: Mutex::new(None),
        })
        .manage(transport::Transport::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            desktop::tray::show_main_window(app)
        }))
        .invoke_handler(tauri::generate_handler![
            cloud_login,
            cloud_signed_in,
            cloud_servers,
            cloud_resume,
            cloud_connect,
            remote_resume,
            remote_login,
            remote_logout,
            runtime_profiles,
            runtime_select_profile,
            cmd_start_local_server,
            cmd_get_local_api_base,
            transport::runtime_reset,
            transport::runtime_open,
            transport::runtime_write,
            transport::runtime_head,
            transport::runtime_read,
            transport::runtime_cancel,
            transport::runtime_socket_open,
            transport::runtime_socket_send,
            desktop::commands::cmd_open_folder_dialog,
            desktop::commands::cmd_open_path,
            desktop::commands::cmd_open_external,
            desktop::commands::cmd_navigate,
            desktop::commands::cmd_notify,
            desktop::commands::cmd_minimize,
            desktop::commands::cmd_maximize,
            desktop::commands::cmd_close_window,
            desktop::commands::cmd_is_maximized,
            desktop::commands::cmd_start_dragging,
        ])
        .setup(|app| {
            if let Ok(resources) = app.path().resource_dir() {
                for path in [resources.join("resources/bin"), resources.join("bin")] {
                    if path
                        .join(if cfg!(windows) {
                            "flamel_mcp.exe"
                        } else {
                            "flamel_mcp"
                        })
                        .exists()
                    {
                        // Configure sidecar location before admitting native work.
                        unsafe {
                            std::env::set_var("FLAMEL_BIN_DIR", path);
                        }
                        break;
                    }
                }
            }

            desktop::tray::create_tray(app.handle())?;
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build Desktop")
        .run(move |app, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                let state = shutdown.clone();
                let transport = app.state::<transport::Transport>();
                rt.block_on(async {
                    transport.cancel_all().await;
                    state.shutdown_processes().await;
                });
            }
        });
}
