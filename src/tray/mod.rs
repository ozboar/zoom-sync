//! System tray interface for zoom-sync

use std::error::Error;
use std::io::{stdout, Seek, Write};
use std::time::Duration;

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

use chrono::DurationRound;
use either::Either;
use futures::future::OptionFuture;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::AnimationDecoder;
use muda::MenuEvent;
use notify_rust::Notification;
use tokio_stream::StreamExt;
use tray_icon::TrayIconBuilder;
use zoom_sync_core::Board;

use crate::config::Config;
use crate::detection::BoardKind;
use crate::info::{apply_system, CpuTemp, GpuStats};
use crate::media::{encode_gif, encode_image};
use crate::weather::apply_weather;

mod commands;
mod menu;

pub use commands::{ConnectionStatus, TrayCommand, TrayState};

/// Icon bytes embedded at compile time
const ZOOM_ICON: &[u8] = include_bytes!("../../assets/zoom_icon.png");

/// Errors that can occur during image/gif processing
#[derive(Debug, thiserror::Error)]
pub enum ImageProcessingError {
    #[error("failed to open file: {0}")]
    OpenFile(#[from] std::io::Error),
    #[error("failed to decode image: {0}")]
    DecodeImage(#[from] image::ImageError),
    #[error("failed to encode image")]
    EncodeImage,
    #[error("failed to encode gif")]
    EncodeGif,
    #[error("png is not animated")]
    NotAnimatedPng,
    #[error("webp is not animated")]
    NotAnimatedWebp,
    #[error("unsupported animation format")]
    UnsupportedFormat,
}

/// Run the tray application
pub fn run_tray_app(board_kind: BoardKind) -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async_tray_app(board_kind))
}

