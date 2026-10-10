use std::io;
use std::path::{Path, PathBuf};

/// Extensions picked up when searching a directory. A file named on the
/// command line is always tried, whatever its extension.
const AUDIO_EXTENSIONS: [&str; 12] = [
    "aac", "caf", "flac", "m4a", "mkv", "mp3", "mp4", "oga", "ogg", "opus", "wav", "webm",
];

/// Expand `inputs` into audio files: files are kept as given, directories
/// are searched recursively. The result is sorted so runs are repeatable.
pub fn audio_files(inputs: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_dir() {
            collect_from_directory(input, &mut files)?;
        } else {
            files.push(input.clone());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_from_directory(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() && has_audio_extension(&entry.path()) {
                files.push(entry.path());
            }
        }
    }
    Ok(())
}

fn has_audio_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}
