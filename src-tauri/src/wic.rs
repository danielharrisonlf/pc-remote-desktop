//! Clipboard images (clipboard.v2) decoded and converted in memory using pure Rust (`image`).
//!
//! An image from the phone (PNG or JPEG) becomes `CF_DIBV5` plus the registered "PNG"
//! format; a DIB becomes PNG for the phone. Both directions refuse more than
//! 32 MiB of PNG/JPEG and more than 50 megapixels.
use crate::protocol::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ColorType, ImageEncoder, ImageFormat, ImageReader, Limits, RgbaImage,
};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Not a valid PNG/JPEG/DIB image.
    Invalid,
    /// Above 32 MiB or 50 megapixels.
    TooLarge,
}

impl ImageError {
    /// Error code of the file socket.
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "file_type",
            Self::TooLarge => "too_large",
        }
    }
}

/// What the clipboard holds: the registered "PNG" format or a DIB.
pub enum ClipImage {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const DIBV5_HEADER: usize = 124;
/// Largest DIB read from the clipboard: 50 MP of 32-bit pixels plus headers.
pub const MAX_DIB_BYTES: usize = MAX_IMAGE_PIXELS as usize * 4 + 64 * 1024;

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIGNATURE)
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

pub fn check_pixels(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid);
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS || width > i32::MAX as u32 {
        return Err(ImageError::TooLarge);
    }
    Ok(())
}

fn detect_format(bytes: &[u8]) -> Result<ImageFormat, ImageError> {
    if is_png(bytes) {
        Ok(ImageFormat::Png)
    } else if is_jpeg(bytes) {
        Ok(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"BM") {
        Ok(ImageFormat::Bmp)
    } else {
        Err(ImageError::Invalid)
    }
}

/// Decodes the first frame of an encoded image into top-down BGRA pixels, checking dimensions first.
fn decode_bgra(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), ImageError> {
    let format = detect_format(bytes)?;
    let dim_reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (width, height) = dim_reader
        .into_dimensions()
        .map_err(|_| ImageError::Invalid)?;
    check_pixels(width, height)?;

    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(MAX_DIB_BYTES as u64 * 2);
    reader.limits(limits);

    let dynamic = reader.decode().map_err(|_| ImageError::Invalid)?;
    let rgba: RgbaImage = dynamic.into_rgba8();
    let (w, h) = rgba.dimensions();
    check_pixels(w, h)?;
    let mut bgra = rgba.into_raw();
    rgba_to_bgra_in_place(&mut bgra);
    Ok((w, h, bgra))
}

fn rgba_to_bgra_in_place(buf: &mut [u8]) {
    for pixel in buf.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

/// Encodes top-down BGRA pixels as PNG in memory.
pub fn encode_png_from_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, ImageError> {
    check_pixels(width, height)?;
    let mut rgba = bgra.to_vec();
    rgba_to_bgra_in_place(&mut rgba);
    let mut out = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut out, CompressionType::Default, FilterType::Adaptive);
    encoder
        .write_image(&rgba, width, height, ColorType::Rgba8.into())
        .map_err(|_| ImageError::Invalid)?;
    if out.len() as u64 > MAX_DIB_BYTES as u64 {
        return Err(ImageError::TooLarge);
    }
    Ok(out)
}

/// `BITMAPV5HEADER` for 32-bit BGRA with alpha, bottom-up, sRGB.
fn dibv5_header(width: u32, height: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(DIBV5_HEADER);
    let u32le = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
    u32le(&mut header, DIBV5_HEADER as u32); // bV5Size
    u32le(&mut header, width); // bV5Width
    u32le(&mut header, height); // bV5Height > 0: bottom-up
    header.extend_from_slice(&1u16.to_le_bytes()); // bV5Planes
    header.extend_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    u32le(&mut header, 3); // BI_BITFIELDS
    u32le(&mut header, width * height * 4); // bV5SizeImage
    u32le(&mut header, 0); // bV5XPelsPerMeter
    u32le(&mut header, 0); // bV5YPelsPerMeter
    u32le(&mut header, 0); // bV5ClrUsed
    u32le(&mut header, 0); // bV5ClrImportant
    u32le(&mut header, 0x00FF_0000); // red mask
    u32le(&mut header, 0x0000_FF00); // green mask
    u32le(&mut header, 0x0000_00FF); // blue mask
    u32le(&mut header, 0xFF00_0000); // alpha mask
    u32le(&mut header, 0x7352_4742); // LCS_sRGB
    header.resize(108, 0); // endpoints and gamma
    u32le(&mut header, 4); // LCS_GM_IMAGES
    u32le(&mut header, 0); // bV5ProfileData
    u32le(&mut header, 0); // bV5ProfileSize
    u32le(&mut header, 0); // bV5Reserved
    debug_assert_eq!(header.len(), DIBV5_HEADER);
    header
}

/// Converts top-down BGRA pixels into a bottom-up `CF_DIBV5`.
fn dibv5(bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImageError> {
    let row = width as usize * 4;
    if bgra.len() != row * height as usize {
        return Err(ImageError::Invalid);
    }
    let mut dib = dibv5_header(width, height);
    dib.resize(DIBV5_HEADER + row * height as usize, 0);
    for y in 0..height {
        let target = DIBV5_HEADER + (height - 1 - y) as usize * row;
        let from = y as usize * row;
        dib[target..target + row].copy_from_slice(&bgra[from..from + row]);
    }
    Ok(dib)
}

/// An image from the phone: the PNG for the "PNG" format (the original, or the JPEG
/// converted) and the `CF_DIBV5` built from it.
pub fn clipboard_formats(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ImageError> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    let png = is_png(bytes);
    if !png && !is_jpeg(bytes) {
        return Err(ImageError::Invalid);
    }
    let (width, height, bgra) = decode_bgra(bytes)?;
    let dib = dibv5(&bgra, width, height)?;
    let png = if png {
        bytes.to_vec()
    } else {
        encode_png_from_bgra(width, height, &bgra)?
    };
    Ok((png, dib))
}

/// A BMP file around a packed DIB (`CF_DIB` or `CF_DIBV5`), so a BMP decoder reads it.
pub fn bmp_file(dib: &[u8]) -> Option<Vec<u8>> {
    let u16le = |at: usize| Some(u16::from_le_bytes(dib.get(at..at + 2)?.try_into().ok()?));
    let u32le = |at: usize| Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?));
    let header = u32le(0)? as usize;
    let palette = if header == 12 {
        // BITMAPCOREHEADER: RGBTRIPLE palette entries.
        let bits = u16le(10)?;
        if bits <= 8 {
            3usize << bits
        } else {
            0
        }
    } else if header >= 40 {
        let bits = u16le(14)?;
        let compression = u32le(16)?;
        let used = u32le(32)? as usize;
        let masks = match (header, compression) {
            (40, 3) => 12, // BI_BITFIELDS after a BITMAPINFOHEADER
            (40, 6) => 16, // BI_ALPHABITFIELDS
            _ => 0,
        };
        let colors = if used != 0 {
            used
        } else if bits <= 8 {
            1usize << bits
        } else {
            0
        };
        masks + colors.checked_mul(4)?
    } else {
        return None;
    };
    let offset = 14usize.checked_add(header)?.checked_add(palette)?;
    let size = 14usize.checked_add(dib.len())?;
    if offset > size || size > u32::MAX as usize {
        return None;
    }
    let mut file = Vec::with_capacity(size);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(size as u32).to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&(offset as u32).to_le_bytes());
    file.extend_from_slice(dib);
    Some(file)
}

