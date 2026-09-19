use tauri::{
    AppHandle, Emitter,
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
};

pub fn build_app_menu(app: &AppHandle, port: u16) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;

    // File menu
    let file_menu = SubmenuBuilder::new(app, "File")
        .item(
            &MenuItemBuilder::with_id("new-tab", "New Tab")
                .accelerator("CmdOrCtrl+T")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("close-tab", "Close Tab")
                .accelerator("CmdOrCtrl+W")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("open-project", "Open Project…")
                .accelerator("CmdOrCtrl+O")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("settings", "Settings")
                .accelerator("Ctrl+,")
                .build(app)?,
        )
        .separator()
        .quit()
        .build()?;

    // Edit menu
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    // View menu
    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .separator()
        .item(
            &MenuItemBuilder::with_id("zoom-in", "Zoom In")
                .accelerator("CmdOrCtrl+=")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("zoom-out", "Zoom Out")
                .accelerator("CmdOrCtrl+-")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("zoom-reset", "Reset Zoom")
                .accelerator("CmdOrCtrl+0")
                .build(app)?,
        )
        .build()?;

    // Window menu
    let open_browser_label = format!("Open in Browser  :{port}");
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .separator()
        .item(&MenuItemBuilder::with_id("open-browser", &open_browser_label).build(app)?)
        .build()?;

    menu.append(&file_menu)?;
    menu.append(&edit_menu)?;
    menu.append(&view_menu)?;
    menu.append(&window_menu)?;

    Ok(menu)
}

pub fn handle_menu_event(app: &AppHandle, event: &tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    match id {
        "new-tab" => {
            let _ = app.emit("menu:new-tab", ());
        }
        "close-tab" => {
            let _ = app.emit("menu:close-tab", ());
        }
        "open-project" => {
            let _ = app.emit("menu:open-project", ());
        }
        "settings" => {
            let _ = app.emit("menu:open-settings", ());
        }
        "open-browser" => {
            // Port is baked into the menu item label but we don't have it here easily.
            // The tray handler covers this use case more reliably.
            let _ = open::that("https://flamel.dev");
        }
        _ => {}
    }
}
