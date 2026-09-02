//! The terminal's clipboard, written the one way that needs no program
//! outside `bdi`.
//!
//! OSC 52 is the escape sequence a terminal takes a clipboard write on. It
//! travels through a multiplexer and ssh the way the rest of what `bdi` draws
//! does,
//! which a clipboard program would not, and a terminal that does not honour
//! it drops the sequence — so the write can do nothing, and cannot fail for
//! that.

use std::io::{self, Write};

use base64::prelude::{Engine as _, BASE64_STANDARD};

/// Put `text` on the clipboard of the terminal `out` reaches.
///
/// Flushed, because the sequence ends in no newline and a line-buffered
/// stdout would hold it until something else wrote one.
pub(super) fn copy(out: &mut dyn Write, text: &str) -> io::Result<()> {
    write!(out, "\x1b]52;c;{}\x07", BASE64_STANDARD.encode(text))?;
    out.flush()
}
