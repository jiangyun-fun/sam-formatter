//! Input source handling: opening a file or stdin, and sniffing SAM vs BAM.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};

use crate::cli::InputFormat;

/// A buffered reader over either a file or standard input.
pub type BufInput = BufReader<Box<dyn Read>>;

/// Opens `path` for reading. `"-"` (the default) reads from standard input.
pub fn open_input(path: &str) -> io::Result<BufInput> {
    let inner: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(path)?)
    };
    Ok(BufReader::new(inner))
}

/// Decides the input format.
///
/// When the override is `Auto`, the first bytes are peeked (without consuming them) so
/// the same reader can be handed to noodles: the BGZF/gzip magic `0x1f 0x8b` indicates
/// BAM; anything else is treated as SAM text.
pub fn detect_format(buf: &mut BufInput, override_fmt: InputFormat) -> io::Result<InputFormat> {
    if override_fmt != InputFormat::Auto {
        return Ok(override_fmt);
    }

    // `fill_buf` is non-consuming, so the peeked bytes stay in the buffer for noodles.
    let peek = buf.fill_buf()?;
    let is_bgzf = peek.starts_with(&[0x1f, 0x8b]);
    Ok(if is_bgzf {
        InputFormat::Bam
    } else {
        InputFormat::Sam
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn buf_of(bytes: Vec<u8>) -> BufInput {
        BufReader::new(Box::new(Cursor::new(bytes)))
    }

    #[test]
    fn detects_bgzf_magic_as_bam() {
        let mut buf = buf_of(vec![0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00]);
        assert_eq!(
            detect_format(&mut buf, InputFormat::Auto).unwrap(),
            InputFormat::Bam
        );
    }

    #[test]
    fn detects_text_as_sam() {
        let mut buf = buf_of(b"@HD\tVN:1.6\n".to_vec());
        assert_eq!(
            detect_format(&mut buf, InputFormat::Auto).unwrap(),
            InputFormat::Sam
        );
    }

    #[test]
    fn override_takes_precedence() {
        let mut buf = buf_of(vec![0x1f, 0x8b]);
        assert_eq!(
            detect_format(&mut buf, InputFormat::Sam).unwrap(),
            InputFormat::Sam
        );
    }

    #[test]
    fn sniff_does_not_consume_bytes() {
        let mut buf = buf_of(vec![0x1f, 0x8b, 0x42]);
        let _ = detect_format(&mut buf, InputFormat::Auto).unwrap();
        // The peeked magic bytes must still be available to read.
        let mut rest = Vec::new();
        buf.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, vec![0x1f, 0x8b, 0x42]);
    }
}
