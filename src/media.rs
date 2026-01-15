use std::cmp::max;
use std::io::{stdout, Write};
use std::sync::atomic::AtomicU16;

use image::imageops::FilterType;
use image::{imageops, DynamicImage, Frames, GenericImageView, ImageBuffer, Pixel};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

/// Encode an square image as rgb565 with an 8 bit alpha channel
pub fn encode_image(
    image: DynamicImage,
    background: [u8; 3],
    nearest: bool,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    print!("resizing and encoding image ... ");
    stdout().flush().unwrap();
    let [br, bg, bb] = background;

    let buf = image
        .resize_to_fill(
            width,
            height,
            if nearest {
                FilterType::Nearest
            } else {
                FilterType::Gaussian
            },
        )
        .to_rgba8()
        .pixels()
        .flat_map(|p| {
            let [mut r, mut g, mut b, a] = p.0;

            // Mix alpha values against black
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
        .collect::<Vec<_>>();
    debug_assert_eq!(buf.len(), (width * height * 3) as usize);

    println!("done");
    Some(buf)
}

/// Re-encode animation frames as a gif
pub fn encode_gif(
    frames: Frames,
    background: [u8; 3],
    nearest: bool,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let frames = frames.collect_frames().ok()?;
    let len = frames.len();
    let [br, bg, bb] = background;
    // GIF dimensions need to be +1 for some reason with zoom65v3
    let gif_width = width + 1;
    let gif_height = height + 1;

    let completed = AtomicU16::new(1);
    let new_frames = frames
        .par_iter()
        .map(|frame| {
            let resized = resize_to_fill(frame.buffer(), gif_width, gif_height, nearest);
            let mut buf = image::ImageBuffer::from_fn(gif_width, gif_height, |_, _| {
                [br, bg, bb, 0xff].into()
            });
            imageops::overlay(&mut buf, &resized, 0, 0);

            let mut frame =
                gif::Frame::from_rgba(gif_width as u16, gif_height as u16, &mut buf.into_vec());
            frame.make_lzw_pre_encoded();
            frame.needs_user_input = true;
            let i = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            print!("\rre-encoding frames ({i}/{len}) ... ");
            stdout().flush().unwrap();
            frame
        })
        .collect::<Vec<_>>();

    let mut buf = Vec::new();
    {
        let mut encoder =
            gif::Encoder::new(&mut buf, gif_width as u16, gif_height as u16, &[]).ok()?;
        encoder.set_repeat(gif::Repeat::Infinite).ok()?;
        for frame in new_frames {
            encoder.write_lzw_pre_encoded_frame(&frame).ok()?;
        }
    }
    println!("done");
    Some(buf)
}

pub fn resize_to_fill<I: GenericImageView>(
    image: &I,
    nwidth: u32,
    nheight: u32,
    nearest: bool,
) -> ImageBuffer<I::Pixel, Vec<<I::Pixel as Pixel>::Subpixel>>
where
    I::Pixel: 'static,
    <I::Pixel as Pixel>::Subpixel: 'static,
{
    let (width2, height2) = resize_dimensions(image.width(), image.height(), nwidth, nheight, true);

    let mut intermediate = imageops::resize(
        image,
        width2,
        height2,
        if nearest {
            FilterType::Nearest
        } else {
            FilterType::Gaussian
        },
    );

    let (iwidth, iheight) = intermediate.dimensions();
    let ratio = u64::from(iwidth) * u64::from(nheight);
    let nratio = u64::from(nwidth) * u64::from(iheight);

    if nratio > ratio {
        imageops::crop(
            &mut intermediate,
            0,
            (iheight - nheight) / 2,
            nwidth,
            nheight,
        )
        .to_image()
    } else {
        imageops::crop(&mut intermediate, (iwidth - nwidth) / 2, 0, nwidth, nheight).to_image()
    }
}

/// https://docs.rs/image/0.25.5/src/image/math/utils.rs.html#12
pub fn resize_dimensions(
    width: u32,
    height: u32,
    nwidth: u32,
    nheight: u32,
    fill: bool,
) -> (u32, u32) {
    let wratio = f64::from(nwidth) / f64::from(width);
    let hratio = f64::from(nheight) / f64::from(height);

    let ratio = if fill {
        f64::max(wratio, hratio)
    } else {
        f64::min(wratio, hratio)
    };

    let nw = max((f64::from(width) * ratio).round() as u64, 1);
    let nh = max((f64::from(height) * ratio).round() as u64, 1);

    if nw > u64::from(u32::MAX) {
        let ratio = f64::from(u32::MAX) / f64::from(width);
        (u32::MAX, max((f64::from(height) * ratio).round() as u32, 1))
    } else if nh > u64::from(u32::MAX) {
        let ratio = f64::from(u32::MAX) / f64::from(height);
        (max((f64::from(width) * ratio).round() as u32, 1), u32::MAX)
    } else {
        (nw as u32, nh as u32)
    }
}
