use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let show_hide = MenuItemBuilder::with_id("show-hide", "Show/Hide Flamel Desktop").build(app)?;
    let open_web = MenuItemBuilder::with_id("tray-open-web", "Open Flamel Desktop").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Flamel Desktop").build(app)?;

    let menu = Menu::with_items(app, &[&show_hide, &open_web, &quit])?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/icon.png"))
        .expect("failed to load tray icon");

    let tray = TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Flamel Desktop")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show-hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false)
                        && !window.is_minimized().unwrap_or(false)
                    {
                        let _ = window.hide();
                    } else {
                        show_main_window(app);
                    }
                }
            }
            "tray-open-web" => {
                show_main_window(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(tray)
}
