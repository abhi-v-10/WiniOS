//! Bounded, memory-only PNG and packed DIB conversion. No native handles or I/O.
use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Seek, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: usize = 8 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_RGBA_BYTES: usize = MAX_PIXELS * 4;
pub const MAX_DIB_BYTES: usize = MAX_RGBA_BYTES + 4096;
pub const CODEC_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub enabled: bool,
    pub max_mib: u8,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: false,
            max_mib: 8,
        }
    }
}
impl Options {
    pub fn validate(self) -> Result<Self, &'static str> {
        if matches!(self.max_mib, 2 | 4 | 8) {
            Ok(self)
        } else {
            Err("Image maximum must be 2, 4 or 8 MiB.")
        }
    }
    pub fn max_bytes(self) -> usize {
        (self.max_mib as usize * 1024 * 1024).min(MAX_PNG_BYTES)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    Invalid,
    TooLarge,
    Timeout,
}
#[derive(Clone)]
pub struct Budget {
    deadline: Instant,
    cancel: Arc<AtomicBool>,
}
impl Budget {
    pub fn new() -> Self {
        Self {
            deadline: Instant::now() + CODEC_TIMEOUT,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<(), ImageError> {
        if Instant::now() >= self.deadline || self.cancel.load(Ordering::Acquire) {
            Err(ImageError::Timeout)
        } else {
            Ok(())
        }
    }
}
pub struct CancelBudget(pub Budget);
impl Drop for CancelBudget {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: u32,
    pub height: u32,
    png: Bytes,
}
impl std::fmt::Debug for ClipboardImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.png.len())
            .finish()
    }
}
impl ClipboardImage {
    pub fn png(&self) -> Bytes {
        self.png.clone()
    }
    pub fn len(&self) -> usize {
        self.png.len()
    }
}
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub struct Prepared {
    pub image: Arc<ClipboardImage>,
    pub dib_v5: Vec<u8>,
    pub dib_24: Vec<u8>,
}
pub enum Source {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

pub fn dimensions(width: u32, height: u32) -> Result<usize, ImageError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageError::TooLarge)?;
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION || pixels > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    pixels.checked_mul(4).ok_or(ImageError::TooLarge)
}
fn le32(bytes: &[u8], offset: usize) -> Result<u32, ImageError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(ImageError::Invalid)?
            .try_into()
            .map_err(|_| ImageError::Invalid)?,
    ))
}
fn be32(bytes: &[u8], offset: usize) -> Result<u32, ImageError> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(ImageError::Invalid)?
            .try_into()
            .map_err(|_| ImageError::Invalid)?,
    ))
}
/// HGLOBAL allocation size can include native padding after PNG's IEND.
/// Only native reads use this; HTTP uploads must end exactly at IEND.
pub fn native_png_length(bytes: &[u8]) -> Result<usize, ImageError> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(ImageError::Invalid);
    }
    let mut at = 8;
    for _ in 0..4096 {
        let size = be32(bytes, at)? as usize;
        let end = at
            .checked_add(12)
            .and_then(|v| v.checked_add(size))
            .filter(|v| *v <= bytes.len())
            .ok_or(ImageError::Invalid)?;
        if &bytes[at + 4..at + 8] == b"IEND" {
            return if size == 0 {
                Ok(end)
            } else {
                Err(ImageError::Invalid)
            };
        }
        at = end;
    }
    Err(ImageError::TooLarge)
}

