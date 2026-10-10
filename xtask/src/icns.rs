//! Writing an Apple icon suite.
//!
//! The format is a header followed by a flat list of elements: a four byte `OSType`, a
//! big-endian length that includes the element's own eight byte header, then the payload.
//! Modern macOS accepts PNG payloads for every type used here, so each entry is simply the
//! icon rendered at that size.

use crate::encode_png;

/// The file magic.
const MAGIC: &[u8; 4] = b"icns";
/// Bytes of header on the file and on each element.
const HEADER: usize = 8;

/// Builds an `.icns` containing one PNG per entry.
pub fn build(entries: &[(&[u8; 4], u32)]) -> Result<Vec<u8>, String> {
    let mut elements = Vec::new();
    for (ostype, size) in entries {
        let png = encode_png(*size)?;
        let length = u32::try_from(png.len() + HEADER)
            .map_err(|_| format!("a {size}px icon is too large for an icns element"))?;
        elements.extend_from_slice(*ostype);
        elements.extend_from_slice(&length.to_be_bytes());
        elements.extend_from_slice(&png);
    }

    let total = u32::try_from(elements.len() + HEADER)
        .map_err(|_| "the icon suite is too large".to_owned())?;
    let mut out = Vec::with_capacity(elements.len() + HEADER);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&elements);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_declares_its_own_length_and_every_element_is_walkable() {
        let entries: [(&[u8; 4], u32); 2] = [(b"icp4", 16), (b"ic07", 128)];
        let bytes = build(&entries).unwrap();

        assert_eq!(&bytes[..4], MAGIC);
        let declared = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(
            declared,
            bytes.len(),
            "the header must describe the whole file"
        );

        // Walk the elements the way macOS does and check we land exactly at the end.
        let mut offset = HEADER;
        let mut seen = Vec::new();
        while offset < bytes.len() {
            let ostype: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
            let length =
                u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            assert!(length > HEADER, "an element must have a payload");
            assert_eq!(
                &bytes[offset + HEADER..offset + HEADER + 8],
                b"\x89PNG\r\n\x1a\n"
            );
            seen.push(ostype);
            offset += length;
        }
        assert_eq!(
            offset,
            bytes.len(),
            "the elements must tile the file exactly"
        );
        assert_eq!(seen, [*b"icp4", *b"ic07"]);
    }

    #[test]
    fn an_empty_suite_is_just_a_header() {
        assert_eq!(build(&[]).unwrap().len(), HEADER);
    }
}
