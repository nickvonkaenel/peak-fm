use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use base64::{engine::general_purpose::STANDARD, Engine};
use image::ImageReader;

/// Check if a file is an image based on extension
pub fn is_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tif")
    )
}

/// Encoded image data ready for display
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImagePreview {
    /// The encoded escape sequence bytes
    pub data: Vec<u8>,
    /// Width in terminal cells
    pub cell_width: u16,
    /// Height in terminal cells
    pub cell_height: u16,
}

impl ImagePreview {
    /// Load and encode an image for terminal display using Kitty graphics protocol
    /// max_width and max_height are in terminal cells
    pub fn load(path: &Path, max_width: u16, max_height: u16) -> io::Result<Self> {
        // Load the image
        let img = ImageReader::open(path)
            .map_err(io::Error::other)?
            .decode()
            .map_err(io::Error::other)?;

        // Pixels per cell (roughly 8x16 for most terminals)
        let cell_px_width = 8u32;
        let cell_px_height = 16u32;

        // Target pixel dimensions based on available cells
        let target_px_width = max_width as u32 * cell_px_width;
        let target_px_height = max_height as u32 * cell_px_height;

        // Check if we need to shrink the image
        let original_width = img.width();
        let original_height = img.height();
        let needs_shrink = original_width > target_px_width || original_height > target_px_height;

        // Resize image to fit target while maintaining aspect ratio (only shrinks, never enlarges)
        let img = img.thumbnail(target_px_width, target_px_height);
        let (width, height) = (img.width(), img.height());

        // Calculate cell dimensions for display (only used when shrinking)
        let cell_width = width.div_ceil(cell_px_width) as u16;
        let cell_height = height.div_ceil(cell_px_height) as u16;

        // Get raw RGBA data
        let rgba = img.to_rgba8();
        let raw = rgba.as_raw();

        // Encode using Kitty graphics protocol
        // Only pass cell dimensions when shrinking to avoid scaling up small images
        let data = encode_kitty(
            raw,
            width,
            height,
            if needs_shrink {
                Some((cell_width, cell_height))
            } else {
                None
            },
        )?;

        Ok(Self {
            data,
            cell_width,
            cell_height,
        })
    }
}

/// Result of async image loading
pub struct ImageLoadResult {
    pub path: PathBuf,
    pub image: Option<ImagePreview>,
}

/// Start loading an image asynchronously
/// Returns a receiver that will contain the result when ready
pub fn load_image_async(
    path: PathBuf,
    max_width: u16,
    max_height: u16,
) -> Receiver<ImageLoadResult> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let image = ImagePreview::load(&path, max_width, max_height).ok();
        let _ = tx.send(ImageLoadResult { path, image });
    });

    rx
}

/// Encode image data using Kitty graphics protocol
/// cell_size is optional - when provided, tells Kitty to scale the image to fit those cells
/// When None, Kitty displays the image at native resolution (1:1 pixels)
fn encode_kitty(
    raw: &[u8],
    width: u32,
    height: u32,
    cell_size: Option<(u16, u16)>,
) -> io::Result<Vec<u8>> {
    let b64 = STANDARD.encode(raw);
    let b64_bytes = b64.as_bytes();

    let mut buf = Vec::with_capacity(b64.len() + 1000);

    // Split into 4096-byte chunks
    let mut chunks = b64_bytes.chunks(4096).peekable();

    if let Some(first) = chunks.next() {
        // First chunk includes image metadata
        // f=32 means RGBA, a=T means transmit and display, q=2 means quiet (no response)
        // s,v = source dimensions, c,r = display size in cells (optional)
        if let Some((cell_cols, cell_rows)) = cell_size {
            write!(
                buf,
                "\x1b_Gq=2,a=T,f=32,s={},v={},c={},r={},m={};",
                width,
                height,
                cell_cols,
                cell_rows,
                if chunks.peek().is_some() { 1 } else { 0 }
            )?;
        } else {
            // No cell size - display at native resolution
            write!(
                buf,
                "\x1b_Gq=2,a=T,f=32,s={},v={},m={};",
                width,
                height,
                if chunks.peek().is_some() { 1 } else { 0 }
            )?;
        }
        buf.extend_from_slice(first);
        buf.extend_from_slice(b"\x1b\\");
    }

    // Remaining chunks
    while let Some(chunk) = chunks.next() {
        write!(
            buf,
            "\x1b_Gm={};",
            if chunks.peek().is_some() { 1 } else { 0 }
        )?;
        buf.extend_from_slice(chunk);
        buf.extend_from_slice(b"\x1b\\");
    }

    Ok(buf)
}