// Validate framing, CRCs and dimensions BEFORE the decoder can allocate. Remove
// metadata (including compressed profiles/text) instead of inflating or exporting it.
fn clean_png(input: &[u8], options: Options, budget: &Budget) -> Result<Vec<u8>, ImageError> {
    if input.len() > options.max_bytes() {
        return Err(ImageError::TooLarge);
    }
    if input.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(ImageError::Invalid);
    }
    let mut clean = input[..8].to_vec();
    let mut at = 8;
    let mut count = 0;
    let mut idat = false;
    while at < input.len() {
        budget.check()?;
        count += 1;
        if count > 4096 {
            return Err(ImageError::TooLarge);
        }
        let size = be32(input, at)? as usize;
        let end = at
            .checked_add(12)
            .and_then(|v| v.checked_add(size))
            .filter(|v| *v <= input.len())
            .ok_or(ImageError::Invalid)?;
        let kind = &input[at + 4..at + 8];
        if !kind.iter().all(u8::is_ascii_alphabetic)
            || crc32fast::hash(&input[at + 4..end - 4]) != be32(input, end - 4)?
        {
            return Err(ImageError::Invalid);
        }
        if count == 1 {
            if kind != b"IHDR" || size != 13 {
                return Err(ImageError::Invalid);
            }
            dimensions(be32(input, at + 8)?, be32(input, at + 12)?)?;
        } else if kind == b"IHDR" {
            return Err(ImageError::Invalid);
        }
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
            return Err(ImageError::Invalid);
        }
        match kind {
            b"IHDR" | b"PLTE" | b"tRNS" | b"IDAT" | b"IEND" => {
                clean.extend_from_slice(&input[at..end])
            }
            _ if kind[0] & 32 == 0 => return Err(ImageError::Invalid),
            _ => {}
        }
        if kind == b"IDAT" {
            idat = true;
        }
        if kind == b"IEND" {
            return if size == 0 && end == input.len() && idat {
                Ok(clean)
            } else {
                Err(ImageError::Invalid)
            };
        }
        at = end;
    }
    Err(ImageError::Invalid)
}
struct Timed<'a> {
    cursor: io::Cursor<&'a [u8]>,
    budget: &'a Budget,
}
impl Read for Timed<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.budget.check().map_err(|_| io::ErrorKind::TimedOut)?;
        let count = out.len().min(16384);
        self.cursor.read(&mut out[..count])
    }
}
impl Seek for Timed<'_> {
    fn seek(&mut self, to: io::SeekFrom) -> io::Result<u64> {
        self.budget.check().map_err(|_| io::ErrorKind::TimedOut)?;
        self.cursor.seek(to)
    }
}
pub fn decode_png(input: &[u8], options: Options, budget: &Budget) -> Result<Decoded, ImageError> {
    let clean = clean_png(input, options, budget)?;
    let mut decoder = png::Decoder::new_with_limits(
        io::BufReader::new(Timed {
            cursor: io::Cursor::new(&clean),
            budget,
        }),
        png::Limits {
            bytes: MAX_RGBA_BYTES + 65536,
        },
    );
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| {
        if budget.check().is_err() {
            ImageError::Timeout
        } else {
            ImageError::Invalid
        }
    })?;
    let (width, height) = (reader.info().width, reader.info().height);
    let bytes = dimensions(width, height)?;
    let size = reader
        .output_buffer_size()
        .filter(|v| *v <= bytes)
        .ok_or(ImageError::TooLarge)?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).map_err(|_| {
        if budget.check().is_err() {
            ImageError::Timeout
        } else {
            ImageError::Invalid
        }
    })?;
    reader.finish().map_err(|_| {
        if budget.check().is_err() {
            ImageError::Timeout
        } else {
            ImageError::Invalid
        }
    })?;
    budget.check()?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err(ImageError::Invalid),
    };
    if info.bit_depth != png::BitDepth::Eight || info.buffer_size() != bytes / 4 * channels {
        return Err(ImageError::Invalid);
    }
    if channels == 4 {
        return Ok(Decoded {
            width,
            height,
            rgba: buffer,
        });
    }
    let mut rgba = Vec::with_capacity(bytes);
    for row in buffer[..info.buffer_size()].chunks(width as usize * channels) {
        budget.check()?;
        for pixel in row.chunks_exact(channels) {
            match channels {
                3 => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
                2 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
                _ => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            }
        }
    }
    Ok(Decoded {
        width,
        height,
        rgba,
    })
}
struct Limited<'a> {
    bytes: Vec<u8>,
    max: usize,
    large: bool,
    budget: &'a Budget,
}
impl Write for Limited<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.budget.check().map_err(|_| io::ErrorKind::TimedOut)?;
        if input.len() > self.max.saturating_sub(self.bytes.len()) {
            self.large = true;
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn encode_png(
    decoded: &Decoded,
    options: Options,
    budget: &Budget,
) -> Result<Arc<ClipboardImage>, ImageError> {
    if dimensions(decoded.width, decoded.height)? != decoded.rgba.len() {
        return Err(ImageError::Invalid);
    }
    budget.check()?;
    let mut output = Limited {
        bytes: Vec::new(),
        max: options.max_bytes(),
        large: false,
        budget,
    };
    let result = (|| {
        let mut encoder = png::Encoder::new(&mut output, decoded.width, decoded.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&decoded.rgba)?;
        writer.finish()
    })();
    budget.check()?;
    if result.is_err() {
        return Err(if output.large {
            ImageError::TooLarge
        } else {
            ImageError::Invalid
        });
    }
    Ok(Arc::new(ClipboardImage {
        width: decoded.width,
        height: decoded.height,
        png: Bytes::from(output.bytes),
    }))
}
pub fn decode_dib(input: &[u8], budget: &Budget) -> Result<Decoded, ImageError> {
    budget.check()?;
    if input.len() > MAX_DIB_BYTES {
        return Err(ImageError::TooLarge);
    }
    let header = le32(input, 0)? as usize;
    if !matches!(header, 40 | 108 | 124) || input.len() < header {
        return Err(ImageError::Invalid);
    }
    let width = le32(input, 4)? as i32;
    let signed_height = le32(input, 8)? as i32;
    let height = signed_height.checked_abs().ok_or(ImageError::Invalid)? as u32;
    if width <= 0 || input[12..14] != [1, 0] {
        return Err(ImageError::Invalid);
    }
    let bytes = dimensions(width as u32, height)?;
    let bpp = u16::from_le_bytes([input[14], input[15]]);
    let compression = le32(input, 16)?;
    if !matches!(bpp, 24 | 32)
        || !matches!(compression, 0 | 3)
        || (compression == 3 && bpp != 32)
        || le32(input, 32)? != 0
    {
        return Err(ImageError::Invalid);
    }
    if header == 124 && (le32(input, 112)? != 0 || le32(input, 116)? != 0) {
        return Err(ImageError::Invalid);
    }
    let mut start = header;
    let mut alpha = false;
    if compression == 3 {
        if le32(input, 40)? != 0xff0000 || le32(input, 44)? != 0xff00 || le32(input, 48)? != 0xff {
            return Err(ImageError::Invalid);
        }
        if header == 40 {
            start += 12;
        } else {
            let mask = le32(input, 52)?;
            if mask != 0 && mask != 0xff000000 {
                return Err(ImageError::Invalid);
            }
            alpha = mask != 0;
        }
    }
    let stride = (width as usize * bpp as usize).div_ceil(32) * 4;
    let payload = stride
        .checked_mul(height as usize)
        .ok_or(ImageError::TooLarge)?;
    if start
        .checked_add(payload)
        .filter(|end| *end <= input.len())
        .is_none()
    {
        return Err(ImageError::Invalid);
    }
    let declared = le32(input, 20)? as usize;
    if declared != 0 && (declared < payload || declared > input.len() - start) {
        return Err(ImageError::Invalid);
    }
    let mut rgba = vec![0; bytes];
    for y in 0..height as usize {
        budget.check()?;
        let source_y = if signed_height < 0 {
            y
        } else {
            height as usize - 1 - y
        };
        let source = &input[start + source_y * stride..start + (source_y + 1) * stride];
        for x in 0..width as usize {
            let from = x * (bpp as usize / 8);
            let to = (y * width as usize + x) * 4;
            rgba[to..to + 4].copy_from_slice(&[
                source[from + 2],
                source[from + 1],
                source[from],
                if alpha { source[from + 3] } else { 255 },
            ]);
        }
    }
    Ok(Decoded {
        width: width as u32,
        height,
        rgba,
    })
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
pub fn dib(decoded: &Decoded, v5: bool, budget: &Budget) -> Result<Vec<u8>, ImageError> {
    budget.check()?;
    if dimensions(decoded.width, decoded.height)? != decoded.rgba.len() {
        return Err(ImageError::Invalid);
    }
    let header = if v5 { 124 } else { 40 };
    let bpp = if v5 { 32 } else { 24 };
    let stride = (decoded.width as usize * bpp).div_ceil(32) * 4;
    let mut output = vec![0; header + stride * decoded.height as usize];
    put(&mut output, 0, header as u32);
    put(&mut output, 4, decoded.width);
    put(&mut output, 8, decoded.height);
    output[12..14].copy_from_slice(&1u16.to_le_bytes());
    output[14..16].copy_from_slice(&(bpp as u16).to_le_bytes());
    put(&mut output, 20, (stride * decoded.height as usize) as u32);
    if v5 {
        put(&mut output, 16, 3);
        for (offset, mask) in [
            (40, 0xff0000),
            (44, 0xff00),
            (48, 0xff),
            (52, 0xff000000),
            (56, 0x73524742),
            (108, 4),
        ] {
            put(&mut output, offset, mask);
        }
    }
    for y in 0..decoded.height as usize {
        budget.check()?;
        for x in 0..decoded.width as usize {
            let from = (y * decoded.width as usize + x) * 4;
            let to = header + (decoded.height as usize - 1 - y) * stride + x * (bpp / 8);
            let p = &decoded.rgba[from..from + 4];
            for (channel, color) in [p[2], p[1], p[0]].into_iter().enumerate() {
                output[to + channel] = if v5 {
                    color
                } else {
                    ((color as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
                };
            }
            if v5 {
                output[to + 3] = p[3];
            }
        }
    }
    Ok(output)
}
pub fn prepare_png(
    input: &[u8],
    options: Options,
    budget: &Budget,
) -> Result<Prepared, ImageError> {
    let decoded = decode_png(input, options, budget)?;
    Ok(Prepared {
        image: encode_png(&decoded, options, budget)?,
        dib_v5: dib(&decoded, true, budget)?,
        dib_24: dib(&decoded, false, budget)?,
    })
}
pub fn convert(
    source: Source,
    options: Options,
    budget: &Budget,
) -> Result<Arc<ClipboardImage>, ImageError> {
    let decoded = match source {
        Source::Png(bytes) => decode_png(&bytes, options, budget)?,
        Source::Dib(bytes) => decode_dib(&bytes, budget)?,
    };
    encode_png(&decoded, options, budget)
}

#[cfg(test)]
pub fn fixture() -> Decoded {
    Decoded {
        width: 2,
        height: 2,
        rgba: vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 25, 50, 75, 255,
        ],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(&crc32fast::hash(&bytes[4..]).to_be_bytes());
        bytes
    }
    #[test]
    fn deterministic_malformed_corpus_limits_and_expired_codec_are_bounded() {
        let options = Options::default();
        let png = encode_png(&fixture(), options, &Budget::new())
            .unwrap()
            .png();
        let mut seed = 7u32;
        for len in 0..512 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    (seed >> 24) as u8
                })
                .collect();
            assert!(decode_png(&bytes, options, &Budget::new()).is_err());
            assert!(decode_dib(&bytes, &Budget::new()).is_err());
        }
        for (width, height) in [(8192u32, 1025u32), (1, u32::MAX), (0, 1)] {
            let mut invalid = png.to_vec();
            invalid[16..20].copy_from_slice(&width.to_be_bytes());
            invalid[20..24].copy_from_slice(&height.to_be_bytes());
            let crc = crc32fast::hash(&invalid[12..29]);
            invalid[29..33].copy_from_slice(&crc.to_be_bytes());
            assert!(decode_png(&invalid, options, &Budget::new()).is_err());
        }
        let budget = Budget {
            deadline: Instant::now() - Duration::from_secs(1),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let packed = dib(&fixture(), true, &Budget::new()).unwrap();
        assert_eq!(
            decode_dib(&packed, &budget).err(),
            Some(ImageError::Timeout)
        );
        assert_eq!(
            dib(&fixture(), true, &budget).err(),
            Some(ImageError::Timeout)
        );
        assert_eq!(
            decode_png(&png, options, &budget).err(),
            Some(ImageError::Timeout)
        );
    }
    #[test]
    fn valid_crc_bombs_animation_trailing_data_and_native_padding() {
        let options = Options {
            enabled: true,
            max_mib: 2,
        };
        let budget = Budget::new();
        let png = encode_png(&fixture(), options, &budget).unwrap().png();
        let mut bomb = png.to_vec();
        bomb[16..20].copy_from_slice(&8193u32.to_be_bytes());
        let crc = crc32fast::hash(&bomb[12..29]);
        bomb[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(
            decode_png(&bomb, options, &budget).err(),
            Some(ImageError::TooLarge)
        );
        let mut animated = png[..33].to_vec();
        animated.extend(chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        animated.extend_from_slice(&png[33..]);
        assert!(decode_png(&animated, options, &budget).is_err());
        let mut padded = png.to_vec();
        padded.extend_from_slice(&[0; 32]);
        assert_eq!(native_png_length(&padded).unwrap(), png.len());
        assert!(decode_png(&padded, options, &budget).is_err());
        let metadata = vec![0; options.max_bytes() - png.len() - 12];
        let mut exact = png[..33].to_vec();
        exact.extend(chunk(b"vpAg", &metadata));
        exact.extend_from_slice(&png[33..]);
        assert_eq!(exact.len(), options.max_bytes());
        assert!(decode_png(&exact, options, &budget).is_ok());
        exact.push(0);
        assert_eq!(
            decode_png(&exact, options, &budget).err(),
            Some(ImageError::TooLarge)
        );
    }
    #[test]
    fn dib_top_down_rgb_alpha_rules_and_header_corruption() {
        let budget = Budget::new();
        let source = fixture();
        let mut top = dib(&source, true, &budget).unwrap();
        put(&mut top, 8, (-2i32) as u32);
        let (row1, row2) = top[124..].split_at_mut(8);
        row1.swap_with_slice(row2);
        assert_eq!(decode_dib(&top, &budget).unwrap().rgba, source.rgba);
        let native = dib(&source, true, &budget).unwrap();
        let mut rgb = native[..40].to_vec();
        put(&mut rgb, 0, 40);
        put(&mut rgb, 16, 0);
        rgb.extend_from_slice(&native[124..]);
        let decoded = decode_dib(&rgb, &budget).unwrap();
        assert!(decoded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        for (offset, value) in [
            (4, u32::MAX),
            (8, 0x80000000),
            (16, 4),
            (20, u32::MAX),
            (32, 1),
            (40, 0xffff),
            (112, 124),
        ] {
            let mut bad = native.clone();
            put(&mut bad, offset, value);
            assert!(decode_dib(&bad, &budget).is_err());
        }
        for length in 0..native.len() {
            assert!(decode_dib(&native[..length], &budget).is_err());
        }
    }
    #[test]
    fn png_and_native_dib_roundtrips_preserve_rgba_and_flatten_fallback() {
        let budget = Budget::new();
        let options = Options {
            enabled: true,
            ..Default::default()
        };
        let source = fixture();
        let png = encode_png(&source, options, &budget).unwrap();
        let prepared = prepare_png(&png.png(), options, &budget).unwrap();
        assert_eq!(
            decode_dib(&prepared.dib_v5, &budget).unwrap().rgba,
            source.rgba
        );
        let fallback = decode_dib(&prepared.dib_24, &budget).unwrap();
        assert_eq!(&fallback.rgba[8..12], &[255, 255, 255, 255]);
        assert_eq!(
            decode_png(&png.png(), options, &budget).unwrap().rgba,
            source.rgba
        );
    }
    #[test]
    fn malformed_sizes_crc_truncation_animation_and_cancellation_fail_closed() {
        let budget = Budget::new();
        let options = Options::default();
        let png = encode_png(&fixture(), options, &budget).unwrap().png();
        for length in 0..png.len() {
            assert!(decode_png(&png[..length], options, &budget).is_err());
        }
        let mut bad = png.to_vec();
        bad[20] ^= 1;
        assert!(decode_png(&bad, options, &budget).is_err());
        assert_eq!(dimensions(8193, 1), Err(ImageError::TooLarge));
        assert_eq!(dimensions(8192, 8192), Err(ImageError::TooLarge));
        assert!(decode_png(&vec![0; MAX_PNG_BYTES + 1], options, &budget).is_err());
        for length in 0..124 {
            assert!(decode_dib(&vec![0; length], &budget).is_err());
        }
        budget.cancel();
        assert_eq!(
            prepare_png(&png, options, &budget).err(),
            Some(ImageError::Timeout)
        );
    }
}
