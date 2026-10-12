//! Reading PNG and JPEG files as the float RGB images the engine works on.
//!
//! [`decode`] runs these steps in this order: guess the format from the bytes and refuse anything
//! but PNG and JPEG, decode, apply the EXIF orientation (so a phone photo is upright, not
//! sideways), composite any transparency over white (the colour of blank paper, instead of
//! whatever colour hides under transparent pixels), shrink with Lanczos resampling so that the
//! longer edge is [`MAX_EDGE_PX`] if it was longer, then convert to floats clamped to 0..1. A
//! smaller image is not enlarged. 8-bit samples are scaled by 255 and 16-bit samples by 65535, so
//! a 16-bit greyscale scan is scaled rather than clipped; greyscale is repeated into three
//! channels.
//!
//! An image with alpha is composited to f32 before the resize, so while it shrinks it takes 12
//! bytes per pixel at full size, and the composite briefly holds the decoded pixels beside that.
//! An image without alpha shrinks at its stored depth and becomes f32 only at its final size. The
//! `image` crate's default memory limit covers decoding alone: a file that needs more than that to
//! decode is a decode error, and the later steps are not limited.
//!
//! Compositing is the "over" operator of T. Porter and T. Duff, "Compositing digital images",
//! *Computer Graphics* 18(3) (SIGGRAPH '84), 1984, pp. 253-259. Lanczos resampling is described in
//! C. E. Duchon, "Lanczos filtering in one and two dimensions", *Journal of Applied Meteorology*
//! 18(8), 1979, pp. 1016-1022.

use std::io::Cursor;

use ::image::imageops::FilterType;
use ::image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgb32FImage};
use thiserror::Error;

use crate::types::Image;

/// The longest edge an image keeps, in pixels; a longer one is shrunk to this.
pub const MAX_EDGE_PX: u32 = 4000;

/// Why an image could not be read; the message is shown to users.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ImageError {
    /// The bytes are not a PNG or JPEG file; the message says what they are.
    #[error("unsupported image: {0}; supported formats are PNG and JPEG")]
    Unsupported(String),
    /// A PNG or JPEG file the decoder rejected; the message is its reason.
    #[error("could not decode the image: {0}")]
    Decode(String),
    /// Float pixels whose count does not match the image size.
    #[error("expected {expected} values for an RGB image, got {got}")]
    Shape {
        /// `width * height * 3`.
        expected: usize,
        /// The number of values given.
        got: usize,
    },
}

fn decode_error(e: impl std::fmt::Display) -> ImageError {
    ImageError::Decode(e.to_string())
}

/// Decode a PNG or JPEG file into an upright, opaque RGB image at most `MAX_EDGE_PX` on its
/// longer edge, following the steps in the module doc.
pub fn decode(bytes: &[u8]) -> Result<Image, ImageError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(decode_error)?;
    match reader.format() {
        Some(ImageFormat::Png | ImageFormat::Jpeg) => {}
        Some(other) => return Err(ImageError::Unsupported(format!("{other:?}"))),
        None => return Err(ImageError::Unsupported("unknown format".to_string())),
    }
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    img.apply_orientation(orientation);
    let mut img = over_white(img);
    let long = img.width().max(img.height());
    if long > MAX_EDGE_PX {
        let fit = |edge: u32| {
            let scaled = f64::from(edge) * f64::from(MAX_EDGE_PX) / f64::from(long);
            (scaled.round() as u32).max(1)
        };
        img = img.resize_exact(fit(img.width()), fit(img.height()), FilterType::Lanczos3);
    }
    let (width, height) = (img.width(), img.height());
    // Scales u8 samples by 255 and u16 samples by 65535 and repeats grey into three channels. The
    // grey paths multiply by the reciprocal, so they can differ from a division by 1 ulp.
    let mut rgb = img.into_rgb32f().into_raw();
    for v in &mut rgb {
        *v = v.clamp(0.0, 1.0); // Lanczos can overshoot
    }
    Ok(Image { width, height, rgb })
}

