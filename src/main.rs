use std::error::Error;
use std::fmt::{Debug, Display};
use std::io::{stdout, Seek, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use bpaf::{Bpaf, Parser};
use chrono::{DurationRound, TimeDelta};
use either::Either;
use futures::future::OptionFuture;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::AnimationDecoder;
use tokio_stream::StreamExt;
use zoom65v3::Zoom65v3;

use crate::detection::{board_kind, BoardKind};
use crate::info::{apply_system, cpu_mode, gpu_mode, system_args, CpuMode, GpuMode, SystemArgs};
use crate::media::{encode_gif, encode_image};
use crate::screen::{apply_screen, screen_args, screen_args_with_reactive, ScreenArgs};
use crate::weather::{apply_weather, weather_args, WeatherArgs};

mod detection;
mod info;
mod media;
mod screen;
mod weather;

fn farenheit() -> impl Parser<bool> {
    bpaf::short('f')
        .long("farenheit")
        .help(
            "Use farenheit for all fetched temperatures. \
May cause clamping for anything greater than 99F. \
No effect on any manually provided data.",
        )
        .switch()
}

#[derive(Debug, Clone, Bpaf)]
struct RefreshArgs {
    /// Interval in seconds to refresh system data
    #[bpaf(short('S'), long, fallback(Duration::from_secs(10).into()), display_fallback)]
    refresh_system: humantime::Duration,
    /// Interval in seconds to refresh weather data
    #[bpaf(short('W'), long, fallback(Duration::from_secs(60 * 60).into()), display_fallback)]
    refresh_weather: humantime::Duration,
    /// Retry interval for reconnecting to keyboard
    #[bpaf(short('R'), long, fallback(Duration::from_secs(5).into()), display_fallback)]
    retry: humantime::Duration,
    /// Enable simulating 12hr time
    #[bpaf(long("12hr"), fallback(false), display_fallback)]
    _12hr: bool,
    #[bpaf(external)]
    farenheit: bool,
    #[bpaf(external(screen_args_with_reactive), optional)]
    screen_args: Option<ScreenArgs>,
    #[bpaf(external)]
    weather_args: WeatherArgs,
    #[bpaf(external)]
    system_args: SystemArgs,
}


#[derive(Clone, Debug, Bpaf)]
enum SetCommand {
    /// Sync time to system clock
    #[bpaf(command)]
    Time,
    /// Set weather data
    #[bpaf(command)]
    Weather {
        #[bpaf(external)]
        farenheit: bool,
        #[bpaf(external)]
        weather_args: WeatherArgs,
    },
    /// Set system info
    #[bpaf(command)]
    System {
        #[bpaf(external)]
        farenheit: bool,
        #[bpaf(external)]
        cpu_mode: CpuMode,
        #[bpaf(external)]
        gpu_mode: GpuMode,
        /// Manually set download speed
        #[bpaf(short, long)]
        download: Option<f32>,
    },
    /// Change current screen
    #[bpaf(command, fallback_to_usage)]
    Screen(#[bpaf(external(screen_args))] ScreenArgs),
    /// Upload static image
    #[bpaf(command, fallback_to_usage)]
    Image(#[bpaf(external(set_media_args))] SetMediaArgs),
    /// Upload animated image (gif/webp/apng)
    #[bpaf(command, fallback_to_usage)]
    Gif(#[bpaf(external(set_media_args))] SetMediaArgs),
    /// Clear all media files
    #[bpaf(command)]
    Clear,
}

#[derive(Clone, Debug, Bpaf)]
enum SetMediaArgs {
    Set {
        /// Use nearest neighbor interpolation when resizing, otherwise uses gaussian
        #[bpaf(short('n'), long("nearest"))]
        nearest: bool,
        /// Optional background color for transparent images
        #[bpaf(
            short,
            long,
            fallback(Color([0; 3])),
            display_fallback,
        )]
        bg: Color,
        /// Path to image to re-encode and upload
        #[bpaf(positional("PATH"), guard(|p| p.exists(), "file not found"))]
        path: PathBuf,
    },
    /// Delete the content, resetting back to the default.
    #[bpaf(command)]
    Clear,
}

/// Utility for easily parsing hex colors from bpaf
#[derive(Debug, Clone, Hash)]
struct Color(pub [u8; 3]);
impl Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [r, g, b] = self.0;
        f.write_str(&format!("#{r:02x}{g:02x}{b:02x}"))
    }
}
impl FromStr for Color {
    type Err = String;
    fn from_str(code: &str) -> Result<Self, Self::Err> {
        // parse hex string into rgb
        let mut hex = (*code).trim_start_matches('#').to_string();
        match hex.len() {
            3 => {
                // Extend 3 character hex colors
                hex = hex.chars().flat_map(|a| [a, a]).collect();
            },
            6 => {},
            l => return Err(format!("Invalid hex length for {code}: {l}")),
        }
        if let Ok(channel_bytes) = u32::from_str_radix(&hex, 16) {
            let r = ((channel_bytes >> 16) & 0xFF) as u8;
            let g = ((channel_bytes >> 8) & 0xFF) as u8;
            let b = (channel_bytes & 0xFF) as u8;
            Ok(Self([r, g, b]))
        } else {
            Err(format!("Invalid hex color: {code}"))
        }
    }
}

