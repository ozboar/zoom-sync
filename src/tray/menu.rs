//! Menu construction and event handling

use muda::{
    accelerator::Accelerator, AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem,
    PredefinedMenuItem, Submenu,
};
use zoom_sync_core::Board;

use super::commands::{TrayCommand, TrayState};

/// Menu item IDs for event handling
pub mod ids {
    pub const STATUS: &str = "status";

    // Screen positions (radio group)
    pub const SCREEN_CPU: &str = "screen_cpu";
    pub const SCREEN_GPU: &str = "screen_gpu";
    pub const SCREEN_DOWNLOAD: &str = "screen_download";
    pub const SCREEN_TIME: &str = "screen_time";
    pub const SCREEN_WEATHER: &str = "screen_weather";
    pub const SCREEN_MELETRIX: &str = "screen_meletrix";
    pub const SCREEN_ZOOM65: &str = "screen_zoom65";
    pub const SCREEN_IMAGE: &str = "screen_image";
    pub const SCREEN_GIF: &str = "screen_gif";
    pub const SCREEN_BATTERY: &str = "screen_battery";

    // Screen navigation
    pub const NAV_UP: &str = "nav_up";
    pub const NAV_DOWN: &str = "nav_down";
    pub const NAV_SWITCH: &str = "nav_switch";

    // Settings toggles
    pub const TOGGLE_WEATHER: &str = "toggle_weather";
    pub const TOGGLE_SYSTEM: &str = "toggle_system";
    pub const TOGGLE_12HR: &str = "toggle_12hr";
    pub const TOGGLE_FAHRENHEIT: &str = "toggle_fahrenheit";
    #[cfg(target_os = "linux")]
    pub const TOGGLE_REACTIVE: &str = "toggle_reactive";

    // Media
    pub const UPLOAD_IMAGE: &str = "upload_image";
    pub const UPLOAD_GIF: &str = "upload_gif";
    pub const CLEAR_IMAGE: &str = "clear_image";
    pub const CLEAR_GIF: &str = "clear_gif";
    pub const CLEAR_ALL: &str = "clear_all";

    // Config
    pub const OPEN_CONFIG: &str = "open_config";
    pub const RELOAD_CONFIG: &str = "reload_config";

    // App
    pub const QUIT: &str = "quit";
}

/// Holds references to menu items that need dynamic updates
pub struct MenuItems {
    pub menu: Menu,
    pub status: MenuItem,
    // Submenus (dynamically added/removed based on board features)
    pub screen_submenu: Submenu,
    pub nav_submenu: Submenu,
    pub media_submenu: Submenu,
    // Track which feature menus are currently shown
    screen_menus_visible: std::cell::Cell<bool>,
    media_menu_visible: std::cell::Cell<bool>,
    // Screen position items
    pub screen_cpu: CheckMenuItem,
    pub screen_gpu: CheckMenuItem,
    pub screen_download: CheckMenuItem,
    pub screen_time: CheckMenuItem,
    pub screen_weather: CheckMenuItem,
    pub screen_meletrix: CheckMenuItem,
    pub screen_zoom65: CheckMenuItem,
    pub screen_image: CheckMenuItem,
    pub screen_gif: CheckMenuItem,
    pub screen_battery: CheckMenuItem,
    // Settings toggles
    pub toggle_weather: CheckMenuItem,
    pub toggle_system: CheckMenuItem,
    pub toggle_12hr: CheckMenuItem,
    pub toggle_fahrenheit: CheckMenuItem,
    #[cfg(target_os = "linux")]
    pub toggle_reactive: CheckMenuItem,
}

