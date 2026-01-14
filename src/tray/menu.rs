//! Menu construction and event handling

use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedSender;

use muda::{
    accelerator::Accelerator, AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem,
    PredefinedMenuItem, Submenu,
};
use zoom65v3::types::ScreenPosition;

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
    pub status: MenuItem,
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
    pub toggle_weather: CheckMenuItem,
    pub toggle_system: CheckMenuItem,
    pub toggle_12hr: CheckMenuItem,
    pub toggle_fahrenheit: CheckMenuItem,
    #[cfg(target_os = "linux")]
    pub toggle_reactive: CheckMenuItem,
}

impl MenuItems {
    /// Update menu state from TrayState
    pub fn update_from_state(&self, state: &TrayState) {
        // Update connection status
        self.status
            .set_text(format!("Status: {}", state.connection.as_str()));

        // Update screen position radio buttons
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

        // Determine current screen string
        let current = state
            .current_screen
            .map(|s| screen_position_to_id(&s))
            .unwrap_or_default();

        for (item, id) in screen_items {
            item.set_checked(current == id);
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

fn screen_position_to_id(pos: &ScreenPosition) -> &'static str {
    use zoom65v3::types::{LogoOffset, SystemOffset, TimeOffset};
    match pos {
        ScreenPosition::System(SystemOffset::CpuTemp) => "cpu",
        ScreenPosition::System(SystemOffset::GpuTemp) => "gpu",
        ScreenPosition::System(SystemOffset::Download) => "download",
        ScreenPosition::Time(TimeOffset::Time) => "time",
        ScreenPosition::Time(TimeOffset::Weather) => "weather",
        ScreenPosition::Logo(LogoOffset::Meletrix) => "meletrix",
        ScreenPosition::Logo(LogoOffset::Zoom65) => "zoom65",
        ScreenPosition::Logo(LogoOffset::Image) => "image",
        ScreenPosition::Logo(LogoOffset::Gif) => "gif",
        ScreenPosition::Battery => "battery",
    }
}

/// Build the tray menu and return the menu + items for updates
pub fn build_menu(state: &TrayState) -> (Menu, MenuItems) {
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
    let screen_submenu = Submenu::new("Screen Position", true);

    let screen_cpu =
        CheckMenuItem::with_id(ids::SCREEN_CPU, "CPU Temp", true, false, None::<Accelerator>);
    let screen_gpu =
        CheckMenuItem::with_id(ids::SCREEN_GPU, "GPU Temp", true, false, None::<Accelerator>);
    let screen_download =
        CheckMenuItem::with_id(ids::SCREEN_DOWNLOAD, "Download", true, false, None::<Accelerator>);
    screen_submenu.append(&screen_cpu).unwrap();
    screen_submenu.append(&screen_gpu).unwrap();
    screen_submenu.append(&screen_download).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_time =
        CheckMenuItem::with_id(ids::SCREEN_TIME, "Time", true, false, None::<Accelerator>);
    let screen_weather =
        CheckMenuItem::with_id(ids::SCREEN_WEATHER, "Weather", true, false, None::<Accelerator>);
    screen_submenu.append(&screen_time).unwrap();
    screen_submenu.append(&screen_weather).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_meletrix =
        CheckMenuItem::with_id(ids::SCREEN_MELETRIX, "Meletrix Logo", true, false, None::<Accelerator>);
    let screen_zoom65 =
        CheckMenuItem::with_id(ids::SCREEN_ZOOM65, "Zoom65 Logo", true, false, None::<Accelerator>);
    let screen_image =
        CheckMenuItem::with_id(ids::SCREEN_IMAGE, "Custom Image", true, false, None::<Accelerator>);
    let screen_gif =
        CheckMenuItem::with_id(ids::SCREEN_GIF, "Custom GIF", true, false, None::<Accelerator>);
    screen_submenu.append(&screen_meletrix).unwrap();
    screen_submenu.append(&screen_zoom65).unwrap();
    screen_submenu.append(&screen_image).unwrap();
    screen_submenu.append(&screen_gif).unwrap();
    screen_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

    let screen_battery =
        CheckMenuItem::with_id(ids::SCREEN_BATTERY, "Battery", true, false, None::<Accelerator>);
    screen_submenu.append(&screen_battery).unwrap();

    menu.append(&screen_submenu).unwrap();

    // Screen navigation submenu
    let nav_submenu = Submenu::new("Screen Navigation", true);
    nav_submenu
        .append(&MenuItem::with_id(ids::NAV_UP, "Up", true, None::<Accelerator>))
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
    menu.append(&nav_submenu).unwrap();

    // Settings submenu
    let settings_submenu = Submenu::new("Settings", true);

    let toggle_weather = CheckMenuItem::with_id(
        ids::TOGGLE_WEATHER,
        "Enable Weather",
        true,
        state.config.weather.enabled,
        None::<Accelerator>,
    );
    let toggle_system = CheckMenuItem::with_id(
        ids::TOGGLE_SYSTEM,
        "Enable System Info",
        true,
        state.config.system_info.enabled,
        None::<Accelerator>,
    );
    settings_submenu.append(&toggle_weather).unwrap();
    settings_submenu.append(&toggle_system).unwrap();
    settings_submenu
        .append(&PredefinedMenuItem::separator())
        .unwrap();

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
    settings_submenu.append(&toggle_12hr).unwrap();
    settings_submenu.append(&toggle_fahrenheit).unwrap();

    #[cfg(target_os = "linux")]
    let toggle_reactive = {
        settings_submenu
            .append(&PredefinedMenuItem::separator())
            .unwrap();
        let toggle = CheckMenuItem::with_id(
            ids::TOGGLE_REACTIVE,
            "Reactive Mode",
            true,
            state.config.general.reactive_mode,
            None::<Accelerator>,
        );
        settings_submenu.append(&toggle).unwrap();
        toggle
    };

    menu.append(&settings_submenu).unwrap();

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
    menu.append(&media_submenu).unwrap();

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
    menu.append(&MenuItem::with_id(ids::QUIT, "Quit", true, None::<Accelerator>))
        .unwrap();

    let items = MenuItems {
        status,
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

    // Set initial state
    items.update_from_state(state);

    (menu, items)
}

/// Handle a menu event and send appropriate command
pub fn handle_menu_event(event: MenuEvent, cmd_tx: &UnboundedSender<TrayCommand>) -> bool {
    use zoom65v3::types::{LogoOffset, SystemOffset, TimeOffset};

    let id = event.id().0.as_str();
    let cmd = match id {
        // Screen positions
        ids::SCREEN_CPU => Some(TrayCommand::SetScreen(ScreenPosition::System(
            SystemOffset::CpuTemp,
        ))),
        ids::SCREEN_GPU => Some(TrayCommand::SetScreen(ScreenPosition::System(
            SystemOffset::GpuTemp,
        ))),
        ids::SCREEN_DOWNLOAD => Some(TrayCommand::SetScreen(ScreenPosition::System(
            SystemOffset::Download,
        ))),
        ids::SCREEN_TIME => Some(TrayCommand::SetScreen(ScreenPosition::Time(
            TimeOffset::Time,
        ))),
        ids::SCREEN_WEATHER => Some(TrayCommand::SetScreen(ScreenPosition::Time(
            TimeOffset::Weather,
        ))),
        ids::SCREEN_MELETRIX => Some(TrayCommand::SetScreen(ScreenPosition::Logo(
            LogoOffset::Meletrix,
        ))),
        ids::SCREEN_ZOOM65 => Some(TrayCommand::SetScreen(ScreenPosition::Logo(
            LogoOffset::Zoom65,
        ))),
        ids::SCREEN_IMAGE => Some(TrayCommand::SetScreen(ScreenPosition::Logo(
            LogoOffset::Image,
        ))),
        ids::SCREEN_GIF => {
            Some(TrayCommand::SetScreen(ScreenPosition::Logo(LogoOffset::Gif)))
        }
        ids::SCREEN_BATTERY => Some(TrayCommand::SetScreen(ScreenPosition::Battery)),

        // Navigation
        ids::NAV_UP => Some(TrayCommand::ScreenUp),
        ids::NAV_DOWN => Some(TrayCommand::ScreenDown),
        ids::NAV_SWITCH => Some(TrayCommand::ScreenSwitch),

        // Toggles - the daemon handles reading current state and flipping it
        ids::TOGGLE_WEATHER => Some(TrayCommand::ToggleWeather),
        ids::TOGGLE_SYSTEM => Some(TrayCommand::ToggleSystemInfo),
        ids::TOGGLE_12HR => Some(TrayCommand::Toggle12HrTime),
        ids::TOGGLE_FAHRENHEIT => Some(TrayCommand::ToggleFahrenheit),
        #[cfg(target_os = "linux")]
        ids::TOGGLE_REACTIVE => Some(TrayCommand::ToggleReactiveMode),

        // Media - file dialogs handled separately
        ids::UPLOAD_IMAGE => {
            pick_image_file().map(TrayCommand::UploadImage)
        }
        ids::UPLOAD_GIF => {
            pick_gif_file().map(TrayCommand::UploadGif)
        }
        ids::CLEAR_IMAGE => Some(TrayCommand::ClearImage),
        ids::CLEAR_GIF => Some(TrayCommand::ClearGif),
        ids::CLEAR_ALL => Some(TrayCommand::ClearAllMedia),

        // Config
        ids::OPEN_CONFIG => {
            open_config_file();
            None
        }
        ids::RELOAD_CONFIG => Some(TrayCommand::ReloadConfig),

        // Quit
        ids::QUIT => Some(TrayCommand::Quit),

        _ => None,
    };

    if let Some(cmd) = cmd {
        let is_quit = matches!(cmd, TrayCommand::Quit);
        let _ = cmd_tx.send(cmd);
        return is_quit;
    }

    false
}

fn pick_image_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
        .set_title("Select Image")
        .pick_file()
}

fn pick_gif_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Animations", &["gif", "webp", "png", "apng"])
        .set_title("Select Animation")
        .pick_file()
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
