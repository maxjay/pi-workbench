//! Files opened for viewing: text as text, images as images, and anything
//! else described rather than sent.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use protocol::{FileContent, FileView};

/// More than this of a text file isn't sent.
const TEXT_LIMIT: usize = 512 * 1024;
/// Images larger than this aren't sent.
const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;

pub fn read(path: &Path) -> std::io::Result<FileView> {
    let view = |content| FileView {
        path: path.display().to_string(),
        content,
    };
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(view(FileContent::Missing));
        }
        Err(err) => return Err(err),
    };
    if let Some(mime_type) = image_type(path) {
        if meta.len() > IMAGE_LIMIT {
            return Ok(view(FileContent::Binary { size: meta.len() }));
        }
        let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?);
        return Ok(view(FileContent::Image {
            mime_type: mime_type.into(),
            data,
        }));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(TEXT_LIMIT as u64)
        .read_to_end(&mut bytes)?;
    if bytes.contains(&0) {
        return Ok(view(FileContent::Binary { size: meta.len() }));
    }
    Ok(view(FileContent::Text {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated: meta.len() > TEXT_LIMIT as u64,
    }))
}

fn image_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_images_and_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "# Hi\n").unwrap();
        assert!(
            matches!(read(&note).unwrap().content, FileContent::Text { text, truncated: false } if text == "# Hi\n")
        );

        let shot = dir.path().join("shot.PNG");
        std::fs::write(&shot, [0x89, b'P', b'N', b'G']).unwrap();
        assert!(
            matches!(read(&shot).unwrap().content, FileContent::Image { mime_type, .. } if mime_type == "image/png")
        );

        let blob = dir.path().join("blob.bin");
        std::fs::write(&blob, [1, 0, 2]).unwrap();
        assert!(matches!(
            read(&blob).unwrap().content,
            FileContent::Binary { size: 3 }
        ));

        assert_eq!(
            read(&dir.path().join("gone")).unwrap().content,
            FileContent::Missing
        );
    }
}