/// What the PC sends to the phone: the "PNG" format as it is, or the DIB encoded as PNG.
pub fn png_for_phone(image: ClipImage) -> Result<Vec<u8>, ImageError> {
    let png = match image {
        ClipImage::Png(bytes) => {
            if !is_png(&bytes) {
                return Err(ImageError::Invalid);
            }
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                return Err(ImageError::TooLarge);
            }
            decode_bgra(&bytes)?;
            bytes
        }
        ClipImage::Dib(dib) => {
            let bmp = bmp_file(&dib).ok_or(ImageError::Invalid)?;
            let (width, height, bgra) = decode_bgra(&bmp)?;
            encode_png_from_bgra(width, height, &bgra)?
        }
    };
    if png.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    Ok(png)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;

    /// Encodes BGRA pixels (top-down) as PNG or JPEG, as a phone or a browser would.
    pub(crate) fn encode_png(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
        encode_png_from_bgra(width, height, bgra).unwrap()
    }

    pub(crate) fn encode_jpeg(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for p in bgra.chunks_exact(4) {
            rgb.push(p[2]);
            rgb.push(p[1]);
            rgb.push(p[0]);
        }
        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, 90);
        encoder
            .encode(&rgb, width, height, ColorType::Rgb8.into())
            .unwrap();
        out
    }

    /// Top-down BGRA of an encoded image.
    fn pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        decode_bgra(bytes).unwrap()
    }

    /// 3 × 2: red, green, blue / white, half-transparent black, transparent.
    pub(crate) fn sample() -> Vec<u8> {
        vec![
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, //
            255, 255, 255, 255, 0, 0, 0, 128, 0, 0, 0, 0,
        ]
    }

    pub(crate) fn sample_png() -> Vec<u8> {
        encode_png(3, 2, &sample())
    }

    #[test]
    fn png_from_the_phone_keeps_its_bytes_and_becomes_a_bottom_up_dibv5() {
        let png = sample_png();
        assert!(is_png(&png));
        let (kept, dib) = clipboard_formats(&png).unwrap();
        assert_eq!(kept, png);
        assert_eq!(dib.len(), DIBV5_HEADER + 3 * 2 * 4);
        assert_eq!(&dib[0..4], &124u32.to_le_bytes());
        assert_eq!(&dib[4..8], &3u32.to_le_bytes());
        assert_eq!(&dib[8..12], &2u32.to_le_bytes());
        assert_eq!(&dib[14..16], &32u16.to_le_bytes());
        assert_eq!(&dib[52..56], &0xFF00_0000u32.to_le_bytes());
        let rows = &dib[DIBV5_HEADER..];
        // Bottom-up: the second source row comes first.
        assert_eq!(&rows[..12], &sample()[12..]);
        assert_eq!(&rows[12..], &sample()[..12]);
        // Reads the DIB back as the same picture.
        let png_again = png_for_phone(ClipImage::Dib(dib)).unwrap();
        let (width, height, back) = pixels(&png_again);
        assert_eq!((width, height), (3, 2));
        assert_eq!(back, sample());
    }

    #[test]
    fn jpeg_from_the_phone_becomes_png() {
        let opaque: Vec<u8> = [[40u8, 80, 120, 255]; 64 * 32].concat();
        let jpeg = encode_jpeg(64, 32, &opaque);
        assert!(is_jpeg(&jpeg));
        let (png, dib) = clipboard_formats(&jpeg).unwrap();
        assert!(is_png(&png));
        assert_eq!(dib.len(), DIBV5_HEADER + 64 * 32 * 4);
        let (width, height, back) = pixels(&png);
        assert_eq!((width, height), (64, 32));
        // JPEG is lossy; a flat color stays close.
        assert!(back.chunks(4).all(|p| p[0].abs_diff(40) < 8
            && p[1].abs_diff(80) < 8
            && p[2].abs_diff(120) < 8
            && p[3] == 255));
    }

    #[test]
    fn classic_dibs_with_palettes_and_masks_are_read() {
        // 2 × 1, 24-bit BI_RGB, rows padded to 4 bytes.
        let mut dib = Vec::new();
        for value in [40u32, 2, 1] {
            dib.extend_from_slice(&value.to_le_bytes());
        }
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        dib.extend_from_slice(&[255, 0, 0, 0, 0, 255, 0, 0]);
        let bmp = bmp_file(&dib).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        let png = png_for_phone(ClipImage::Dib(dib.clone())).unwrap();
        let (width, height, back) = pixels(&png);
        assert_eq!((width, height), (2, 1));
        assert_eq!(back, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        // 8-bit palettes and BI_BITFIELDS masks move the pixel offset.
        let mut paletted = dib.clone();
        paletted[14..16].copy_from_slice(&8u16.to_le_bytes());
        paletted.extend_from_slice(&[0; 1024]);
        assert_eq!(
            u32::from_le_bytes(bmp_file(&paletted).unwrap()[10..14].try_into().unwrap()),
            54 + 1024
        );
        let mut masked = dib.clone();
        masked[14..16].copy_from_slice(&32u16.to_le_bytes());
        masked[16..20].copy_from_slice(&3u32.to_le_bytes());
        masked.extend_from_slice(&[0; 12]);
        assert_eq!(
            u32::from_le_bytes(bmp_file(&masked).unwrap()[10..14].try_into().unwrap()),
            54 + 12
        );
        // Truncated or unknown headers, and a palette the data does not hold, are refused.
        assert!(bmp_file(&dib[..2]).is_none());
        let mut missing = dib.clone();
        missing[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert!(bmp_file(&missing).is_none());
        assert!(bmp_file(&[20, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn limits_and_garbage() {
        assert_eq!(check_pixels(7071, 7071), Ok(()));
        assert_eq!(check_pixels(10_000, 5_000), Ok(()));
        assert_eq!(check_pixels(7072, 7071), Err(ImageError::TooLarge));
        assert_eq!(check_pixels(0, 10), Err(ImageError::Invalid));
        assert_eq!(
            clipboard_formats(b"GIF89a....").unwrap_err(),
            ImageError::Invalid
        );
        let mut fake = PNG_SIGNATURE.to_vec();
        fake.extend_from_slice(b"not really a png");
        assert_eq!(clipboard_formats(&fake).unwrap_err(), ImageError::Invalid);
        assert_eq!(
            png_for_phone(ClipImage::Png(fake)).unwrap_err(),
            ImageError::Invalid
        );
        let big = vec![0u8; MAX_IMAGE_BYTES as usize + 1];
        assert_eq!(clipboard_formats(&big).unwrap_err(), ImageError::TooLarge);
        // A real PNG whose header claims 10 000 × 10 000 pixels is refused before decoding.
        let huge = resized(&sample_png(), 10_000, 10_000);
        assert_eq!(clipboard_formats(&huge).unwrap_err(), ImageError::TooLarge);
        assert_eq!(
            png_for_phone(ClipImage::Png(huge)).unwrap_err(),
            ImageError::TooLarge
        );
        assert_eq!(ImageError::TooLarge.code(), "too_large");
        assert_eq!(ImageError::Invalid.code(), "file_type");
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// `png` with other IHDR dimensions (and a valid CRC); the pixel data stays as it was.
    fn resized(png: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut out = png.to_vec();
        assert_eq!(&out[12..16], b"IHDR");
        out[16..20].copy_from_slice(&width.to_be_bytes());
        out[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&out[12..29]);
        out[29..33].copy_from_slice(&crc.to_be_bytes());
        out
    }
}