impl MenuItems {
    /// Update menu state based on board features
    pub fn update_from_state(&self, state: &TrayState, board: &mut Option<Box<dyn Board>>) {
        // Update connection status and check features
        let (status_text, has_screen, has_media) = match board.as_mut() {
            Some(b) => {
                let has_screen = b.as_screen().is_some();
                let has_media = b.as_image().is_some() || b.as_gif().is_some();
                (format!("{} Connected", b.info().name), has_screen, has_media)
            }
            None => ("Disconnected".to_string(), false, false),
        };
        self.status.set_text(status_text);

        // Add/remove screen menus based on feature
        let screen_visible = self.screen_menus_visible.get();
        if has_screen && !screen_visible {
            self.menu.insert(&self.screen_submenu, 2).unwrap();
            self.menu.insert(&self.nav_submenu, 3).unwrap();
            self.screen_menus_visible.set(true);
        } else if !has_screen && screen_visible {
            self.menu.remove(&self.screen_submenu).unwrap();
            self.menu.remove(&self.nav_submenu).unwrap();
            self.screen_menus_visible.set(false);
        }

        // Add/remove media menu based on feature
        let media_visible = self.media_menu_visible.get();
        // Position after: status, separator, [screen, nav]
        let media_position = if self.screen_menus_visible.get() { 4 } else { 2 };
        if has_media && !media_visible {
            self.menu.insert(&self.media_submenu, media_position).unwrap();
            self.media_menu_visible.set(true);
        } else if !has_media && media_visible {
            self.menu.remove(&self.media_submenu).unwrap();
            self.media_menu_visible.set(false);
        }

        // Update screen checkmarks to show current default
        let screen_items = [
            (&self.screen_cpu, "cpu"),
            (&self.screen_gpu, "gpu"),
            (&self.screen_download, "download"),
            (&self.screen_time, "time"),
            (&self.screen_weather, "weather"),
            (&self.screen_meletrix, "meletrix"),
            (&self.screen_zoom65, "zoom65"),
            (&self.screen_image, "image"),
            (&self.screen_gif, "gif"),
            (&self.screen_battery, "battery"),
        ];

        let default_screen = &state.config.general.initial_screen;
        for (item, id) in screen_items {
            item.set_checked(default_screen == id);
        }

        // Update toggles from config
        self.toggle_weather
            .set_checked(state.config.weather.enabled);
        self.toggle_system
            .set_checked(state.config.system_info.enabled);
        self.toggle_12hr
            .set_checked(state.config.general.use_12hr_time);
        self.toggle_fahrenheit
            .set_checked(state.config.general.fahrenheit);
        #[cfg(target_os = "linux")]
        self.toggle_reactive
            .set_checked(state.config.general.reactive_mode);
    }
}