async fn async_tray_app(board_kind: BoardKind) -> Result<(), Box<dyn Error>> {
    // Initialize GTK (required for libappindicator on Linux)
    #[cfg(target_os = "linux")]
    gtk::init()?;

    // Load or create config
    let config = Config::load_or_create()?;
    println!("config loaded from {:?}", Config::path());

    // Build initial state
    let mut state = TrayState {
        connection: ConnectionStatus::Disconnected,
        current_screen: None,
        config,
        reactive_active: false,
    };

    // Load icon and build menu
    let icon = load_icon()?;
    let menu_items = menu::build_menu(&state);

    // Create tray icon
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu_items.menu.clone()))
        .with_tooltip("zoom-sync")
        .with_icon(icon)
        .build()?;

    // Process GTK events to render tray icon before entering main loop
    #[cfg(target_os = "linux")]
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }

    // Process Win32 messages to render tray icon before entering main loop
    #[cfg(target_os = "windows")]
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Get menu event receiver
    let menu_rx = MenuEvent::receiver();

    // Internal command channel
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TrayCommand>();

    // UI polling interval
    let mut ui_interval = tokio::time::interval(Duration::from_millis(200));
    ui_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Board connection state
    let mut board: Option<Box<dyn Board>> = None;

    // Temperature monitors (initialized when board connects)
    let mut cpu: Option<Either<CpuTemp, u8>> = None;
    let mut gpu: Option<Either<GpuStats, (u32, u32)>> = None;

    // Weather args
    let mut weather_args = build_weather_args(&state.config);

    // Refresh intervals (skip missed ticks instead of bursting)
    let mut weather_interval = tokio::time::interval(state.config.refresh.weather);
    weather_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut system_interval = tokio::time::interval(state.config.refresh.system);
    system_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut retry_interval = tokio::time::interval(state.config.refresh.retry);
    retry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Time sync interval (only used in 12hr mode, syncs on the hour)
    let mut time_interval: Option<tokio::time::Interval> = None;

    // Reactive mode (Linux only)
    #[cfg(target_os = "linux")]
    let mut reactive_stream: Option<
        std::pin::Pin<Box<tokio_stream::Timeout<evdev::EventStream>>>,
    > = None;
    #[cfg(not(target_os = "linux"))]
    let mut reactive_stream: Option<futures::stream::Empty<()>> = None;

    let mut is_reactive_running = false;

    loop {
        tokio::select! {
            // UI polling: platform events + menu events
            _ = ui_interval.tick() => {
                // Process GTK events (required for libappindicator on Linux)
                #[cfg(target_os = "linux")]
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }

                // Process Win32 messages (required for tray-icon/muda on Windows)
                #[cfg(target_os = "windows")]
                unsafe {
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }

                // Process menu events
                while let Ok(event) = menu_rx.try_recv() {
                    match menu::handle_menu_event(event) {
                        menu::MenuAction::Command(cmd) => {
                            let _ = cmd_tx.send(cmd);
                        }
                        menu::MenuAction::PickImage => {
                            // Get encoding params before spawning
                            let screen_size = board.as_ref().and_then(|b| b.as_screen_size());
                            if let Some((width, height)) = screen_size {
                                let tx = cmd_tx.clone();
                                let bg = parse_hex_color(&state.config.media.background_color).unwrap_or([0, 0, 0]);
                                let nearest = state.config.media.use_nearest_neighbor;
                                tokio::spawn(async move {
                                    if let Some(handle) = rfd::AsyncFileDialog::new()
                                        .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
                                        .set_title("Select Image")
                                        .pick_file()
                                        .await
                                    {
                                        let path = handle.path().to_path_buf();
                                        // Encode in blocking thread
                                        let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ImageProcessingError> {
                                            let image = image::open(&path)?;
                                            encode_image(image, bg, nearest, width, height)
                                                .ok_or(ImageProcessingError::EncodeImage)
                                        }).await;
                                        match result {
                                            Ok(Ok(data)) => { let _ = tx.send(TrayCommand::UploadImage(data)); }
                                            Ok(Err(e)) => {
                                                eprintln!("{e}");
                                                notify_error(&e.to_string());
                                            }
                                            Err(e) => {
                                                eprintln!("image encoding task panicked: {e}");
                                                notify_error(&format!("Image encoding failed: {e}"));
                                            }
                                        }
                                    }
                                });
                            } else {
                                eprintln!("no board connected for image upload");
                            }
                        }
                        menu::MenuAction::PickGif => {
                            // Get encoding params before spawning
                            let screen_size = board.as_ref().and_then(|b| b.as_screen_size());
                            if let Some((width, height)) = screen_size {
                                let tx = cmd_tx.clone();
                                let bg = parse_hex_color(&state.config.media.background_color).unwrap_or([0, 0, 0]);
                                let nearest = state.config.media.use_nearest_neighbor;
                                tokio::spawn(async move {
                                    if let Some(handle) = rfd::AsyncFileDialog::new()
                                        .add_filter("Animations", &["gif", "webp", "png", "apng"])
                                        .set_title("Select Animation")
                                        .pick_file()
                                        .await
                                    {
                                        let path = handle.path().to_path_buf();
                                        // Decode and encode in blocking thread
                                        let result = tokio::task::spawn_blocking(move || {
                                            decode_and_encode_gif(&path, bg, nearest, width, height)
                                        }).await;
                                        match result {
                                            Ok(Ok(data)) => { let _ = tx.send(TrayCommand::UploadGif(data)); }
                                            Ok(Err(e)) => {
                                                eprintln!("{e}");
                                                notify_error(&e.to_string());
                                            }
                                            Err(e) => {
                                                eprintln!("gif encoding task panicked: {e}");
                                                notify_error(&format!("GIF encoding failed: {e}"));
                                            }
                                        }
                                    }
                                });
                            } else {
                                eprintln!("no board connected for gif upload");
                            }
                        }
                        menu::MenuAction::None => {}
                    }
                }
            }

            // Process commands
            Some(cmd) = cmd_rx.recv() => {
                match handle_command(
                    cmd,
                    &mut board,
                    &mut state,
                    &menu_items,
                    &mut cpu,
                    &mut gpu,
                    &mut weather_args,
                ).await {
                    CommandResult::Quit => return Ok(()),
                    CommandResult::Continue => {}
                    #[cfg(target_os = "linux")]
                    CommandResult::ToggleReactive => {
                        if state.reactive_active {
                            // Disable reactive mode
                            reactive_stream = None;
                            is_reactive_running = false;
                            state.reactive_active = false;
                            // Restore to default screen
                            state.config.general.initial_screen = "meletrix".into();
                            let _ = state.config.save();
                            println!("reactive mode disabled");
                        } else if let Some(ref mut b) = board {
                            // Enable reactive mode
                            if let Some(screen) = b.as_screen_pos() {
                                let _ = screen.set_screen("image");
                            }
                            let board_name = b.info().name.to_lowercase();
                            let search = format!("{board_name} keyboard");
                            reactive_stream = evdev::enumerate().find_map(|(_, device)| {
                                let name = device.name()?.to_string();
                                let name_lower = name.to_lowercase();
                                // Must contain board name + "keyboard" suffix
                                if name_lower.contains(&search) {
                                    device
                                        .into_event_stream()
                                        .map(|s| Box::pin(s.timeout(Duration::from_millis(500))))
                                        .ok()
                                } else {
                                    None
                                }
                            });
                            if reactive_stream.is_some() {
                                state.reactive_active = true;
                                state.config.general.initial_screen = "reactive".into();
                                let _ = state.config.save();
                                println!("reactive mode enabled");
                            } else {
                                eprintln!("reactive mode: no input device found (are you in the 'input' group?)");
                            }
                        }
                        menu_items.update_from_state(&state, &mut board);
                    }
                }
            }

            // Try to connect if disconnected
            _ = retry_interval.tick(), if board.is_none() => {
                match board_kind.as_board() {
                    Ok(mut b) => {
                        println!("connected to {}", b.info().name);
                        state.connection = ConnectionStatus::Connected;

                        // Initialize temperature monitors
                        if state.config.system_info.enabled {
                            cpu = Some(Either::Left(CpuTemp::new(&state.config.system_info.cpu_source)));
                            gpu = Some(Either::Left(GpuStats::new(state.config.system_info.gpu_device)));
                        }

                        // Initialize reactive mode if configured (Linux only)
                        #[cfg(target_os = "linux")]
                        if state.config.general.initial_screen == "reactive" {
                            println!("initializing reactive mode");
                            if let Some(screen) = b.as_screen_pos() {
                                let _ = screen.set_screen("image");
                            }
                            let board_name = b.info().name.to_lowercase();
                            reactive_stream = evdev::enumerate().find_map(|(_, device)| {
                                let name = device.name()?.to_string();
                                let name_lower = name.to_lowercase();
                                // Must contain board name + "keyboard" suffix
                                if name_lower.contains(&format!("{board_name} keyboard")) {
                                    device
                                        .into_event_stream()
                                        .map(|s| Box::pin(s.timeout(Duration::from_millis(500))))
                                        .ok()
                                } else {
                                    None
                                }
                            });
                            if reactive_stream.is_some() {
                                state.reactive_active = true;
                                println!("reactive mode enabled");
                            } else {
                                eprintln!("reactive mode: no input device found (are you in the 'input' group?)");
                            }
                        }

                        // Set initial screen if configured (skip for reactive mode)
                        #[cfg(target_os = "linux")]
                        let skip_initial = state.config.general.initial_screen == "reactive";
                        #[cfg(not(target_os = "linux"))]
                        let skip_initial = false;

                        if !skip_initial {
                            if let Some(screen) = b.as_screen_pos() {
                                let initial = &state.config.general.initial_screen;
                                if screen.set_screen(initial).is_ok() {
                                    state.current_screen = Some(initial.clone());
                                }
                            }
                        }

                        // Sync time immediately
                        if let Err(e) = crate::apply_time(b.as_mut(), state.config.general.use_12hr_time) {
                            eprintln!("time sync failed: {e}");
                        }

                        // Set up time interval for 12hr mode
                        if state.config.general.use_12hr_time {
                            time_interval = Some(create_hourly_interval());
                        }

                        // Set board, then update menu with features
                        board = Some(b);
                        menu_items.update_from_state(&state, &mut board);
                    }
                    Err(e) => {
                        if state.connection != ConnectionStatus::Disconnected {
                            eprintln!("failed to connect: {e}");
                            state.connection = ConnectionStatus::Disconnected;
                            menu_items.update_from_state(&state, &mut board);
                        }
                    }
                }
            }

            // Weather updates (only if board connected and enabled)
            _ = weather_interval.tick(), if board.is_some() && state.config.weather.enabled => {
                if let Some(ref mut b) = board {
                    match apply_weather(b.as_mut(), &mut weather_args, state.config.general.fahrenheit).await {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("weather update failed: {e}");
                            // Check if board disconnected
                            if e.to_string().contains("device") {
                                handle_disconnect(&mut board, &mut state, &menu_items);
                            }
                        }
                    }
                }
            }

            // System info updates (only if board connected and enabled)
            _ = system_interval.tick(), if board.is_some() && state.config.system_info.enabled => {
                if let Some(ref mut b) = board {
                    if let (Some(ref mut c), Some(ref g)) = (&mut cpu, &gpu) {
                        if let Err(e) = apply_system(
                            b.as_mut(),
                            state.config.general.fahrenheit,
                            c,
                            g,
                            None,
                        ) {
                            eprintln!("system update failed: {e}");
                            if e.to_string().contains("device") {
                                handle_disconnect(&mut board, &mut state, &menu_items);
                            }
                        }
                    }
                }
            }

            // Time sync (12hr mode, on the hour)
            Some(_) = OptionFuture::from(time_interval.as_mut().map(|i| i.tick())), if board.is_some() => {
                if let Some(ref mut b) = board {
                    if let Err(e) = crate::apply_time(b.as_mut(), state.config.general.use_12hr_time) {
                        eprintln!("time sync failed: {e}");
                        if e.to_string().contains("device") {
                            handle_disconnect(&mut board, &mut state, &menu_items);
                        }
                    }
                }
            }

            // Reactive mode keypress handling (Linux only)

            Some(Some(res)) = OptionFuture::from(reactive_stream.as_mut().map(|s| s.next())), if board.is_some() => {
                match res {
                    #[cfg(target_os = "linux")]
                    Ok(Err(e)) => {
                        eprintln!("reactive stream error: {e}");
                        handle_disconnect(&mut board, &mut state, &menu_items);
                    }
                    #[cfg(target_os = "linux")]
                    Ok(Ok(ev)) if !is_reactive_running => {
                        if matches!(ev.destructure(), evdev::EventSummary::Key(_, _, _)) {
                            is_reactive_running = true;
                            if let Some(ref mut b) = board {
                                if let Some(screen) = b.as_screen_nav() {
                                    let _ = screen.screen_switch();
                                }
                            }
                        }
                    }
                    #[cfg(target_os = "linux")]
                    Err(_) if is_reactive_running => {
                        is_reactive_running = false;
                        if let Some(ref mut b) = board {
                            if let Some(screen) = b.as_screen_nav() {
                                let _ = screen.screen_reset();
                                let _ = screen.screen_switch();
                                let _ = screen.screen_switch();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

enum CommandResult {
    Continue,
    Quit,
    /// Toggle reactive mode on/off (Linux only)
    #[cfg(target_os = "linux")]
    ToggleReactive,
}

async fn handle_command(
    cmd: TrayCommand,
    board: &mut Option<Box<dyn Board>>,
    state: &mut TrayState,
    menu_items: &menu::MenuItems,
    cpu: &mut Option<Either<CpuTemp, u8>>,
    gpu: &mut Option<Either<GpuStats, (u32, u32)>>,
    weather_args: &mut crate::weather::WeatherArgs,
) -> CommandResult {
    match cmd {
        TrayCommand::Quit => return CommandResult::Quit,

        TrayCommand::SetScreen(id) => {
            // Handle reactive mode specially (Linux only)
            #[cfg(target_os = "linux")]
            if id == "reactive" {
                return CommandResult::ToggleReactive;
            }

            if let Some(ref mut b) = board {
                if let Some(screen) = b.as_screen_pos() {
                    match screen.set_screen(id) {
                        Ok(()) => {
                            state.current_screen = Some(id.to_string());
                            // Also save as default
                            state.config.general.initial_screen = id.to_string();
                            let _ = state.config.save();
                            menu_items.update_from_state(state, board);
                            println!("set screen to {id}");
                        },
                        Err(e) => eprintln!("failed to set screen: {e}"),
                    }
                }
            }
        },

        TrayCommand::ToggleWeather => {
            state.config.weather.enabled = !state.config.weather.enabled;
            *weather_args = build_weather_args(&state.config);
            let _ = state.config.save();
            menu_items.update_from_state(state, board);
            println!("weather: {}", state.config.weather.enabled);
        },
        TrayCommand::ToggleSystemInfo => {
            state.config.system_info.enabled = !state.config.system_info.enabled;
            if state.config.system_info.enabled && board.is_some() {
                *cpu = Some(Either::Left(CpuTemp::new(
                    &state.config.system_info.cpu_source,
                )));
                *gpu = Some(Either::Left(GpuStats::new(
                    state.config.system_info.gpu_device,
                )));
            }
            let _ = state.config.save();
            menu_items.update_from_state(state, board);
            println!("system info: {}", state.config.system_info.enabled);
        },
        TrayCommand::Toggle12HrTime => {
            state.config.general.use_12hr_time = !state.config.general.use_12hr_time;
            if let Some(ref mut b) = board {
                let _ = crate::apply_time(b.as_mut(), state.config.general.use_12hr_time);
            }
            let _ = state.config.save();
            menu_items.update_from_state(state, board);
            println!("12hr time: {}", state.config.general.use_12hr_time);
        },
        TrayCommand::ToggleFahrenheit => {
            state.config.general.fahrenheit = !state.config.general.fahrenheit;
            let _ = state.config.save();
            menu_items.update_from_state(state, board);
            println!("fahrenheit: {}", state.config.general.fahrenheit);

            // Immediately update displays with new temperature unit
            if let Some(ref mut b) = board {
                if state.config.weather.enabled {
                    if let Err(e) =
                        apply_weather(b.as_mut(), weather_args, state.config.general.fahrenheit)
                            .await
                    {
                        eprintln!("weather update failed: {e}");
                    }
                }
                if state.config.system_info.enabled {
                    if let (Some(ref mut c), Some(ref g)) = (cpu, gpu) {
                        if let Err(e) =
                            apply_system(b.as_mut(), state.config.general.fahrenheit, c, g, None)
                        {
                            eprintln!("system update failed: {e}");
                        }
                    }
                }
            }
        },

        TrayCommand::UploadImage(encoded) => {
            if let Some(ref mut b) = board {
                if let Some(image_handler) = b.as_image() {
                    let len = encoded.len();
                    let total = len / 24;
                    let progress_width = total.to_string().len();
                    notify_uploading("Image");
                    let result = image_handler.upload_image(&encoded, &mut |i| {
                        print!("\ruploading {len} bytes ({i:progress_width$}/{total}) ... ");
                        stdout().flush().unwrap();
                    });
                    match result {
                        Ok(()) => {
                            println!("done");
                            notify_success("Image");
                        },
                        Err(e) => {
                            eprintln!("failed to upload image: {e}");
                            notify_error(&format!("Failed to upload image: {e}"));
                        },
                    }
                }
            }
        },
        TrayCommand::UploadGif(encoded) => {
            if let Some(ref mut b) = board {
                if let Some(gif_handler) = b.as_gif() {
                    let len = encoded.len();
                    let total = len / 24;
                    let progress_width = total.to_string().len();
                    notify_uploading("GIF");
                    let result = gif_handler.upload_gif(&encoded, &mut |i| {
                        print!("\ruploading {len} bytes ({i:progress_width$}/{total}) ... ");
                        stdout().flush().unwrap();
                    });
                    match result {
                        Ok(()) => {
                            println!("done");
                            notify_success("GIF");
                        },
                        Err(e) => {
                            eprintln!("failed to upload gif: {e}");
                            notify_error(&format!("Failed to upload GIF: {e}"));
                        },
                    }
                }
            }
        },
        TrayCommand::ClearImage => {
            if let Some(ref mut b) = board {
                if let Some(image) = b.as_image() {
                    match image.clear_image() {
                        Ok(()) => println!("cleared image"),
                        Err(e) => eprintln!("failed to clear image: {e}"),
                    }
                }
            }
        },
        TrayCommand::ClearGif => {
            if let Some(ref mut b) = board {
                if let Some(gif) = b.as_gif() {
                    match gif.clear_gif() {
                        Ok(()) => println!("cleared gif"),
                        Err(e) => eprintln!("failed to clear gif: {e}"),
                    }
                }
            }
        },
        TrayCommand::ClearAllMedia => {
            if let Some(ref mut b) = board {
                if let Some(image) = b.as_image() {
                    let _ = image.clear_image();
                }
                if let Some(gif) = b.as_gif() {
                    let _ = gif.clear_gif();
                }
                println!("cleared all media");
            }
        },

        TrayCommand::ReloadConfig => {
            if let Err(e) = state.config.reload() {
                eprintln!("failed to reload config: {e}");
            } else {
                println!("config reloaded");
                *weather_args = build_weather_args(&state.config);
            }
            menu_items.update_from_state(state, board);
        },
    }

    CommandResult::Continue
}

fn handle_disconnect(
    board: &mut Option<Box<dyn Board>>,
    state: &mut TrayState,
    menu_items: &menu::MenuItems,
) {
    *board = None;
    state.connection = ConnectionStatus::Reconnecting;
    menu_items.update_from_state(state, board);
}

fn build_weather_args(config: &Config) -> crate::weather::WeatherArgs {
    if config.weather.enabled {
        if let (Some(lat), Some(lon)) = (config.weather.latitude, config.weather.longitude) {
            crate::weather::WeatherArgs::Auto {
                coords: Some(crate::weather::Coords {
                    coords: (),
                    lat: lat as f32,
                    long: lon as f32,
                }),
            }
        } else {
            crate::weather::WeatherArgs::Auto { coords: None }
        }
    } else {
        crate::weather::WeatherArgs::Disabled
    }
}

fn create_hourly_interval() -> tokio::time::Interval {
    let now = chrono::Local::now();
    let delay = now
        .duration_trunc(chrono::TimeDelta::try_minutes(60).unwrap())
        .unwrap()
        .timestamp_millis()
        + 100
        - now.timestamp_millis();
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(delay as u64),
        Duration::from_secs(60 * 60),
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

fn load_icon() -> Result<tray_icon::Icon, Box<dyn Error>> {
    let image = image::load_from_memory(ZOOM_ICON)?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let icon = tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)?;
    Ok(icon)
}

/// Decode and encode a gif/animation file (runs in blocking thread)
fn decode_and_encode_gif(
    path: &std::path::Path,
    bg: [u8; 3],
    nearest: bool,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ImageProcessingError> {
    let decoder = image::ImageReader::open(path)?.with_guessed_format()?;

    let frames = match decoder.format() {
        Some(image::ImageFormat::Gif) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            GifDecoder::new(reader)?.into_frames()
        },
        Some(image::ImageFormat::Png) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            let png = PngDecoder::new(reader)?;
            if !png.is_apng()? {
                return Err(ImageProcessingError::NotAnimatedPng);
            }
            png.apng()?.into_frames()
        },
        Some(image::ImageFormat::WebP) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            let webp = WebPDecoder::new(reader)?;
            if !webp.has_animation() {
                return Err(ImageProcessingError::NotAnimatedWebp);
            }
            webp.into_frames()
        },
        _ => return Err(ImageProcessingError::UnsupportedFormat),
    };

    encode_gif(frames, bg, nearest, width, height).ok_or(ImageProcessingError::EncodeGif)
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Show an upload-in-progress notification
fn notify_uploading(kind: &str) {
    let _ = Notification::new()
        .summary("zoom-sync")
        .body(&format!("Uploading {kind}..."))
        .timeout(3000)
        .show();
}

/// Show a success notification
fn notify_success(kind: &str) {
    let _ = Notification::new()
        .summary("zoom-sync")
        .body(&format!("{kind} uploaded successfully"))
        .timeout(3000)
        .show();
}

/// Show an error notification
fn notify_error(message: &str) {
    let _ = Notification::new()
        .summary("zoom-sync: Error")
        .body(message)
        .timeout(5000)
        .show();
}
