//! Utility functions for Tiga-based keyboards.

use std::io::Cursor;
use std::sync::atomic::AtomicU16;

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Encode a raw GIF buffer as RGB565 with frame delays for Tiga-based keyboards.
///
/// Output format:
/// - 2 bytes: frame count (u16 BE)
/// - 2 bytes per frame: delay in centiseconds (u16 BE)
/// - Then: RGB565+alpha data for all frames (concatenated)
///
/// The callback receives (current_frame, total_frames) for progress updates.
pub fn encode_gif(
    gif_data: &[u8],
    background: [u8; 3],
    nearest: bool,
    cb: impl Fn(usize, usize) + Sync,
) -> Option<Vec<u8>> {
    let decoder = GifDecoder::new(Cursor::new(gif_data)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    let frame_count = frames.len();
    let [br, bg, bb] = background;

    let filter = if nearest {
        image::imageops::FilterType::Nearest
    } else {
        image::imageops::FilterType::Gaussian
    };

    // Extract delays (in centiseconds) and encode frames
    let completed = AtomicU16::new(1);
    let encoded_frames: Vec<(u16, Vec<u8>)> = frames
        .par_iter()
        .map(|frame| {
            // Get delay in centiseconds
            let delay = frame.delay();
            let (numer, denom) = delay.numer_denom_ms();
            let delay_cs = ((numer / denom) / 10) as u16;

            // Resize and encode frame as RGB565
            let resized =
                image::imageops::resize(frame.buffer(), SCREEN_WIDTH, SCREEN_HEIGHT, filter);
            let buf: Vec<u8> = resized
                .pixels()
                .flat_map(|p| {
                    let [mut r, mut g, mut b, a] = p.0;

                    // Mix alpha values against background
                    let a = a as f64 / 255.0;
                    let ba = 1. - a;
                    r = ((br as f64 * ba) + (r as f64 * a)) as u8;
                    g = ((bg as f64 * ba) + (g as f64 * a)) as u8;
                    b = ((bb as f64 * ba) + (b as f64 * a)) as u8;

                    // Convert into rgb565 pixel type
                    let [x, y] = rgb565::Rgb565::from_rgb888_components(r, g, b).to_rgb565_be();

                    // Extend with hard coded alpha channel
                    [x, y, 0xff]
                })
                .collect();

            let i = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            cb(i as usize, frame_count);

            (delay_cs, buf)
        })
        .collect();

    // Build output: header + delays + frame data
    let mut output = Vec::new();

    // Frame count (2 bytes, BE)
    output.extend_from_slice(&(frame_count as u16).to_be_bytes());

    // Frame delays (2 bytes each, BE)
    for (delay, _) in &encoded_frames {
        output.extend_from_slice(&delay.to_be_bytes());
    }

    // Frame data (concatenated)
    for (_, data) in encoded_frames {
        output.extend(data);
    }

    Some(output)
}