/// Build the tray menu and return items for updates (menu is inside MenuItems)
pub fn build_menu(state: &TrayState) -> MenuItems {
    let menu = Menu::new();

    // Connection status (disabled, just for display)
    let status = MenuItem::with_id(
        ids::STATUS,
        format!("Status: {}", state.connection.as_str()),
        false,
        None::<Accelerator>,
    );
    menu.append(&status).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Screen position submenu
    let screen_submenu = Submenu::new("Set Screen", true);

    let screen_cpu = CheckMenuItem::with_id(
        ids::SCREEN_CPU,
        "CPU Temp",
        true,
        false,
        None::<Accelerator>,
    );
    let screen_gpu = CheckMenuItem::with_id(
        ids::SCREEN_GPU,
        "GPU Temp",
        true,
        false,
        None::<Accelerator>,
    );
    let screen_download = CheckMenuItem::with_id(
        ids::SCREEN_DOWNLOAD,
        "Download",
        true,
        false,
        None::<Accelerator>,
    );
    screen_submenu.append(&screen_cpu).unwrap();
    screen_submenu.append(&screen_gpu).unwrap();
    screen_submenu.append(&screen_download).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_time =
        CheckMenuItem::with_id(ids::SCREEN_TIME, "Time", true, false, None::<Accelerator>);
    let screen_weather = CheckMenuItem::with_id(
        ids::SCREEN_WEATHER,
        "Weather",
        true,
        false,
        None::<Accelerator>,
    );
    screen_submenu.append(&screen_time).unwrap();
    screen_submenu.append(&screen_weather).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_meletrix = CheckMenuItem::with_id(
        ids::SCREEN_MELETRIX,
        "Meletrix Logo",
        true,
        false,
        None::<Accelerator>,
    );
    let screen_zoom65 = CheckMenuItem::with_id(
        ids::SCREEN_ZOOM65,
        "Zoom65 Logo",
        true,
        false,
        None::<Accelerator>,
    );
    let screen_image = CheckMenuItem::with_id(
        ids::SCREEN_IMAGE,
        "Custom Image",
        true,
        false,
        None::<Accelerator>,
    );
    let screen_gif = CheckMenuItem::with_id(
        ids::SCREEN_GIF,
        "Custom GIF",
        true,
        false,
        None::<Accelerator>,
    );
    screen_submenu.append(&screen_meletrix).unwrap();
    screen_submenu.append(&screen_zoom65).unwrap();
    screen_submenu.append(&screen_image).unwrap();
    screen_submenu.append(&screen_gif).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_battery = CheckMenuItem::with_id(
        ids::SCREEN_BATTERY,
        "Battery",
        true,
        false,
        None::<Accelerator>,
    );
    screen_submenu.append(&screen_battery).unwrap();

    // Don't append screen_submenu yet - added dynamically when connected

    // Screen navigation submenu
    let nav_submenu = Submenu::new("Screen Navigation", true);
    nav_submenu
        .append(&MenuItem::with_id(
            ids::NAV_UP,
            "Up",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    nav_submenu
        .append(&MenuItem::with_id(
            ids::NAV_DOWN,
            "Down",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    nav_submenu
        .append(&MenuItem::with_id(
            ids::NAV_SWITCH,
            "Switch",
            true,
            None::<Accelerator>,
        ))
        .unwrap();

    // Don't append nav_submenu yet - added dynamically when connected

    // Media submenu
    let media_submenu = Submenu::new("Media", true);
    media_submenu
        .append(&MenuItem::with_id(
            ids::UPLOAD_IMAGE,
            "Upload Image...",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    media_submenu
        .append(&MenuItem::with_id(
            ids::UPLOAD_GIF,
            "Upload GIF...",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    media_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();
    media_submenu
        .append(&MenuItem::with_id(
            ids::CLEAR_IMAGE,
            "Clear Image",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    media_submenu
        .append(&MenuItem::with_id(
            ids::CLEAR_GIF,
            "Clear GIF",
            true,
            None::<Accelerator>,
        ))
        .unwrap();
    media_submenu
        .append(&MenuItem::with_id(
            ids::CLEAR_ALL,
            "Clear All Media",
            true,
            None::<Accelerator>,
        ))
        .unwrap();

    // Don't append media_submenu yet - added dynamically when connected

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Settings toggles (inlined)
    let toggle_weather = CheckMenuItem::with_id(
        ids::TOGGLE_WEATHER,
        "Weather Updates",
        true,
        state.config.weather.enabled,
        None::<Accelerator>,
    );
    let toggle_system = CheckMenuItem::with_id(
        ids::TOGGLE_SYSTEM,
        "System Info Updates",
        true,
        state.config.system_info.enabled,
        None::<Accelerator>,
    );
    let toggle_12hr = CheckMenuItem::with_id(
        ids::TOGGLE_12HR,
        "12-Hour Time",
        true,
        state.config.general.use_12hr_time,
        None::<Accelerator>,
    );
    let toggle_fahrenheit = CheckMenuItem::with_id(
        ids::TOGGLE_FAHRENHEIT,
        "Fahrenheit",
        true,
        state.config.general.fahrenheit,
        None::<Accelerator>,
    );
    menu.append(&toggle_weather).unwrap();
    menu.append(&toggle_system).unwrap();
    menu.append(&toggle_12hr).unwrap();
    menu.append(&toggle_fahrenheit).unwrap();

    #[cfg(target_os = "linux")]
    let toggle_reactive = {
        let toggle = CheckMenuItem::with_id(
            ids::TOGGLE_REACTIVE,
            "Reactive Mode",
            true,
            state.config.general.reactive_mode,
            None::<Accelerator>,
        );
        menu.append(&toggle).unwrap();
        toggle
    };

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Config options
    menu.append(&MenuItem::with_id(
        ids::OPEN_CONFIG,
        "Open Config File",
        true,
        None::<Accelerator>,
    ))
    .unwrap();
    menu.append(&MenuItem::with_id(
        ids::RELOAD_CONFIG,
        "Reload Config",
        true,
        None::<Accelerator>,
    ))
    .unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // About and Quit
    menu.append(&PredefinedMenuItem::about(
        Some("About zoom-sync"),
        Some(AboutMetadata {
            name: Some("zoom-sync".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            authors: Some(vec!["ozwaldorf".into()]),
            website: Some("https://github.com/ozwaldorf/zoom-sync".into()),
            ..Default::default()
        }),
    ))
    .unwrap();
    menu.append(&MenuItem::with_id(
        ids::QUIT,
        "Quit",
        true,
        None::<Accelerator>,
    ))
    .unwrap();

    let items = MenuItems {
        menu,
        status,
        screen_submenu,
        nav_submenu,
        media_submenu,
        screen_menus_visible: std::cell::Cell::new(false),
        media_menu_visible: std::cell::Cell::new(false),
        screen_cpu,
        screen_gpu,
        screen_download,
        screen_time,
        screen_weather,
        screen_meletrix,
        screen_zoom65,
        screen_image,
        screen_gif,
        screen_battery,
        toggle_weather,
        toggle_system,
        toggle_12hr,
        toggle_fahrenheit,
        #[cfg(target_os = "linux")]
        toggle_reactive,
    };

    // Initial state is disconnected - menus will be added when board connects
    items
}

/// Menu event that may require async handling
pub enum MenuAction {
    /// Immediate command
    Command(TrayCommand),
    /// Need to pick an image file (async)
    PickImage,
    /// Need to pick a gif file (async)
    PickGif,
    /// No action needed
    None,
}

/// Handle a menu event and return the appropriate action
pub fn handle_menu_event(event: MenuEvent) -> MenuAction {
    let id = event.id().0.as_str();
    match id {
        // Screen positions
        ids::SCREEN_CPU => MenuAction::Command(TrayCommand::SetScreen("cpu")),
        ids::SCREEN_GPU => MenuAction::Command(TrayCommand::SetScreen("gpu")),
        ids::SCREEN_DOWNLOAD => MenuAction::Command(TrayCommand::SetScreen("download")),
        ids::SCREEN_TIME => MenuAction::Command(TrayCommand::SetScreen("time")),
        ids::SCREEN_WEATHER => MenuAction::Command(TrayCommand::SetScreen("weather")),
        ids::SCREEN_MELETRIX => MenuAction::Command(TrayCommand::SetScreen("meletrix")),
        ids::SCREEN_ZOOM65 => MenuAction::Command(TrayCommand::SetScreen("zoom65")),
        ids::SCREEN_IMAGE => MenuAction::Command(TrayCommand::SetScreen("image")),
        ids::SCREEN_GIF => MenuAction::Command(TrayCommand::SetScreen("gif")),
        ids::SCREEN_BATTERY => MenuAction::Command(TrayCommand::SetScreen("battery")),

        // Navigation
        ids::NAV_UP => MenuAction::Command(TrayCommand::ScreenUp),
        ids::NAV_DOWN => MenuAction::Command(TrayCommand::ScreenDown),
        ids::NAV_SWITCH => MenuAction::Command(TrayCommand::ScreenSwitch),

        // Toggles
        ids::TOGGLE_WEATHER => MenuAction::Command(TrayCommand::ToggleWeather),
        ids::TOGGLE_SYSTEM => MenuAction::Command(TrayCommand::ToggleSystemInfo),
        ids::TOGGLE_12HR => MenuAction::Command(TrayCommand::Toggle12HrTime),
        ids::TOGGLE_FAHRENHEIT => MenuAction::Command(TrayCommand::ToggleFahrenheit),
        #[cfg(target_os = "linux")]
        ids::TOGGLE_REACTIVE => MenuAction::Command(TrayCommand::ToggleReactiveMode),

        // Media - file dialogs need async handling
        ids::UPLOAD_IMAGE => MenuAction::PickImage,
        ids::UPLOAD_GIF => MenuAction::PickGif,
        ids::CLEAR_IMAGE => MenuAction::Command(TrayCommand::ClearImage),
        ids::CLEAR_GIF => MenuAction::Command(TrayCommand::ClearGif),
        ids::CLEAR_ALL => MenuAction::Command(TrayCommand::ClearAllMedia),

        // Config
        ids::OPEN_CONFIG => {
            open_config_file();
            MenuAction::None
        },
        ids::RELOAD_CONFIG => MenuAction::Command(TrayCommand::ReloadConfig),

        // Quit
        ids::QUIT => MenuAction::Command(TrayCommand::Quit),

        _ => MenuAction::None,
    }
}

fn open_config_file() {
    if let Some(path) = crate::config::Config::path() {
        if path.exists() {
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg(&path).spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("notepad").arg(&path).spawn();
            }
        }
    }
}
