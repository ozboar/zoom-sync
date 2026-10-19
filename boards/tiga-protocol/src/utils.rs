//! Media encoding for Tiga-based keyboards.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::{imageops, AnimationDecoder};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::types::Rgb565;
use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Size of the header prepended to uploaded media
pub const MEDIA_HEADER_LEN: usize = 452;
/// Size of a single little-endian RGB565 screen frame
pub const FRAME_LEN: usize = (SCREEN_WIDTH * SCREEN_HEIGHT * 2) as usize;

/// Media upload header. Contents are static regardless of frame count.
pub fn media_header() -> [u8; MEDIA_HEADER_LEN] {
    let mut header = [0u8; MEDIA_HEADER_LEN];
    header[360..364].copy_from_slice(&(MEDIA_HEADER_LEN as u32).to_le_bytes());
    header[400] = 1;
    header[403] = 1;
    header
}

/// Convert a screen sized RGBA frame into little-endian RGB565, ignoring alpha.
pub fn encode_frame(frame: &image::RgbaImage) -> Vec<u8> {
    frame
        .pixels()
        .flat_map(|p| {
            let [r, g, b, _] = p.0;
            Rgb565::from_rgb(r, g, b).to_le_bytes()
        })
        .collect()
}

/// Convert a screen sized image of big-endian RGB565 plus an alpha byte per pixel into a
/// little-endian RGB565 frame.
pub fn frame_from_rgb565a(data: &[u8]) -> Option<Vec<u8>> {
    (data.len() == (SCREEN_WIDTH * SCREEN_HEIGHT * 3) as usize)
        .then(|| data.chunks_exact(3).flat_map(|p| [p[1], p[0]]).collect())
}

/// Re-encode a GIF as concatenated screen sized RGB565 little-endian frames.
///
/// Frames larger than the screen are cropped from the top left corner, smaller frames are
/// resized to fit. Frame delays are not supported by the device.
pub fn encode_gif(gif_data: &[u8]) -> Option<Vec<u8>> {
    let frames = GifDecoder::new(Cursor::new(gif_data))
        .ok()?
        .into_frames()
        .collect_frames()
        .ok()?;
    let encoded: Vec<Vec<u8>> = frames
        .par_iter()
        .map(|frame| {
            let buf = frame.buffer();
            let frame = if buf.width() >= SCREEN_WIDTH && buf.height() >= SCREEN_HEIGHT {
                imageops::crop_imm(buf, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT).to_image()
            } else {
                imageops::resize(
                    buf,
                    SCREEN_WIDTH,
                    SCREEN_HEIGHT,
                    imageops::FilterType::Gaussian,
                )
            };
            encode_frame(&frame)
        })
        .collect();
    Some(encoded.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_header_matches_reference() {
        let header = media_header();
        assert_eq!(&header[360..364], &[196, 1, 0, 0]);
        assert_eq!(header[400], 1);
        assert_eq!(header[403], 1);
        assert_eq!(header.iter().filter(|&&b| b != 0).count(), 4);
    }

    #[test]
    fn frame_is_little_endian() {
        let frame = image::RgbaImage::from_pixel(1, 1, [255, 0, 0, 255].into());
        assert_eq!(encode_frame(&frame), vec![0x00, 0xF8]);
    }

    #[test]
    fn rgb565a_is_swapped_to_little_endian() {
        let data: Vec<u8> = [0xF8, 0x00, 0xFF].repeat((SCREEN_WIDTH * SCREEN_HEIGHT) as usize);
        let frame = frame_from_rgb565a(&data).unwrap();
        assert_eq!(frame.len(), FRAME_LEN);
        assert_eq!(&frame[..2], &[0x00, 0xF8]);
        assert!(frame_from_rgb565a(&data[3..]).is_none());
    }
}
