//! Writing a Windows icon file.
//!
//! A six byte directory header, one sixteen byte entry per image, then the payloads.
//! Windows Vista and later accept PNG payloads, which keeps the file small and avoids
//! hand-rolling a bottom-up BMP with a separate mask.

use crate::encode_png;

/// Bytes in the directory header.
const DIR_HEADER: usize = 6;
/// Bytes in one directory entry.
const DIR_ENTRY: usize = 16;

/// Builds an `.ico` containing one PNG per size.
///
/// Sizes must be 256 or smaller: the format stores a dimension in a single byte, where
/// zero means 256.
pub fn build(sizes: &[u32]) -> Result<Vec<u8>, String> {
    let mut images = Vec::new();
    for size in sizes {
        if *size == 0 || *size > 256 {
            return Err(format!("{size}px does not fit in an ico directory entry"));
        }
        images.push((*size, encode_png(*size)?));
    }

    let count = u16::try_from(images.len()).map_err(|_| "too many icon sizes".to_owned())?;
    let mut header = Vec::with_capacity(DIR_HEADER + images.len() * DIR_ENTRY);
    header.extend_from_slice(&0u16.to_le_bytes()); // reserved
    header.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon, 2 = cursor
    header.extend_from_slice(&count.to_le_bytes());

    let mut offset = DIR_HEADER + images.len() * DIR_ENTRY;
    let mut payloads = Vec::new();
    for (size, png) in &images {
        // 256 is stored as zero, which is why the format cannot hold anything larger.
        let dimension = u8::try_from(*size % 256).unwrap_or(0);
        header.push(dimension);
        header.push(dimension);
        header.push(0); // palette size: none, this is a true-colour image
        header.push(0); // reserved
        header.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        header.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        let length =
            u32::try_from(png.len()).map_err(|_| format!("the {size}px icon is too large"))?;
        header.extend_from_slice(&length.to_le_bytes());
        header.extend_from_slice(
            &u32::try_from(offset)
                .map_err(|_| "the icon file is too large".to_owned())?
                .to_le_bytes(),
        );
        offset += png.len();
        payloads.extend_from_slice(png);
    }

    header.extend_from_slice(&payloads);
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_describes_every_payload() {
        let sizes = [16u32, 32, 256];
        let bytes = build(&sizes).unwrap();

        assert_eq!(&bytes[0..2], &[0, 0], "reserved");
        assert_eq!(
            u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
            1,
            "type: icon"
        );
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 3);

        for (index, size) in sizes.iter().enumerate() {
            let entry = DIR_HEADER + index * DIR_ENTRY;
            let expected = if *size == 256 {
                0
            } else {
                u8::try_from(*size).unwrap()
            };
            assert_eq!(bytes[entry], expected, "width byte for {size}px");
            assert_eq!(bytes[entry + 1], expected, "height byte for {size}px");
            assert_eq!(
                u16::from_le_bytes(bytes[entry + 6..entry + 8].try_into().unwrap()),
                32
            );

            let length =
                u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
            let offset =
                u32::from_le_bytes(bytes[entry + 12..entry + 16].try_into().unwrap()) as usize;
            assert!(
                offset + length <= bytes.len(),
                "{size}px payload runs off the end"
            );
            assert_eq!(
                &bytes[offset..offset + 8],
                b"\x89PNG\r\n\x1a\n",
                "{size}px payload is not a PNG"
            );
        }
    }

    #[test]
    fn sizes_that_do_not_fit_the_format_are_refused() {
        assert!(build(&[512]).is_err());
        assert!(build(&[0]).is_err());
    }
}
