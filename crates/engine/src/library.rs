use std::path::{Path, PathBuf};

/// File extensions recognized as playable audio, shared between the file
/// picker's filter and folder scanning so both stay in sync.
pub const AUDIO_EXTENSIONS: &[&str] = &["flac", "mp3", "wav", "ogg", "oga", "m4a", "aac"];

fn has_audio_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
}

/// Recursively collects every audio file under `dir`, sorted so the
/// resulting playlist order is predictable (alphabetical, depth-first).
pub fn scan_audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending_dirs = vec![dir.to_path_buf()];

    while let Some(current) = pending_dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending_dirs.push(path);
            } else if has_audio_extension(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_audio_files_recursively_and_ignores_others() {
        let root =
            std::env::temp_dir().join(format!("openbar5000-scan-test-{}", std::process::id()));
        let nested = root.join("Artist/Album");
        std::fs::create_dir_all(&nested).unwrap();

        std::fs::write(root.join("top.flac"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        std::fs::write(nested.join("track.MP3"), b"").unwrap();
        std::fs::write(nested.join("cover.jpg"), b"").unwrap();

        let found = scan_audio_files(&root);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found, vec![nested.join("track.MP3"), root.join("top.flac")]);
    }
}