#[derive(Clone, Debug, Bpaf)]
#[bpaf(options, version, descr(env!("CARGO_PKG_DESCRIPTION")))]
enum Cli {
    /// Update the keyboard periodically in a loop, reconnecting on errors.
    Run(#[bpaf(external(refresh_args))] RefreshArgs),
    /// Set specific options on the keyboard.
    /// Must not be used while zoom-sync is already running.
    #[bpaf(command, fallback_to_usage)]
    Set {
        #[bpaf(external)]
        set_command: SetCommand,
    },
}

pub fn apply_time(keyboard: &mut Zoom65v3, _12hr: bool) -> Result<(), Box<dyn Error>> {
    let time = chrono::Local::now();
    keyboard
        .set_time(time, _12hr)
        .map_err(|e| format!("failed to set time: {e}"))?;
    println!("updated time to {time}");
    Ok(())
}

async fn refresh(mut args: RefreshArgs) -> Result<(), Box<dyn Error>> {
    let mut cpu = match &args.system_args {
        SystemArgs::Disabled => None,
        SystemArgs::Enabled { cpu_mode, .. } => Some(cpu_mode.either()),
    };
    let gpu = match &args.system_args {
        SystemArgs::Disabled => None,
        SystemArgs::Enabled { gpu_mode, .. } => Some(gpu_mode.either()),
    };

    loop {
        if let Err(e) = run(&mut args, &mut cpu, &gpu).await {
            eprintln!("error: {e}\nreconnecting in {} seconds...", args.retry);
            tokio::time::sleep(args.retry.into()).await;
        }
    }
}

async fn run(
    args: &mut RefreshArgs,
    cpu: &mut Option<Either<info::CpuTemp, u8>>,
    gpu: &Option<Either<info::GpuTemp, u8>>,
) -> Result<(), Box<dyn Error>> {
    let mut keyboard = Zoom65v3::open()?;
    println!("connected to keyboard");

    if let Some(ref args) = args.screen_args {
        #[cfg(not(target_os = "linux"))]
        {
            apply_screen(args, &mut keyboard)?;
            println!("set screen");
        }
        #[cfg(target_os = "linux")]
        if *args != ScreenArgs::Reactive {
            apply_screen(args, &mut keyboard)?;
            println!("set screen");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let mut reactive_stream: Option<
        Box<
            dyn tokio_stream::Stream<Item = Result<Result<(), std::io::Error>, Box<dyn Error>>>
                + Unpin,
        >,
    > = None;
    #[cfg(target_os = "linux")]
    let mut reactive_stream = args.screen_args.and_then(|args| match args {
        #[cfg(target_os = "linux")]
        ScreenArgs::Reactive => {
            println!("initializing reactive mode");
            keyboard
                .set_screen(zoom65v3::types::LogoOffset::Image.pos())
                .unwrap();
            let stream = evdev::enumerate().find_map(|(_, device)| {
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
            });
            if stream.is_none() {
                eprintln!("warning: couldn't find/access ev device");
            }
            stream
        },
        _ => None,
    });
    let mut is_reactive_running = false;

    // Sync time immediately, and if 12hr time is enabled, resync every next hour
    apply_time(&mut keyboard, args._12hr)?;
    let mut time_interval = args._12hr.then_some({
        let now = chrono::Local::now();

        let delay = now
            .duration_trunc(TimeDelta::try_minutes(60).unwrap())
            .unwrap()
            .timestamp_millis()
            + 100
            - now.timestamp_millis();

        tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_millis(delay as u64),
            Duration::from_secs(60 * 60),
        )
    });
    let mut weather_interval = tokio::time::interval(args.refresh_weather.into());
    let mut system_interval = tokio::time::interval(args.refresh_system.into());

    loop {
        tokio::select! {
            Some(_) = OptionFuture::from(time_interval.as_mut().map(|i| i.tick())) => {
                apply_time(&mut keyboard, args._12hr)?;
            },
            _ = weather_interval.tick() => {
                apply_weather(&mut keyboard, &mut args.weather_args, args.farenheit).await?
            },
            _ = system_interval.tick() => {
                if let SystemArgs::Enabled { download, .. } = args.system_args {
                    apply_system(
                        &mut keyboard,
                        args.farenheit,
                        cpu.as_mut().unwrap(),
                        gpu.as_ref().unwrap(),
                        download,
                    )?;
                }
            },
            Some(Some(res)) = {
                OptionFuture::from(reactive_stream.as_mut().map(|s| s.next()))
            } => {
                match res {
                    Ok(Err(e)) => return Err(Box::new(e)),
                    // keypress, play gif if not already running
                    #[cfg(target_os = "linux")]
                    Ok(Ok(ev)) if !is_reactive_running => {
                        if matches!(ev.kind(), evdev::InputEventKind::Key(_)) {
                            is_reactive_running = true;
                            keyboard.screen_switch()?;
                        }
                    },
                    // timeout, reset back to image
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = cli().run();
    match args {
        Cli::Run(args) => refresh(args).await,
        Cli::Set { set_command } => {
            let mut keyboard = Zoom65v3::open()?;
            match set_command {
                SetCommand::Time => apply_time(&mut keyboard, false),
                SetCommand::Weather {
                    farenheit,
                    mut weather_args,
                } => apply_weather(&mut keyboard, &mut weather_args, farenheit).await,
                SetCommand::System {
                    farenheit,
                    cpu_mode,
                    gpu_mode,
                    download,
                } => apply_system(
                    &mut keyboard,
                    farenheit,
                    &mut cpu_mode.either(),
                    &gpu_mode.either(),
                    download,
                ),
                SetCommand::Screen(args) => apply_screen(&args, &mut keyboard),
                SetCommand::Image(args) => match args {
                    SetMediaArgs::Set { nearest, path, bg } => {
                        let image = ::image::open(path)?;
                        // re-encode and upload to keyboard
                        let encoded =
                            encode_image(image, bg.0, nearest).ok_or("failed to encode image")?;
                        let len = encoded.len();
                        let total = len / 24;
                        let width = total.to_string().len();
                        keyboard.upload_image(encoded, |i| {
                            print!("\ruploading {len} bytes ({i:width$}/{total}) ... ");
                            stdout().flush().unwrap();
                        })?;
                        Ok(())
                    },
                    SetMediaArgs::Clear => {
                        keyboard.clear_image()?;
                        Ok(())
                    },
                },
                SetCommand::Gif(args) => match args {
                    SetMediaArgs::Set { nearest, path, bg } => {
                        print!("decoding animation ... ");
                        stdout().flush().unwrap();
                        let decoder = image::ImageReader::open(path)?
                            .with_guessed_format()
                            .unwrap();
                        let frames = match decoder.format() {
                            Some(image::ImageFormat::Gif) => {
                                // Reset reader and decode gif as an animation
                                let mut reader = decoder.into_inner();
                                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                                Some(GifDecoder::new(reader)?.into_frames())
                            },
                            Some(image::ImageFormat::Png) => {
                                // Reset reader
                                let mut reader = decoder.into_inner();
                                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                                let decoder = PngDecoder::new(reader)?;
                                // If the png contains an apng, decode as an animation
                                decoder
                                    .is_apng()?
                                    .then_some(decoder.apng().unwrap().into_frames())
                            },
                            Some(image::ImageFormat::WebP) => {
                                // Reset reader
                                let mut reader = decoder.into_inner();
                                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                                let decoder = WebPDecoder::new(reader).unwrap();
                                // If the webp contains an animation, decode as an animation
                                decoder.has_animation().then_some(decoder.into_frames())
                            },
                            _ => None,
                        }
                        .ok_or("failed to decode animation")?;
                        println!("done");

                        // re-encode and upload to keyboard
                        let encoded = encode_gif(frames, bg.0, nearest)
                            .ok_or("failed to encode gif image")?;
                        let len = encoded.len();
                        let total = len / 24;
                        let width = total.to_string().len();
                        keyboard.upload_gif(encoded, |i| {
                            print!("\ruploading {len} bytes ({i:width$}/{total}) ... ");
                            stdout().flush().unwrap();
                        })?;
                        println!("done");
                        Ok(())
                    },
                    SetMediaArgs::Clear => {
                        keyboard.clear_gif()?;
                        Ok(())
                    },
                },
                SetCommand::Clear => {
                    keyboard.clear_image()?;
                    keyboard.clear_gif()?;
                    println!("cleared media");
                    Ok(())
                },
            }
        },
    }
}

#[cfg(test)]
#[test]
fn generate_docs() {
    let app = env!("CARGO_PKG_NAME");
    let options = cli();

    let roff = options.render_manpage(app, bpaf::doc::Section::General, None, None, None);
    std::fs::write("docs/zoom-sync.1", roff).expect("failed to write manpage");

    let md = options.header("").render_markdown(app);
    std::fs::write("docs/README.md", md).expect("failed to write markdown docs");
}
