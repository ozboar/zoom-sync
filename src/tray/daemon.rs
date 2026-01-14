//! Async daemon loop for tray mode

use std::error::Error;
use std::io::{stdout, Seek, Write};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedReceiver;
use std::time::Duration;

use either::Either;
use futures::future::OptionFuture;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::AnimationDecoder;
use tokio::sync::watch;
use tokio_stream::StreamExt;
use zoom65v3::types::ScreenPosition;
use zoom65v3::Zoom65v3;

use super::commands::{ConnectionStatus, TrayCommand, TrayState};
use crate::config::Config;
use crate::info::{apply_system, CpuTemp, GpuTemp};
use crate::media::{encode_gif, encode_image};
use crate::weather::apply_weather;

/// Main daemon loop that handles commands and keyboard sync
pub async fn daemon_loop(
    mut cmd_rx: UnboundedReceiver<TrayCommand>,
    state_tx: watch::Sender<TrayState>,
    config: Config,
) {
    let mut state = TrayState {
        connection: ConnectionStatus::Disconnected,
        current_screen: None,
        config,
    };

    loop {
        // Wait for command or retry timeout
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    TrayCommand::Quit => return,
                    TrayCommand::ReloadConfig => {
                        if let Err(e) = state.config.reload() {
                            eprintln!("failed to reload config: {e}");
                        }
                        let _ = state_tx.send(state.clone());
                    }
                    TrayCommand::ToggleWeather => {
                        state.config.weather.enabled = !state.config.weather.enabled;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                    }
                    TrayCommand::ToggleSystemInfo => {
                        state.config.system_info.enabled = !state.config.system_info.enabled;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                    }
                    TrayCommand::Toggle12HrTime => {
                        state.config.general.use_12hr_time = !state.config.general.use_12hr_time;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                    }
                    TrayCommand::ToggleFahrenheit => {
                        state.config.general.fahrenheit = !state.config.general.fahrenheit;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                    }
                    TrayCommand::ToggleReactiveMode => {
                        state.config.general.reactive_mode = !state.config.general.reactive_mode;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                    }
                    _ => {} // Other commands need keyboard connection
                }
                continue;
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }

        // Try to connect
        match Zoom65v3::open() {
            Ok(mut keyboard) => {
                state.connection = ConnectionStatus::Connected;
                let _ = state_tx.send(state.clone());
                println!("connected to keyboard");

                // Set initial screen if configured
                if let Ok(pos) = state.config.general.initial_screen.parse::<ScreenPosition>() {
                    if keyboard.set_screen(pos).is_ok() {
                        state.current_screen = Some(pos);
                        let _ = state_tx.send(state.clone());
                    }
                }

                // Run the connected loop
                if let Err(e) = run_connected(&mut keyboard, &mut cmd_rx, &state_tx, &mut state).await {
                    eprintln!("error: {e}");
                }

                state.connection = ConnectionStatus::Reconnecting;
                let _ = state_tx.send(state.clone());
            }
            Err(e) => {
                if state.connection != ConnectionStatus::Disconnected {
                    eprintln!("failed to connect: {e}");
                }
                state.connection = ConnectionStatus::Disconnected;
                let _ = state_tx.send(state.clone());
            }
        }

        // Wait before retry
        tokio::time::sleep(state.config.refresh.retry).await;
    }
}

