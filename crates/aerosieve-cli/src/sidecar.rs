//! Transcripts stored beside recordings as `<name>.txt`.
//!
//! A corpus may come from an untrusted source. The file is opened without
//! following a symlink and every check is made on the open handle, so a link
//! swapped in during the run cannot copy another file into the manifest.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::Path;

use crate::CliResult;

const EXTENSION: &str = "txt";
const MAX_BYTES: u64 = 1024 * 1024;

/// The transcript for `audio`, or `None` if it has no sidecar file.
pub fn transcript(audio: &Path) -> CliResult<Option<String>> {
    let path = audio.with_extension(EXTENSION);
    let file = match open_without_following_links(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display()).into()),
    };
    if !file.metadata()?.is_file() {
        return Err(format!("{}: not a regular file", path.display()).into());
    }

    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("{}: larger than {MAX_BYTES} bytes", path.display()).into());
    }
    Ok(Some(text))
}

#[cfg(unix)]
fn open_without_following_links(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_without_following_links(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    /// Opens a symlink itself rather than its target.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}