/// Composite any transparency over white; an image without alpha comes back unchanged.
///
/// 8-bit samples use `(a * c / 255 + (255 - a)) / 255` in f32; wider samples are scaled to 0..1
/// first and use `c * a + (1 - a)`. The source buffer is consumed, not copied.
fn over_white(img: DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return img;
    }
    let (w, h) = (img.width(), img.height());
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    let over8 = |c: u8, a: u8| {
        let a = f32::from(a);
        (a * f32::from(c) / 255.0 + (255.0 - a)) / 255.0
    };
    let over = |c: f32, a: f32| c * a + (1.0 - a);
    let unit16 = |v: u16| f32::from(v) / 65535.0;
    match img {
        DynamicImage::ImageRgba8(buf) => rgb.extend(
            buf.pixels()
                .flat_map(|p| [p[0], p[1], p[2]].map(|c| over8(c, p[3]))),
        ),
        DynamicImage::ImageLumaA8(buf) => {
            rgb.extend(buf.pixels().flat_map(|p| [over8(p[0], p[1]); 3]))
        }
        DynamicImage::ImageRgba16(buf) => rgb.extend(
            buf.pixels()
                .flat_map(|p| [p[0], p[1], p[2]].map(|c| over(unit16(c), unit16(p[3])))),
        ),
        DynamicImage::ImageLumaA16(buf) => rgb.extend(
            buf.pixels()
                .flat_map(|p| [over(unit16(p[0]), unit16(p[1])); 3]),
        ),
        // Rgba32F, which neither decoder produces, and any alpha layout added later.
        other => rgb.extend(
            other
                .into_rgba32f()
                .pixels()
                .flat_map(|p| [p[0], p[1], p[2]].map(|c| over(c, p[3]))),
        ),
    }
    DynamicImage::ImageRgb32F(Rgb32FImage::from_raw(w, h, rgb).expect("three floats per pixel"))
}

