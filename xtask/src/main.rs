//! Build tasks.
//!
//! `cargo xtask icons` renders the application icon at every size the two installers want
//! and writes the `.icns`, `.ico` and `.png` files they embed. The artwork comes from
//! `eye_saver_core::icon`, the same code that draws the tray at runtime, so there is one
//! source of truth and nothing binary to keep in step by hand.
//!
//! The container formats are written directly rather than by shelling out to `iconutil`,
//! so the icons can be regenerated on any host - which matters, because the Windows
//! installer is built on a Windows runner that has no Apple tooling.

#![warn(clippy::pedantic)]
#![allow(clippy::doc_markdown)]

mod icns;
mod ico;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use eye_saver_core::icon;

/// Sizes stored in the Windows `.ico`. Windows picks the closest to what it needs.
const ICO_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

/// Sizes stored in the macOS `.icns`, paired with their `OSType` tags.
const ICNS_ENTRIES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("icons") => {
            let out = args.next().map_or_else(default_out_dir, PathBuf::from);
            match generate_icons(&out) {
                Ok(written) => {
                    for path in written {
                        println!("wrote {}", path.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("xtask: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!("xtask: unknown task '{other}'\n\nUsage: cargo xtask icons [out-dir]");
            ExitCode::from(2)
        }
        None => {
            eprintln!("Usage: cargo xtask icons [out-dir]");
            ExitCode::from(2)
        }
    }
}

/// `packaging/generated`, relative to the workspace root.
fn default_out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("packaging")
        .join("generated")
}

/// Writes every icon the installers need.
fn generate_icons(out: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    let mut written = Vec::new();

    let icns = out.join("EyeSaver.icns");
    fs::write(&icns, icns::build(&ICNS_ENTRIES)?)
        .map_err(|e| format!("writing {}: {e}", icns.display()))?;
    written.push(icns);

    let ico = out.join("EyeSaver.ico");
    fs::write(&ico, ico::build(&ICO_SIZES)?)
        .map_err(|e| format!("writing {}: {e}", ico.display()))?;
    written.push(ico);

    // A plain PNG for the README and for anything that wants a preview.
    let png = out.join("EyeSaver-512.png");
    fs::write(&png, encode_png(512)?).map_err(|e| format!("writing {}: {e}", png.display()))?;
    written.push(png);

    Ok(written)
}

/// Renders the application icon at `size` and encodes it as a PNG.
fn encode_png(size: u32) -> Result<Vec<u8>, String> {
    let image = icon::render_app_icon(size);
    let mut buffer = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buffer, size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("encoding a {size}px icon: {e}"))?;
        writer
            .write_image_data(image.as_bytes())
            .map_err(|e| format!("encoding a {size}px icon: {e}"))?;
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pngs_are_encoded_with_a_png_signature_and_decode_back() {
        let bytes = encode_png(32).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");

        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().width, 32);
        assert_eq!(reader.info().height, 32);
        let mut out = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut out).unwrap();
        assert_eq!(out.len(), 32 * 32 * 4);
    }

    #[test]
    fn the_default_output_directory_is_inside_the_repository() {
        let dir = default_out_dir();
        assert!(dir.ends_with("packaging/generated"), "{dir:?}");
    }

    #[test]
    fn every_icon_file_is_written() {
        let dir = std::env::temp_dir().join(format!("eye-saver-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let written = generate_icons(&dir).unwrap();
        assert_eq!(written.len(), 3);
        for path in &written {
            assert!(path.is_file(), "{path:?} was not written");
            assert!(fs::metadata(path).unwrap().len() > 0);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
