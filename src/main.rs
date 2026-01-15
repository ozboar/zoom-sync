use std::error::Error;
use std::fmt::{Debug, Display};
use std::io::{stdout, Seek, Write};
use std::path::PathBuf;
use std::str::FromStr;

use bpaf::{Bpaf, Parser};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::AnimationDecoder;
use zoom_sync_core::Board;

use crate::detection::{board_kind, BoardKind};
use crate::info::{apply_system, cpu_mode, gpu_mode, CpuMode, GpuMode};
use crate::media::{encode_gif, encode_image};
use crate::screen::{apply_screen, screen_args, ScreenArgs};
use crate::weather::{apply_weather, weather_args, WeatherArgs};

mod config;
mod detection;
mod info;
mod lock;
mod media;
mod screen;
mod tray;
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
    /// Set screen theme colors (zoom-tkl-dyna only)
    #[bpaf(command)]
    Theme {
        /// Background color (hex: #RRGGBB or #RGB)
        #[bpaf(short, long, fallback(Color([0; 3])), display_fallback)]
        bg: Color,
        /// Font/foreground color (hex: #RRGGBB or #RGB)
        #[bpaf(short('c'), long("color"), fallback(Color([255; 3])), display_fallback)]
        font: Color,
        /// Theme preset ID
        #[bpaf(short, long, fallback(0u8))]
        id: u8,
    },
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
struct Cli {
    #[bpaf(external(board_kind))]
    board: BoardKind,
    #[bpaf(external(command))]
    command: Command,
}

#[derive(Clone, Debug)]
enum Command {
    /// Run with a system tray menu for GUI control (default).
    Tray,
    /// Set specific options on the keyboard.
    /// Must not be used while zoom-sync is already running.
    Set { set_command: SetCommand },
}

fn command() -> impl Parser<Command> {
    let tray = bpaf::pure(Command::Tray)
        .to_options()
        .descr("Run with a system tray menu for GUI control")
        .command("tray")
        .help("Run with a system tray menu for GUI control (default)");

    let set = set_command()
        .map(|set_command| Command::Set { set_command })
        .to_options()
        .descr("Set specific options on the keyboard")
        .command("set")
        .help("Set specific options on the keyboard");

    bpaf::construct!([tray, set]).fallback(Command::Tray)
}

/// Convert RGB888 to RGB565
fn rgb_to_rgb565(rgb: [u8; 3]) -> u16 {
    let r5 = ((rgb[0] >> 3) & 0x1F) as u16;
    let g6 = ((rgb[1] >> 2) & 0x3F) as u16;
    let b5 = ((rgb[2] >> 3) & 0x1F) as u16;
    (r5 << 11) | (g6 << 5) | b5
}

pub fn apply_time(board: &mut dyn Board, _12hr: bool) -> Result<(), Box<dyn Error>> {
    let time = chrono::Local::now();
    board
        .as_time()
        .ok_or("board does not support time")?
        .set_time(time, _12hr)?;
    println!("updated time to {time}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = cli().run();
    match cli.command {
        Command::Tray => {
            let _lock = lock::Lock::acquire()?;
            tray::run_tray_app(cli.board)
        },
        Command::Set { set_command } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let mut board = cli.board.as_board()?;
                match set_command {
                    SetCommand::Time => apply_time(board.as_mut(), false),
                    SetCommand::Weather {
                        farenheit,
                        mut weather_args,
                    } => apply_weather(board.as_mut(), &mut weather_args, farenheit).await,
                    SetCommand::System {
                        farenheit,
                        cpu_mode,
                        gpu_mode,
                        download,
                    } => apply_system(
                        board.as_mut(),
                        farenheit,
                        &mut cpu_mode.either(),
                        &gpu_mode.either(),
                        download,
                    ),
                    SetCommand::Screen(args) => apply_screen(&args, board.as_mut()),
                    SetCommand::Theme { bg, font, id } => {
                        // Convert RGB to RGB565
                        let bg_565 = rgb_to_rgb565(bg.0);
                        let font_565 = rgb_to_rgb565(font.0);
                        board
                            .as_theme()
                            .ok_or("board does not support theme customization")?
                            .set_theme(bg_565, font_565, id)?;
                        println!("set theme: bg={bg}, font={font}, id={id}");
                        Ok(())
                    },
                    SetCommand::Image(args) => match args {
                        SetMediaArgs::Set { nearest, path, bg } => {
                            let (width, height) = board
                                .as_screen_size()
                                .ok_or("board does not support images")?;
                            let image = ::image::open(path)?;
                            // re-encode and upload to keyboard
                            let encoded = encode_image(image, bg.0, nearest, width, height)
                                .ok_or("failed to encode image")?;
                            let len = encoded.len();
                            let total = len / 24;
                            let fmt_width = total.to_string().len();
                            board
                                .as_image()
                                .ok_or("board does not support images")?
                                .upload_image(&encoded, &mut |i| {
                                    print!("\ruploading {len} bytes ({i:fmt_width$}/{total}) ... ");
                                    stdout().flush().unwrap();
                                })?;
                            Ok(())
                        },
                        SetMediaArgs::Clear => {
                            board
                                .as_image()
                                .ok_or("board does not support images")?
                                .clear_image()?;
                            Ok(())
                        },
                    },
                    SetCommand::Gif(args) => match args {
                        SetMediaArgs::Set { nearest, path, bg } => {
                            let (width, height) = board
                                .as_screen_size()
                                .ok_or("board does not support gifs")?;
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
                            let encoded = encode_gif(frames, bg.0, nearest, width, height)
                                .ok_or("failed to encode gif image")?;
                            let len = encoded.len();
                            let total = len / 24;
                            let fmt_width = total.to_string().len();
                            board
                                .as_gif()
                                .ok_or("board does not support gifs")?
                                .upload_gif(&encoded, &mut |i| {
                                    print!("\ruploading {len} bytes ({i:fmt_width$}/{total}) ... ");
                                    stdout().flush().unwrap();
                                })?;
                            println!("done");
                            Ok(())
                        },
                        SetMediaArgs::Clear => {
                            board
                                .as_gif()
                                .ok_or("board does not support gifs")?
                                .clear_gif()?;
                            Ok(())
                        },
                    },
                    SetCommand::Clear => {
                        if let Some(img) = board.as_image() {
                            img.clear_image()?;
                        }
                        if let Some(gif) = board.as_gif() {
                            gif.clear_gif()?;
                        }
                        println!("cleared media");
                        Ok(())
                    },
                }
            })
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
