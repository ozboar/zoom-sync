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
use zoom_sync_core::{Board, Capabilities};

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

/// Pre-parse board kind from args before full parsing
fn pre_parse_board() -> BoardKind {
    let args: Vec<String> = std::env::args().collect();
    for (i, arg) in args.iter().enumerate() {
        if arg == "-b" || arg == "--board" {
            if let Some(board_str) = args.get(i + 1) {
                if let Ok(kind) = board_str.parse() {
                    return kind;
                }
            }
        } else if let Some(board_str) = arg.strip_prefix("--board=") {
            if let Ok(kind) = board_str.parse() {
                return kind;
            }
        }
    }
    BoardKind::Auto
}

/// Set commands - variants are dynamically included based on board capabilities
#[derive(Clone, Debug)]
enum SetCommand {
    Time,
    Weather {
        farenheit: bool,
        weather_args: WeatherArgs,
    },
    System {
        farenheit: bool,
        cpu_mode: CpuMode,
        gpu_mode: GpuMode,
        download: Option<f32>,
    },
    Screen(ScreenArgs),
    Theme {
        bg: Color,
        font: Color,
        id: u8,
    },
    Image(SetMediaArgs),
    Gif(SetMediaArgs),
    Clear,
}

/// Build set_command parser dynamically based on capabilities
fn set_command_for(caps: &Capabilities) -> impl Parser<SetCommand> {
    let mut commands: Vec<Box<dyn Parser<SetCommand>>> = Vec::new();

    if caps.time {
        let time = bpaf::pure(SetCommand::Time)
            .to_options()
            .descr("Sync time to system clock")
            .command("time")
            .help("Sync time to system clock");
        commands.push(Box::new(time));
    }

    if caps.weather {
        let weather = bpaf::construct!(SetCommand::Weather {
            farenheit(),
            weather_args()
        })
        .to_options()
        .descr("Set weather data")
        .command("weather")
        .help("Set weather data");
        commands.push(Box::new(weather));
    }

    if caps.system_info {
        let download = bpaf::short('d')
            .long("download")
            .help("Manually set download speed")
            .argument::<f32>("SPEED")
            .optional();
        let system = bpaf::construct!(SetCommand::System {
            farenheit(),
            cpu_mode(),
            gpu_mode(),
            download
        })
        .to_options()
        .descr("Set system info")
        .command("system")
        .help("Set system info");
        commands.push(Box::new(system));
    }

    if caps.screen_pos || caps.screen_nav {
        let screen = screen_args()
            .map(SetCommand::Screen)
            .to_options()
            .descr("Change current screen")
            .command("screen")
            .help("Change current screen");
        commands.push(Box::new(screen));
    }

    if caps.theme {
        let bg = bpaf::short('b')
            .long("bg")
            .help("Background color (hex: #RRGGBB or #RGB)")
            .argument::<Color>("COLOR")
            .fallback(Color([0; 3]))
            .display_fallback();
        let font = bpaf::short('c')
            .long("color")
            .help("Font/foreground color (hex: #RRGGBB or #RGB)")
            .argument::<Color>("COLOR")
            .fallback(Color([255; 3]))
            .display_fallback();
        let id = bpaf::short('i')
            .long("id")
            .help("Theme preset ID")
            .argument::<u8>("ID")
            .fallback(0u8);
        let theme = bpaf::construct!(SetCommand::Theme { bg, font, id })
            .to_options()
            .descr("Set screen theme colors")
            .command("theme")
            .help("Set screen theme colors");
        commands.push(Box::new(theme));
    }

    if caps.image {
        let image = set_media_args()
            .map(SetCommand::Image)
            .to_options()
            .descr("Upload static image")
            .command("image")
            .help("Upload static image");
        commands.push(Box::new(image));
    }

    if caps.gif {
        let gif = set_media_args()
            .map(SetCommand::Gif)
            .to_options()
            .descr("Upload animated image (gif/webp/apng)")
            .command("gif")
            .help("Upload animated image (gif/webp/apng)");
        commands.push(Box::new(gif));
    }

    // Clear is always available if we have any media capability
    if caps.image || caps.gif {
        let clear = bpaf::pure(SetCommand::Clear)
            .to_options()
            .descr("Clear all media files")
            .command("clear")
            .help("Clear all media files");
        commands.push(Box::new(clear));
    }

    // Combine all commands
    bpaf::choice(commands)
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

#[derive(Clone, Debug)]
struct Cli {
    board: BoardKind,
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

fn command_for(caps: &Capabilities, board_note: &str) -> impl Parser<Command> {
    let tray = bpaf::pure(Command::Tray)
        .to_options()
        .descr("Run with a system tray menu for GUI control")
        .command("tray")
        .help("Run with a system tray menu for GUI control (default)");

    let set = set_command_for(caps)
        .map(|set_command| Command::Set { set_command })
        .to_options()
        .descr("Set specific options on the keyboard")
        .header(board_note)
        .command("set")
        .help("Set specific options on the keyboard");

    bpaf::construct!([tray, set]).fallback(Command::Tray)
}

/// Build the CLI parser dynamically based on detected board capabilities
fn cli_for(caps: &Capabilities, board_note: &str) -> impl Parser<Cli> {
    let board = board_kind();
    let command = command_for(caps, board_note);
    bpaf::construct!(Cli { board, command })
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
    // Pre-parse board to determine capabilities for dynamic CLI building
    let pre_board = pre_parse_board();
    let caps = pre_board.capabilities();

    // Build note for set subcommand with board info
    let board_note = match pre_board {
        BoardKind::Auto => {
            if let Some(detected) = BoardKind::detect() {
                format!(
                    "Note: Showing available commands for your {}",
                    detected.info().map(|i| i.name).unwrap_or("board")
                )
            } else {
                "Note: No board detected - showing all commands".to_string()
            }
        },
        _ => format!(
            "Note: Showing available commands for your {}",
            pre_board.info().map(|i| i.name).unwrap_or("board")
        ),
    };

    // Build and run CLI with board-specific commands
    let cli = cli_for(&caps, &board_note)
        .to_options()
        .version(env!("CARGO_PKG_VERSION"))
        .descr(env!("CARGO_PKG_DESCRIPTION"))
        .run();

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

                            // re-encode to resized standard gif and upload
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
    // Use full capabilities for docs to show all commands
    let all_caps = Capabilities {
        time: true,
        weather: true,
        system_info: true,
        screen_pos: true,
        screen_nav: true,
        image: true,
        gif: true,
        theme: true,
    };
    let options = cli_for(&all_caps, "")
        .to_options()
        .version(env!("CARGO_PKG_VERSION"))
        .descr(env!("CARGO_PKG_DESCRIPTION"));

    let roff = options.render_manpage(app, bpaf::doc::Section::General, None, None, None);
    std::fs::write("docs/zoom-sync.1", roff).expect("failed to write manpage");

    let md = options.header("").render_markdown(app);
    std::fs::write("docs/README.md", md).expect("failed to write markdown docs");
}
