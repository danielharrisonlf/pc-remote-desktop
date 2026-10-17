//! Clipboard images (clipboard.v2) through the Windows Imaging Component, in memory only.
//!
//! An image from the phone (PNG or JPEG) becomes `CF_DIBV5` plus the registered "PNG"
//! format; a Windows DIB becomes PNG for the phone. Both directions refuse more than
//! 32 MiB of PNG/JPEG and more than 50 megapixels.
use crate::protocol::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS};
use windows::{
    core::{Interface, GUID},
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
            IWICBitmapSource, IWICImagingFactory, IWICPalette, WICBitmapDitherTypeNone,
            WICBitmapEncoderNoCache, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
            WICRect,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IStream, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED, STREAM_SEEK_END, STREAM_SEEK_SET,
        },
        UI::Shell::SHCreateMemStream,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG/JPEG/DIB that WIC can read.
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

/// What the Windows clipboard holds: the registered "PNG" format or a DIB.
pub enum ClipImage {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const DIBV5_HEADER: usize = 124;
/// Largest DIB read from the clipboard: 50 MP of 32-bit pixels plus headers.
pub const MAX_DIB_BYTES: usize = MAX_IMAGE_PIXELS as usize * 4 + 64 * 1024;
/// Rows copied per WIC call; keeps decoding sequential without a second full buffer.
const STRIP_ROWS: u32 = 64;

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

/// COM for the current thread, released only when this call initialized it.
struct Com(bool);

impl Com {
    fn init() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn invalid<T>(_: T) -> ImageError {
    ImageError::Invalid
}

fn factory() -> Result<IWICImagingFactory, ImageError> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
        .map_err(invalid)
}

/// The first frame of an encoded image, with its size already checked.
fn decode(
    factory: &IWICImagingFactory,
    bytes: &[u8],
) -> Result<(IWICBitmapSource, u32, u32), ImageError> {
    let stream = unsafe { SHCreateMemStream(Some(bytes)) }.ok_or(ImageError::Invalid)?;
    let decoder = unsafe {
        factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
    }
    .map_err(invalid)?;
    let frame = unsafe { decoder.GetFrame(0) }.map_err(invalid)?;
    let source: IWICBitmapSource = frame.cast().map_err(invalid)?;
    let (mut width, mut height) = (0u32, 0u32);
    unsafe { source.GetSize(&mut width, &mut height) }.map_err(invalid)?;
    check_pixels(width, height)?;
    Ok((source, width, height))
}

fn convert(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    format: &GUID,
) -> Result<IWICBitmapSource, ImageError> {
    let converter = unsafe { factory.CreateFormatConverter() }.map_err(invalid)?;
    unsafe {
        converter.Initialize(
            source,
            format,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }
    .map_err(invalid)?;
    converter.cast().map_err(invalid)
}

fn read_stream(stream: &IStream) -> Result<Vec<u8>, ImageError> {
    let mut end = 0u64;
    unsafe { stream.Seek(0, STREAM_SEEK_END, Some(&mut end)) }.map_err(invalid)?;
    if end > MAX_DIB_BYTES as u64 {
        return Err(ImageError::TooLarge);
    }
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(invalid)?;
    let mut out = vec![0u8; end as usize];
    let mut filled = 0usize;
    while filled < out.len() {
        let mut read = 0u32;
        let rest = &mut out[filled..];
        let result =
            unsafe { stream.Read(rest.as_mut_ptr().cast(), rest.len() as u32, Some(&mut read)) };
        if result.is_err() || read == 0 {
            return Err(ImageError::Invalid);
        }
        filled += read as usize;
    }
    Ok(out)
}

/// PNG of `source`, in a pixel format the PNG encoder accepts.
fn encode_png(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
) -> Result<Vec<u8>, ImageError> {
    let (mut width, mut height) = (0u32, 0u32);
    unsafe { source.GetSize(&mut width, &mut height) }.map_err(invalid)?;
    let stream = unsafe { SHCreateMemStream(None) }.ok_or(ImageError::Invalid)?;
    let encoder = unsafe { factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()) }
        .map_err(invalid)?;
    unsafe { encoder.Initialize(&stream, WICBitmapEncoderNoCache) }.map_err(invalid)?;
    let (mut frame, mut options) = (None, None);
    unsafe { encoder.CreateNewFrame(&mut frame, &mut options) }.map_err(invalid)?;
    let frame = frame.ok_or(ImageError::Invalid)?;
    unsafe { frame.Initialize(options.as_ref()) }.map_err(invalid)?;
    unsafe { frame.SetSize(width, height) }.map_err(invalid)?;
    let wanted = unsafe { source.GetPixelFormat() }.map_err(invalid)?;
    let mut format = wanted;
    unsafe { frame.SetPixelFormat(&mut format) }.map_err(invalid)?;
    let input = if format == wanted {
        source.clone()
    } else {
        convert(factory, source, &format)?
    };
    unsafe { frame.WriteSource(&input, std::ptr::null()) }.map_err(invalid)?;
    unsafe { frame.Commit() }.map_err(invalid)?;
    unsafe { encoder.Commit() }.map_err(invalid)?;
    read_stream(&stream)
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

/// Decodes into a bottom-up `CF_DIBV5`, a strip of rows at a time.
fn dibv5(source: &IWICBitmapSource, width: u32, height: u32) -> Result<Vec<u8>, ImageError> {
    let row = width as usize * 4;
    let mut dib = dibv5_header(width, height);
    dib.resize(DIBV5_HEADER + row * height as usize, 0);
    let mut strip = vec![0u8; row * STRIP_ROWS.min(height) as usize];
    let mut y = 0u32;
    while y < height {
        let rows = STRIP_ROWS.min(height - y);
        let rect = WICRect {
            X: 0,
            Y: y as i32,
            Width: width as i32,
            Height: rows as i32,
        };
        let part = &mut strip[..row * rows as usize];
        unsafe { source.CopyPixels(&rect, row as u32, part) }.map_err(invalid)?;
        for i in 0..rows {
            let target = DIBV5_HEADER + (height - 1 - (y + i)) as usize * row;
            let from = i as usize * row;
            dib[target..target + row].copy_from_slice(&part[from..from + row]);
        }
        y += rows;
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
    let _com = Com::init();
    let factory = factory()?;
    let (source, width, height) = decode(&factory, bytes)?;
    let bgra = convert(&factory, &source, &GUID_WICPixelFormat32bppBGRA)?;
    let dib = dibv5(&bgra, width, height)?;
    let png = if png {
        bytes.to_vec()
    } else {
        encode_png(&factory, &bgra)?
    };
    Ok((png, dib))
}

/// A BMP file around a packed DIB (`CF_DIB` or `CF_DIBV5`), so WIC's BMP decoder reads it.
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
    let _com = Com::init();
    let factory = factory()?;
    let png = match image {
        ClipImage::Png(bytes) => {
            if !is_png(&bytes) {
                return Err(ImageError::Invalid);
            }
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                return Err(ImageError::TooLarge);
            }
            decode(&factory, &bytes)?;
            bytes
        }
        ClipImage::Dib(dib) => {
            let bmp = bmp_file(&dib).ok_or(ImageError::Invalid)?;
            let (source, ..) = decode(&factory, &bmp)?;
            encode_png(&factory, &source)?
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
    use windows::Win32::Graphics::Imaging::GUID_ContainerFormatJpeg;

    /// Encodes BGRA pixels (top-down) with WIC, as a phone or a browser would.
    pub(crate) fn encode(container: &GUID, width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
        let _com = Com::init();
        let factory = factory().unwrap();
        let bitmap = unsafe {
            factory.CreateBitmapFromMemory(
                width,
                height,
                &GUID_WICPixelFormat32bppBGRA,
                width * 4,
                bgra,
            )
        }
        .unwrap();
        let source: IWICBitmapSource = bitmap.cast().unwrap();
        if *container == GUID_ContainerFormatPng {
            return encode_png(&factory, &source).unwrap();
        }
        let stream = unsafe { SHCreateMemStream(None) }.unwrap();
        let encoder = unsafe { factory.CreateEncoder(container, std::ptr::null()) }.unwrap();
        unsafe { encoder.Initialize(&stream, WICBitmapEncoderNoCache) }.unwrap();
        let (mut frame, mut options) = (None, None);
        unsafe { encoder.CreateNewFrame(&mut frame, &mut options) }.unwrap();
        let frame = frame.unwrap();
        unsafe { frame.Initialize(options.as_ref()) }.unwrap();
        unsafe { frame.SetSize(width, height) }.unwrap();
        let mut format = GUID_WICPixelFormat32bppBGRA;
        unsafe { frame.SetPixelFormat(&mut format) }.unwrap();
        let input = convert(&factory, &source, &format).unwrap();
        unsafe { frame.WriteSource(&input, std::ptr::null()) }.unwrap();
        unsafe { frame.Commit() }.unwrap();
        unsafe { encoder.Commit() }.unwrap();
        read_stream(&stream).unwrap()
    }

    /// Top-down BGRA of an encoded image.
    fn pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let _com = Com::init();
        let factory = factory().unwrap();
        let (source, width, height) = decode(&factory, bytes).unwrap();
        let bgra = convert(&factory, &source, &GUID_WICPixelFormat32bppBGRA).unwrap();
        let mut out = vec![0u8; (width * height * 4) as usize];
        unsafe { bgra.CopyPixels(std::ptr::null(), width * 4, &mut out) }.unwrap();
        (width, height, out)
    }

    /// 3 × 2: red, green, blue / white, half-transparent black, transparent.
    pub(crate) fn sample() -> Vec<u8> {
        vec![
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, //
            255, 255, 255, 255, 0, 0, 0, 128, 0, 0, 0, 0,
        ]
    }

    pub(crate) fn sample_png() -> Vec<u8> {
        encode(&GUID_ContainerFormatPng, 3, 2, &sample())
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
        // Windows reads the DIB back as the same picture.
        let png_again = png_for_phone(ClipImage::Dib(dib)).unwrap();
        let (width, height, back) = pixels(&png_again);
        assert_eq!((width, height), (3, 2));
        assert_eq!(back, sample());
    }

    #[test]
    fn jpeg_from_the_phone_becomes_png() {
        let opaque: Vec<u8> = [[40u8, 80, 120, 255]; 64 * 32].concat();
        let jpeg = encode(&GUID_ContainerFormatJpeg, 64, 32, &opaque);
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
