//! System tray interface for zoom-sync

use std::collections::HashMap;
use std::error::Error;
use std::io::{stdout, Seek, Write};
use std::mem::Discriminant;
use std::path::PathBuf;
use std::time::Duration;

use chrono::DurationRound;
use either::Either;
use futures::future::OptionFuture;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::AnimationDecoder;
use muda::MenuEvent;
use tokio_stream::StreamExt;
use tray_icon::TrayIconBuilder;
use zoom_sync_core::Board;

use crate::config::Config;
use crate::detection::BoardKind;
use crate::info::{apply_system, CpuTemp, GpuTemp};
use crate::media::{encode_gif, encode_image};
use crate::weather::apply_weather;

mod commands;
mod menu;

pub use commands::{ConnectionStatus, TrayCommand, TrayState};

/// Icon bytes embedded at compile time
const ICON_BYTES: &[u8] = include_bytes!("../../assets/icon.png");

/// Run the tray application
pub fn run_tray_app() -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async_tray_app())
}

async fn async_tray_app() -> Result<(), Box<dyn Error>> {
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
    };

    // Load icon and build menu
    let icon = load_icon()?;
    let (tray_menu, menu_items) = menu::build_menu(&state);

    // Create tray icon
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("zoom-sync")
        .with_icon(icon)
        .build()?;

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
    let mut gpu: Option<Either<GpuTemp, u8>> = None;

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
        std::pin::Pin<
            Box<
                tokio_stream::Timeout<
                    evdev::EventStream,
                >,
            >,
        >,
    > = None;
    #[cfg(not(target_os = "linux"))]
    let mut reactive_stream: Option<futures::stream::Empty<()>> = None;

    let mut is_reactive_running = false;

    loop {
        tokio::select! {
            // UI polling: GTK events + menu events
            _ = ui_interval.tick() => {
                // Process GTK events (required for libappindicator on Linux)
                #[cfg(target_os = "linux")]
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }

                // Process menu events
                while let Ok(event) = menu_rx.try_recv() {
                    match menu::handle_menu_event(event) {
                        menu::MenuAction::Command(cmd) => {
                            let _ = cmd_tx.send(cmd);
                        }
                        menu::MenuAction::PickImage => {
                            let tx = cmd_tx.clone();
                            tokio::spawn(async move {
                                if let Some(handle) = rfd::AsyncFileDialog::new()
                                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
                                    .set_title("Select Image")
                                    .pick_file()
                                    .await
                                {
                                    let _ = tx.send(TrayCommand::UploadImage(handle.path().to_path_buf()));
                                }
                            });
                        }
                        menu::MenuAction::PickGif => {
                            let tx = cmd_tx.clone();
                            tokio::spawn(async move {
                                if let Some(handle) = rfd::AsyncFileDialog::new()
                                    .add_filter("Animations", &["gif", "webp", "png", "apng"])
                                    .set_title("Select Animation")
                                    .pick_file()
                                    .await
                                {
                                    let _ = tx.send(TrayCommand::UploadGif(handle.path().to_path_buf()));
                                }
                            });
                        }
                        menu::MenuAction::None => {}
                    }
                }
            }

            // Process commands (deduplicated by type - only latest of each type)
            Some(cmd) = cmd_rx.recv() => {
                // Drain all pending commands into slots, keeping only latest of each type
                let mut slots: HashMap<Discriminant<TrayCommand>, TrayCommand> = HashMap::new();
                slots.insert(std::mem::discriminant(&cmd), cmd);
                while let Ok(cmd) = cmd_rx.try_recv() {
                    slots.insert(std::mem::discriminant(&cmd), cmd);
                }

                // Process deduplicated commands
                for cmd in slots.into_values() {
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
                    }
                }
            }

            // Try to connect if disconnected
            _ = retry_interval.tick(), if board.is_none() => {
                match BoardKind::Auto.as_board() {
                    Ok(mut b) => {
                        state.connection = ConnectionStatus::Connected;
                        menu_items.update_from_state(&state);
                        println!("connected to {}", b.info().name);

                        // Initialize temperature monitors
                        if state.config.system_info.enabled {
                            cpu = Some(Either::Left(CpuTemp::new(&state.config.system_info.cpu_source)));
                            gpu = Some(Either::Left(GpuTemp::new(state.config.system_info.gpu_device)));
                        }

                        // Set initial screen if configured
                        if let Some(screen) = b.as_screen() {
                            let initial = &state.config.general.initial_screen;
                            if screen.set_screen(initial).is_ok() {
                                state.current_screen = Some(initial.clone());
                                menu_items.update_from_state(&state);
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

                        // Initialize reactive mode (Linux only)
                        #[cfg(target_os = "linux")]
                        if state.config.general.reactive_mode {
                            println!("initializing reactive mode");
                            if let Some(screen) = b.as_screen() {
                                let _ = screen.set_screen("image");
                            }
                            reactive_stream = evdev::enumerate().find_map(|(_, device)| {
                                device
                                    .name()
                                    .unwrap()
                                    .contains(b.info().name)
                                    .then_some(
                                        device
                                            .into_event_stream()
                                            .map(|s| Box::pin(s.timeout(Duration::from_millis(500))))
                                            .ok(),
                                    )
                                    .flatten()
                            });
                        }

                        board = Some(b);
                    }
                    Err(e) => {
                        if state.connection != ConnectionStatus::Disconnected {
                            eprintln!("failed to connect: {e}");
                            state.connection = ConnectionStatus::Disconnected;
                            menu_items.update_from_state(&state);
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
                    Ok(Err(e)) => {
                        eprintln!("reactive stream error: {e}");
                        handle_disconnect(&mut board, &mut state, &menu_items);
                    }
                    #[cfg(target_os = "linux")]
                    Ok(Ok(ev)) if !is_reactive_running => {
                        if matches!(ev.kind(), evdev::InputEventKind::Key(_)) {
                            is_reactive_running = true;
                            if let Some(ref mut b) = board {
                                if let Some(screen) = b.as_screen() {
                                    let _ = screen.screen_switch();
                                }
                            }
                        }
                    }
                    Err(_) if is_reactive_running => {
                        is_reactive_running = false;
                        if let Some(ref mut b) = board {
                            if let Some(screen) = b.as_screen() {
                                let _ = screen.reset_screen();
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
}

async fn handle_command(
    cmd: TrayCommand,
    board: &mut Option<Box<dyn Board>>,
    state: &mut TrayState,
    menu_items: &menu::MenuItems,
    cpu: &mut Option<Either<CpuTemp, u8>>,
    gpu: &mut Option<Either<GpuTemp, u8>>,
    weather_args: &mut crate::weather::WeatherArgs,
) -> CommandResult {
    match cmd {
        TrayCommand::Quit => return CommandResult::Quit,

        TrayCommand::SetScreen(id) => {
            if let Some(ref mut b) = board {
                if let Some(screen) = b.as_screen() {
                    match screen.set_screen(id) {
                        Ok(()) => {
                            state.current_screen = Some(id.to_string());
                            menu_items.update_from_state(state);
                            println!("set screen to {id}");
                        }
                        Err(e) => eprintln!("failed to set screen: {e}"),
                    }
                }
            }
        }
        TrayCommand::ScreenUp => {
            if let Some(ref mut b) = board {
                if let Some(screen) = b.as_screen() {
                    if let Err(e) = screen.screen_up() {
                        eprintln!("screen up failed: {e}");
                    } else {
                        println!("screen up");
                    }
                }
            }
        }
        TrayCommand::ScreenDown => {
            if let Some(ref mut b) = board {
                if let Some(screen) = b.as_screen() {
                    if let Err(e) = screen.screen_down() {
                        eprintln!("screen down failed: {e}");
                    } else {
                        println!("screen down");
                    }
                }
            }
        }
        TrayCommand::ScreenSwitch => {
            if let Some(ref mut b) = board {
                if let Some(screen) = b.as_screen() {
                    if let Err(e) = screen.screen_switch() {
                        eprintln!("screen switch failed: {e}");
                    } else {
                        println!("screen switch");
                    }
                }
            }
        }

        TrayCommand::ToggleWeather => {
            state.config.weather.enabled = !state.config.weather.enabled;
            *weather_args = build_weather_args(&state.config);
            let _ = state.config.save();
            menu_items.update_from_state(state);
            println!("weather: {}", state.config.weather.enabled);
        }
        TrayCommand::ToggleSystemInfo => {
            state.config.system_info.enabled = !state.config.system_info.enabled;
            if state.config.system_info.enabled && board.is_some() {
                *cpu = Some(Either::Left(CpuTemp::new(&state.config.system_info.cpu_source)));
                *gpu = Some(Either::Left(GpuTemp::new(state.config.system_info.gpu_device)));
            }
            let _ = state.config.save();
            menu_items.update_from_state(state);
            println!("system info: {}", state.config.system_info.enabled);
        }
        TrayCommand::Toggle12HrTime => {
            state.config.general.use_12hr_time = !state.config.general.use_12hr_time;
            if let Some(ref mut b) = board {
                let _ = crate::apply_time(b.as_mut(), state.config.general.use_12hr_time);
            }
            let _ = state.config.save();
            menu_items.update_from_state(state);
            println!("12hr time: {}", state.config.general.use_12hr_time);
        }
        TrayCommand::ToggleFahrenheit => {
            state.config.general.fahrenheit = !state.config.general.fahrenheit;
            let _ = state.config.save();
            menu_items.update_from_state(state);
            println!("fahrenheit: {}", state.config.general.fahrenheit);

            // Immediately update displays with new temperature unit
            if let Some(ref mut b) = board {
                if state.config.weather.enabled {
                    if let Err(e) = apply_weather(b.as_mut(), weather_args, state.config.general.fahrenheit).await {
                        eprintln!("weather update failed: {e}");
                    }
                }
                if state.config.system_info.enabled {
                    if let (Some(ref mut c), Some(ref g)) = (cpu, gpu) {
                        if let Err(e) = apply_system(b.as_mut(), state.config.general.fahrenheit, c, g, None) {
                            eprintln!("system update failed: {e}");
                        }
                    }
                }
            }
        }
        TrayCommand::ToggleReactiveMode => {
            state.config.general.reactive_mode = !state.config.general.reactive_mode;
            let _ = state.config.save();
            menu_items.update_from_state(state);
            println!("reactive mode: {} (restart required)", state.config.general.reactive_mode);
        }

        TrayCommand::UploadImage(path) => {
            if let Some(ref mut b) = board {
                if let Err(e) = upload_image(b.as_mut(), &path, &state.config) {
                    eprintln!("failed to upload image: {e}");
                }
            }
        }
        TrayCommand::UploadGif(path) => {
            if let Some(ref mut b) = board {
                if let Err(e) = upload_gif(b.as_mut(), &path, &state.config) {
                    eprintln!("failed to upload gif: {e}");
                }
            }
        }
        TrayCommand::ClearImage => {
            if let Some(ref mut b) = board {
                if let Some(image) = b.as_image() {
                    match image.clear_image() {
                        Ok(()) => println!("cleared image"),
                        Err(e) => eprintln!("failed to clear image: {e}"),
                    }
                }
            }
        }
        TrayCommand::ClearGif => {
            if let Some(ref mut b) = board {
                if let Some(gif) = b.as_gif() {
                    match gif.clear_gif() {
                        Ok(()) => println!("cleared gif"),
                        Err(e) => eprintln!("failed to clear gif: {e}"),
                    }
                }
            }
        }
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
        }

        TrayCommand::ReloadConfig => {
            if let Err(e) = state.config.reload() {
                eprintln!("failed to reload config: {e}");
            } else {
                println!("config reloaded");
                *weather_args = build_weather_args(&state.config);
            }
            menu_items.update_from_state(state);
        }
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
    menu_items.update_from_state(state);
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
    let image = image::load_from_memory(ICON_BYTES)?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let icon = tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)?;
    Ok(icon)
}

fn upload_image(
    board: &mut dyn Board,
    path: &PathBuf,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let (width, height) = board
        .as_screen_size()
        .ok_or("board does not support screen size")?;
    let image_handler = board
        .as_image()
        .ok_or("board does not support images")?;

    let bg = parse_hex_color(&config.media.background_color).unwrap_or([0, 0, 0]);
    let image = image::open(path)?;
    let encoded = encode_image(image, bg, config.media.use_nearest_neighbor, width, height)
        .ok_or("failed to encode image")?;

    let len = encoded.len();
    let total = len / 24;
    let progress_width = total.to_string().len();
    image_handler.upload_image(&encoded, &|i| {
        print!("\ruploading {len} bytes ({i:progress_width$}/{total}) ... ");
        stdout().flush().unwrap();
    })?;
    println!("done");
    Ok(())
}

fn upload_gif(
    board: &mut dyn Board,
    path: &PathBuf,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let (width, height) = board
        .as_screen_size()
        .ok_or("board does not support screen size")?;
    let gif_handler = board.as_gif().ok_or("board does not support gifs")?;

    let bg = parse_hex_color(&config.media.background_color).unwrap_or([0, 0, 0]);

    print!("decoding animation ... ");
    stdout().flush().unwrap();

    let decoder = image::ImageReader::open(path)?.with_guessed_format()?;
    let frames = match decoder.format() {
        Some(image::ImageFormat::Gif) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            Some(GifDecoder::new(reader)?.into_frames())
        }
        Some(image::ImageFormat::Png) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            let decoder = PngDecoder::new(reader)?;
            decoder
                .is_apng()?
                .then_some(decoder.apng().unwrap().into_frames())
        }
        Some(image::ImageFormat::WebP) => {
            let mut reader = decoder.into_inner();
            reader.seek(std::io::SeekFrom::Start(0))?;
            let decoder = WebPDecoder::new(reader)?;
            decoder.has_animation().then_some(decoder.into_frames())
        }
        _ => None,
    }
    .ok_or("failed to decode animation")?;

    println!("done");

    let encoded = encode_gif(frames, bg, config.media.use_nearest_neighbor, width, height)
        .ok_or("failed to encode gif")?;

    let len = encoded.len();
    let total = len / 24;
    let progress_width = total.to_string().len();
    gif_handler.upload_gif(&encoded, &|i| {
        print!("\ruploading {len} bytes ({i:progress_width$}/{total}) ... ");
        stdout().flush().unwrap();
    })?;
    println!("done");
    Ok(())
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
