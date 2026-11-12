//! User-supplied artwork and its shared midnight / cyan / blue / violet palette.
pub const NAVY: [u8; 3] = [12, 16, 36];
pub const SURFACE: [u8; 3] = [20, 27, 52];
pub const CYAN: [u8; 3] = [54, 217, 255];
pub const BLUE: [u8; 3] = [64, 117, 255];
pub const VIOLET: [u8; 3] = [161, 110, 255];

/// Resource loading is preferred; this exact 32px conversion is the tray fallback.
pub fn pixels(size: usize) -> Vec<u8> {
    let source = include_bytes!("../resources/winios-32.rgba");
    let mut pixels = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let from = ((y * 32 / size) * 32 + x * 32 / size) * 4;
            let to = (y * size + x) * 4;
            pixels[to..to + 4].copy_from_slice(&source[from..from + 4]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    #[test]
    fn supplied_artwork_has_transparent_edges_and_visible_colored_mark() {
        let pixels = super::pixels(32);
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(pixels[3], 0);
        assert!(pixels.chunks_exact(4).filter(|p| p[3] > 200).count() > 400);
        assert!(pixels.chunks_exact(4).any(|p| p[2] > 200 && p[3] > 200));
        let ico = include_bytes!("../resources/winios.ico");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
    }
}