async fn run_connected(
    keyboard: &mut Zoom65v3,
    cmd_rx: &mut UnboundedReceiver<TrayCommand>,
    state_tx: &watch::Sender<TrayState>,
    state: &mut TrayState,
) -> Result<(), Box<dyn Error>> {
    // Initialize monitors
    let mut cpu: Option<Either<CpuTemp, u8>> = if state.config.system_info.enabled {
        Some(Either::Left(CpuTemp::new(&state.config.system_info.cpu_source)))
    } else {
        None
    };

    let gpu: Option<Either<GpuTemp, u8>> = if state.config.system_info.enabled {
        Some(Either::Left(GpuTemp::new(state.config.system_info.gpu_device)))
    } else {
        None
    };

    // Sync time immediately
    crate::apply_time(keyboard, state.config.general.use_12hr_time)?;

    // Set up time interval for 12hr mode
    let mut time_interval = state.config.general.use_12hr_time.then(|| {
        let now = chrono::Local::now();
        use chrono::DurationRound;
        let delay = now
            .duration_trunc(chrono::TimeDelta::try_minutes(60).unwrap())
            .unwrap()
            .timestamp_millis()
            + 100
            - now.timestamp_millis();
        tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_millis(delay as u64),
            Duration::from_secs(60 * 60),
        )
    });

    let mut weather_interval = tokio::time::interval(state.config.refresh.weather);
    let mut system_interval = tokio::time::interval(state.config.refresh.system);

    // For weather args, create a simple auto mode
    let mut weather_args = if state.config.weather.enabled {
        if let (Some(lat), Some(lon)) = (
            state.config.weather.latitude,
            state.config.weather.longitude,
        ) {
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
    };

    // Reactive mode setup (Linux only)
    #[cfg(not(target_os = "linux"))]
    let mut reactive_stream: Option<
        Box<
            dyn tokio_stream::Stream<Item = Result<Result<(), std::io::Error>, Box<dyn Error>>>
                + Unpin,
        >,
    > = None;

    #[cfg(target_os = "linux")]
    let mut reactive_stream = if state.config.general.reactive_mode {
        println!("initializing reactive mode");
        keyboard.set_screen(zoom65v3::types::LogoOffset::Image.pos())?;
        evdev::enumerate().find_map(|(_, device)| {
            device
                .name()
                .unwrap()
                .contains("Zoom65 v3 Keyboard")
                .then_some(
                    device
                        .into_event_stream()
                        .map(|s| Box::pin(s.timeout(Duration::from_millis(500))))
                        .ok(),
                )
                .flatten()
        })
    } else {
        None
    };

    let mut is_reactive_running = false;

    loop {
        // Run the select loop for periodic updates AND commands
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    TrayCommand::Quit => return Ok(()),

                    TrayCommand::SetScreen(pos) => {
                        keyboard.set_screen(pos)?;
                        state.current_screen = Some(pos);
                        let _ = state_tx.send(state.clone());
                        println!("set screen to {:?}", pos);
                    }
                    TrayCommand::ScreenUp => {
                        keyboard.screen_up()?;
                        println!("screen up");
                    }
                    TrayCommand::ScreenDown => {
                        keyboard.screen_down()?;
                        println!("screen down");
                    }
                    TrayCommand::ScreenSwitch => {
                        keyboard.screen_switch()?;
                        println!("screen switch");
                    }

                    TrayCommand::ToggleWeather => {
                        state.config.weather.enabled = !state.config.weather.enabled;
                        weather_args = if state.config.weather.enabled {
                            crate::weather::WeatherArgs::Auto { coords: None }
                        } else {
                            crate::weather::WeatherArgs::Disabled
                        };
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                        println!("weather: {}", state.config.weather.enabled);
                    }
                    TrayCommand::ToggleSystemInfo => {
                        state.config.system_info.enabled = !state.config.system_info.enabled;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                        println!("system info: {}", state.config.system_info.enabled);
                    }
                    TrayCommand::Toggle12HrTime => {
                        state.config.general.use_12hr_time = !state.config.general.use_12hr_time;
                        crate::apply_time(keyboard, state.config.general.use_12hr_time)?;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                        println!("12hr time: {}", state.config.general.use_12hr_time);
                    }
                    TrayCommand::ToggleFahrenheit => {
                        state.config.general.fahrenheit = !state.config.general.fahrenheit;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                        println!("fahrenheit: {}", state.config.general.fahrenheit);

                        // Immediately update displays with new temperature unit
                        if state.config.weather.enabled {
                            if let Err(e) = apply_weather(keyboard, &mut weather_args, state.config.general.fahrenheit).await {
                                eprintln!("weather update failed: {e}");
                            }
                        }
                        if state.config.system_info.enabled {
                            if let (Some(ref mut c), Some(ref g)) = (&mut cpu, &gpu) {
                                if let Err(e) = apply_system(keyboard, state.config.general.fahrenheit, c, g, None) {
                                    eprintln!("system update failed: {e}");
                                }
                            }
                        }
                    }
                    TrayCommand::ToggleReactiveMode => {
                        state.config.general.reactive_mode = !state.config.general.reactive_mode;
                        let _ = state.config.save();
                        let _ = state_tx.send(state.clone());
                        println!("reactive mode: {} (restart required)", state.config.general.reactive_mode);
                    }

                    TrayCommand::UploadImage(path) => {
                        if let Err(e) = upload_image(keyboard, &path, &state.config) {
                            eprintln!("failed to upload image: {e}");
                        }
                    }
                    TrayCommand::UploadGif(path) => {
                        if let Err(e) = upload_gif(keyboard, &path, &state.config) {
                            eprintln!("failed to upload gif: {e}");
                        }
                    }
                    TrayCommand::ClearImage => {
                        keyboard.clear_image()?;
                        println!("cleared image");
                    }
                    TrayCommand::ClearGif => {
                        keyboard.clear_gif()?;
                        println!("cleared gif");
                    }
                    TrayCommand::ClearAllMedia => {
                        keyboard.clear_image()?;
                        keyboard.clear_gif()?;
                        println!("cleared all media");
                    }

                    TrayCommand::ReloadConfig => {
                        if let Err(e) = state.config.reload() {
                            eprintln!("failed to reload config: {e}");
                        } else {
                            println!("config reloaded");
                        }
                        let _ = state_tx.send(state.clone());
                    }
                }
            },
            Some(_) = OptionFuture::from(time_interval.as_mut().map(|i| i.tick())) => {
                crate::apply_time(keyboard, state.config.general.use_12hr_time)?;
            },
            _ = weather_interval.tick() => {
                if state.config.weather.enabled {
                    if let Err(e) = apply_weather(keyboard, &mut weather_args, state.config.general.fahrenheit).await {
                        eprintln!("weather update failed: {e}");
                    }
                }
            },
            _ = system_interval.tick() => {
                if state.config.system_info.enabled {
                    if let (Some(ref mut cpu), Some(ref gpu)) = (&mut cpu, &gpu) {
                        if let Err(e) = apply_system(
                            keyboard,
                            state.config.general.fahrenheit,
                            cpu,
                            gpu,
                            None,
                        ) {
                            eprintln!("system update failed: {e}");
                        }
                    }
                }
            },
            Some(Some(res)) = OptionFuture::from(reactive_stream.as_mut().map(|s| s.next())) => {
                match res {
                    Ok(Err(e)) => return Err(Box::new(e)),
                    #[cfg(target_os = "linux")]
                    Ok(Ok(ev)) if !is_reactive_running => {
                        if matches!(ev.kind(), evdev::InputEventKind::Key(_)) {
                            is_reactive_running = true;
                            keyboard.screen_switch()?;
                        }
                    },
                    Err(_) if is_reactive_running => {
                        is_reactive_running = false;
                        keyboard.reset_screen()?;
                        keyboard.screen_switch()?;
                        keyboard.screen_switch()?;
                    },
                    _ => {}
                }
            }
        }
    }
}

fn upload_image(
    keyboard: &mut Zoom65v3,
    path: &PathBuf,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let bg = parse_hex_color(&config.media.background_color).unwrap_or([0, 0, 0]);
    let image = image::open(path)?;
    let encoded = encode_image(image, bg, config.media.use_nearest_neighbor)
        .ok_or("failed to encode image")?;

    let len = encoded.len();
    let total = len / 24;
    let width = total.to_string().len();
    keyboard.upload_image(encoded, |i| {
        print!("\ruploading {len} bytes ({i:width$}/{total}) ... ");
        stdout().flush().unwrap();
    })?;
    println!("done");
    Ok(())
}

fn upload_gif(
    keyboard: &mut Zoom65v3,
    path: &PathBuf,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
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

    let encoded =
        encode_gif(frames, bg, config.media.use_nearest_neighbor).ok_or("failed to encode gif")?;

    let len = encoded.len();
    let total = len / 24;
    let width = total.to_string().len();
    keyboard.upload_gif(encoded, |i| {
        print!("\ruploading {len} bytes ({i:width$}/{total}) ... ");
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
