//! System tray interface for zoom-sync

use std::error::Error;
use std::time::Duration;

use muda::MenuEvent;
use tokio::sync::{mpsc, watch};
use tray_icon::TrayIconBuilder;

use crate::config::Config;

mod commands;
mod daemon;
mod menu;

pub use commands::{ConnectionStatus, TrayCommand, TrayState};

/// Icon bytes embedded at compile time
const ICON_BYTES: &[u8] = include_bytes!("../../assets/icon.png");

/// Run the tray application
pub fn run_tray_app() -> Result<(), Box<dyn Error>> {
    // Initialize GTK (required for libappindicator on Linux)
    #[cfg(target_os = "linux")]
    gtk::init()?;

    // Load or create config
    let config = Config::load_or_create()?;
    println!("config loaded from {:?}", Config::path());

    // Create channels for communication
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (state_tx, state_rx) = watch::channel(TrayState {
        connection: ConnectionStatus::Disconnected,
        current_screen: None,
        config: config.clone(),
    });

    // Spawn tokio runtime in background thread
    let daemon_config = config;
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(daemon::daemon_loop(cmd_rx, state_tx, daemon_config));
    });

    // Run tray on main thread
    run_tray_loop(cmd_tx, state_rx)
}

fn run_tray_loop(
    cmd_tx: mpsc::UnboundedSender<TrayCommand>,
    mut state_rx: watch::Receiver<TrayState>,
) -> Result<(), Box<dyn Error>> {
    // Load icon
    let icon = load_icon()?;

    // Build initial menu
    let initial_state = state_rx.borrow().clone();
    let (tray_menu, menu_items) = menu::build_menu(&initial_state);

    // Create tray icon
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("zoom-sync")
        .with_icon(icon)
        .build()?;

    // Get menu event receiver
    let menu_rx = MenuEvent::receiver();

    // Main event loop
    loop {
        // Process GTK events (required for libappindicator on Linux)
        #[cfg(target_os = "linux")]
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }

        // Check for state updates
        if state_rx.has_changed().unwrap_or(false) {
            let state = state_rx.borrow_and_update().clone();
            menu_items.update_from_state(&state);
        }

        // Check for menu events with timeout
        match menu_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => {
                if menu::handle_menu_event(event, &cmd_tx) {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

fn load_icon() -> Result<tray_icon::Icon, Box<dyn Error>> {
    let image = image::load_from_memory(ICON_BYTES)?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let icon = tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)?;
    Ok(icon)
}