/// Wrap float RGB pixels, row-major with three per pixel, as an image of the given size.
pub fn from_rgb_f32(width: u32, height: u32, rgb: Vec<f32>) -> Result<Image, ImageError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3));
    if expected != Some(rgb.len()) {
        return Err(ImageError::Shape {
            expected: expected.unwrap_or(usize::MAX),
            got: rgb.len(),
        });
    }
    Ok(Image { width, height, rgb })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::codecs::jpeg::JpegEncoder;
    use ::image::codecs::png::PngEncoder;
    use ::image::{ExtendedColorType, ImageEncoder};

    fn encode_png(w: u32, h: u32, data: &[u8], color: ExtendedColorType) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(data, w, h, color)
            .unwrap();
        out
    }

    fn encode_png_rgb8(w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        encode_png(w, h, data, ExtendedColorType::Rgb8)
    }

    fn encode_png_rgba8(w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        encode_png(w, h, data, ExtendedColorType::Rgba8)
    }

    fn encode_png_luma16(w: u32, h: u32, data: &[u16]) -> Vec<u8> {
        // The encoder takes 16-bit samples in native byte order.
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        encode_png(w, h, &bytes, ExtendedColorType::L16)
    }

    fn encode_jpeg_rgb8(w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 95)
            .write_image(data, w, h, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    /// A `w` x `h` JPEG whose EXIF block sets Orientation (tag 0x0112) to 6, rotate 90 clockwise.
    fn jpeg_with_orientation_6(w: u32, h: u32) -> Vec<u8> {
        let jpeg = encode_jpeg_rgb8(w, h, &vec![128; (w * h * 3) as usize]);
        assert_eq!(jpeg[..2], [0xFF, 0xD8], "a JPEG starts with the SOI marker");
        let app1: &[u8] = &[
            0xFF, 0xE1, // APP1 marker
            0x00, 0x22, // segment length, big-endian: 34 bytes counting these two
            b'E', b'x', b'i', b'f', 0x00, 0x00, // EXIF identifier
            b'M', b'M', 0x00, 0x2A, // TIFF header: big-endian byte order, magic 42
            0x00, 0x00, 0x00, 0x08, // offset of the first IFD from the TIFF header
            0x00, 0x01, // the IFD holds one entry
            0x01, 0x12, // tag 0x0112, Orientation
            0x00, 0x03, // type 3, SHORT
            0x00, 0x00, 0x00, 0x01, // one value
            0x00, 0x06, 0x00, 0x00, // value 6, left-justified in the 4-byte field
            0x00, 0x00, 0x00, 0x00, // no next IFD
        ];
        assert_eq!(app1.len(), 2 + 0x22);
        // The segment goes right after SOI, before the encoder's own segments.
        [&jpeg[..2], app1, &jpeg[2..]].concat()
    }

    #[test]
    fn png_rgb_decodes_to_floats() {
        let bytes = encode_png_rgb8(2, 1, &[255, 0, 0, 0, 0, 255]);
        let im = decode(&bytes).unwrap();
        assert_eq!((im.width, im.height), (2, 1));
        assert_eq!(im.rgb, vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn transparency_becomes_white() {
        let bytes = encode_png_rgba8(1, 1, &[0, 0, 0, 0]);
        assert_eq!(decode(&bytes).unwrap().rgb, vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn sixteen_bit_grey_scales_by_65535() {
        let bytes = encode_png_luma16(1, 1, &[32768]);
        let v = decode(&bytes).unwrap().rgb[0];
        assert!((v - 32768.0 / 65535.0).abs() < 1e-6);
    }

    #[test]
    fn exif_orientation_is_applied() {
        // A 2 x 1 JPEG with orientation 6 (rotate 90 clockwise) decodes as 1 x 2.
        let bytes = jpeg_with_orientation_6(2, 1);
        let im = decode(&bytes).unwrap();
        assert_eq!((im.width, im.height), (1, 2));
    }

    #[test]
    fn large_images_are_downsized_to_max_edge() {
        let bytes = encode_png_rgb8(4500, 10, &vec![128; 4500 * 10 * 3]);
        let im = decode(&bytes).unwrap();
        assert_eq!((im.width, im.height), (4000, 9));
    }

    #[test]
    fn unsupported_bytes_name_the_supported_formats() {
        let e = decode(b"not an image").unwrap_err().to_string();
        assert!(e.contains("PNG") && e.contains("JPEG"), "{e}");
    }

    #[test]
    fn jpeg_decodes_within_its_lossiness() {
        // JPEG is lossy, so a solid red image only has to come back red within 0.05 per channel.
        let bytes = encode_jpeg_rgb8(8, 8, &[255, 0, 0].repeat(64));
        let im = decode(&bytes).unwrap();
        assert_eq!((im.width, im.height), (8, 8));
        for px in im.rgb.chunks(3) {
            assert!(
                (px[0] - 1.0).abs() < 0.05 && px[1] < 0.05 && px[2] < 0.05,
                "{px:?}"
            );
        }
    }

    #[test]
    fn transparency_is_flattened_before_downsizing() {
        // Opaque white beside transparent black: shrinking before compositing would blend the
        // hidden black in and leave grey, compositing first leaves white.
        let px = [[255, 255, 255, 255], [0, 0, 0, 0]];
        let data: Vec<u8> = (0..4500).flat_map(|i| px[i % 2]).collect();
        let im = decode(&encode_png_rgba8(4500, 1, &data)).unwrap();
        assert_eq!((im.width, im.height), (4000, 1));
        assert!(im.rgb.iter().all(|&v| v > 0.999), "a pixel is not white");
    }

    #[test]
    fn sixteen_bit_alpha_is_composited_over_white() {
        // Half alpha, RGBA then grey: each channel is c * a + (1 - a), c and a scaled by 65535.
        let px: [u16; 4] = [60000, 0, 20000, 32768];
        let bytes: Vec<u8> = px.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let im = decode(&encode_png(1, 1, &bytes, ExtendedColorType::Rgba16)).unwrap();
        let a = 32768.0 / 65535.0;
        for (got, c) in im.rgb.iter().zip(&px[..3]) {
            let want = f32::from(*c) / 65535.0 * a + (1.0 - a);
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
        let grey: Vec<u8> = [20000_u16, 32768]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let im = decode(&encode_png(1, 1, &grey, ExtendedColorType::La16)).unwrap();
        let want = 20000.0 / 65535.0 * a + (1.0 - a);
        assert!(
            im.rgb.iter().all(|v| (v - want).abs() < 1e-6),
            "{:?}",
            im.rgb
        );
    }

    #[test]
    fn grey_with_alpha_is_composited_over_white() {
        // Grey 128 at alpha 128: (128 * 128 / 255 + 127) / 255 in every channel.
        let im = decode(&encode_png(1, 1, &[128, 128], ExtendedColorType::La8)).unwrap();
        let want = (128.0 * 128.0 / 255.0 + 127.0) / 255.0;
        assert!(
            im.rgb.iter().all(|v| (v - want).abs() < 1e-6),
            "{:?}",
            im.rgb
        );
        assert_eq!(im.rgb.len(), 3);
    }

    #[test]
    fn errors_say_what_failed() {
        let mut png = encode_png_rgb8(2, 1, &[0; 6]);
        png.truncate(png.len() / 2);
        let e = decode(&png).unwrap_err();
        assert!(matches!(e, ImageError::Decode(_)), "{e}");
        assert_eq!(
            from_rgb_f32(2, 1, vec![0.0; 5]).unwrap_err().to_string(),
            "expected 6 values for an RGB image, got 5"
        );
        let im = from_rgb_f32(1, 1, vec![0.25, 0.5, 0.75]).unwrap();
        assert_eq!((im.width, im.height, im.rgb), (1, 1, vec![0.25, 0.5, 0.75]));
    }
}
