//! Document management, dirty tracking, versioning, conflict detection, and file persistence.

pub mod auto_save;

pub use auto_save::{AutoSaveConfig, AutoSaveMode, AutoSaveService};

use crate::model::piece_table::PieceTable;
use encoding_rs::{Encoding, SHIFT_JIS, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};
use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use rustcode_platform::fs::{
    hash_content, FileChangeEvent, FileChangeType, FileConflict, FileMetadata, FileSystemError,
    FileSystemService,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Supported text encodings for loading and saving `TextDocument`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum TextEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
    ShiftJis,
}

impl TextEncoding {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Windows1252 => "Windows-1252",
            Self::ShiftJis => "Shift JIS",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        let norm = label.trim().to_ascii_lowercase().replace(['_', '-'], "");
        match norm.as_str() {
            "utf8" => Some(Self::Utf8),
            "utf8bom" | "utf8withbom" => Some(Self::Utf8Bom),
            "utf16le" => Some(Self::Utf16Le),
            "utf16be" => Some(Self::Utf16Be),
            "windows1252" | "cp1252" | "win1252" => Some(Self::Windows1252),
            "shiftjis" | "sjis" => Some(Self::ShiftJis),
            _ => None,
        }
    }

    pub fn has_bom(&self) -> bool {
        matches!(self, Self::Utf8Bom | Self::Utf16Le | Self::Utf16Be)
    }

    pub fn to_encoding_rs(&self) -> &'static Encoding {
        match self {
            Self::Utf8 | Self::Utf8Bom => UTF_8,
            Self::Utf16Le => UTF_16LE,
            Self::Utf16Be => UTF_16BE,
            Self::Windows1252 => WINDOWS_1252,
            Self::ShiftJis => SHIFT_JIS,
        }
    }

    /// Encode UTF-8 string content into raw file bytes using this encoding.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf8Bom => {
                let mut out = Vec::with_capacity(3 + text.len());
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                out.extend_from_slice(text.as_bytes());
                out
            }
            Self::Utf16Le => {
                let mut out = Vec::with_capacity(2 + text.len() * 2);
                out.extend_from_slice(&[0xFF, 0xFE]);
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_le_bytes());
                }
                out
            }
            Self::Utf16Be => {
                let mut out = Vec::with_capacity(2 + text.len() * 2);
                out.extend_from_slice(&[0xFE, 0xFF]);
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_be_bytes());
                }
                out
            }
            Self::Windows1252 => {
                let (cow, _, _) = WINDOWS_1252.encode(text);
                cow.into_owned()
            }
            Self::ShiftJis => {
                let (cow, _, _) = SHIFT_JIS.encode(text);
                cow.into_owned()
            }
        }
    }

    /// Decode raw file bytes into UTF-8 text and detected/applied `TextEncoding`.
    /// Rejects binary files (containing NUL bytes) and malformed byte sequences cleanly.
    pub fn decode(
        bytes: &[u8],
        hint: Option<TextEncoding>,
    ) -> Result<(String, TextEncoding), FileSystemError> {
        // 1. Check explicit BOM first
        if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
            let payload = &bytes[bom_len..];
            if enc == UTF_8 {
                if payload.contains(&0) {
                    return Err(binary_file_error());
                }
                let text = std::str::from_utf8(payload).map_err(|_| {
                    FileSystemError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid UTF-8 bytes after UTF-8 BOM",
                    ))
                })?;
                return Ok((text.to_string(), TextEncoding::Utf8Bom));
            }
            if enc == UTF_16LE || enc == UTF_16BE {
                if !payload.len().is_multiple_of(2) {
                    return Err(FileSystemError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid UTF-16 byte length (odd number of bytes)",
                    )));
                }
                let Some(cow) = enc.decode_without_bom_handling_and_without_replacement(payload)
                else {
                    return Err(FileSystemError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid UTF-16 surrogate sequence",
                    )));
                };
                if cow.contains('\0') {
                    return Err(binary_file_error());
                }
                let detected = if enc == UTF_16LE {
                    TextEncoding::Utf16Le
                } else {
                    TextEncoding::Utf16Be
                };
                return Ok((cow.into_owned(), detected));
            }
        }

        // 2. Explicit UTF-16 hint without BOM
        if let Some(enc_hint @ (TextEncoding::Utf16Le | TextEncoding::Utf16Be)) = hint {
            if !bytes.len().is_multiple_of(2) {
                return Err(FileSystemError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid UTF-16 byte length (odd number of bytes)",
                )));
            }
            let rs_enc = enc_hint.to_encoding_rs();
            let Some(cow) = rs_enc.decode_without_bom_handling_and_without_replacement(bytes)
            else {
                return Err(FileSystemError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid UTF-16 surrogate sequence",
                )));
            };
            if cow.contains('\0') {
                return Err(binary_file_error());
            }
            return Ok((cow.into_owned(), enc_hint));
        }

        // 3. Reject binary files containing NUL bytes
        if bytes.contains(&0) {
            return Err(binary_file_error());
        }

        // 4. Explicit single-byte / multi-byte hint
        if let Some(enc_hint) = hint {
            match enc_hint {
                TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
                    let text = std::str::from_utf8(bytes).map_err(|_| {
                        FileSystemError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid UTF-8 file content",
                        ))
                    })?;
                    return Ok((text.to_string(), enc_hint));
                }
                TextEncoding::Windows1252 => {
                    let (cow, _) = WINDOWS_1252.decode_without_bom_handling(bytes);
                    return Ok((cow.into_owned(), TextEncoding::Windows1252));
                }
                TextEncoding::ShiftJis => {
                    let Some(cow) =
                        SHIFT_JIS.decode_without_bom_handling_and_without_replacement(bytes)
                    else {
                        return Err(FileSystemError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid Shift-JIS byte sequence",
                        )));
                    };
                    return Ok((cow.into_owned(), TextEncoding::ShiftJis));
                }
                TextEncoding::Utf16Le | TextEncoding::Utf16Be => unreachable!(),
            }
        }

        // 5. Auto-detect: strict UTF-8 first, then Shift-JIS heuristic, then Windows-1252
        if let Ok(text) = std::str::from_utf8(bytes) {
            return Ok((text.to_string(), TextEncoding::Utf8));
        }

        if let Some(sjis_cow) =
            SHIFT_JIS.decode_without_bom_handling_and_without_replacement(bytes)
        {
            if looks_like_shift_jis(bytes, &sjis_cow) {
                return Ok((sjis_cow.into_owned(), TextEncoding::ShiftJis));
            }
        }

        let (win_cow, _) = WINDOWS_1252.decode_without_bom_handling(bytes);
        Ok((win_cow.into_owned(), TextEncoding::Windows1252))
    }
}

fn binary_file_error() -> FileSystemError {
    FileSystemError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Binary file detected (contains NUL bytes)",
    ))
}

fn looks_like_shift_jis(raw: &[u8], decoded: &str) -> bool {
    let has_jis_primary_lead = raw.iter().any(|&b| (0x81..=0x9F).contains(&b));
    let has_kana_or_cjk = decoded.chars().any(|c| {
        ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)
    });
    // Windows-1252 single-byte sequences with 0xE0..0xFF followed by ASCII letters can
    // accidentally match Shift-JIS Level-2 kanji; require primary JIS lead bytes (0x81..0x9F)
    // or kana output to pick Shift-JIS automatically.
    has_jis_primary_lead
        || (has_kana_or_cjk
            && decoded
                .chars()
                .any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)))
}

/// A managed text document backed by a Piece Table, tracking dirty state,
/// version history, disk metadata, encoding metadata, and file conflict status.
pub struct TextDocument {
    pub buffer: Arc<RwLock<PieceTable>>,
    pub path: Option<PathBuf>,
    pub metadata: Option<FileMetadata>,
    pub is_dirty: bool,
    pub version_id: u64,
    pub saved_version_id: u64,
    pub language_id: String,
    pub encoding: TextEncoding,
    pub on_did_change_dirty: Emitter<bool>,
    pub on_did_save: Emitter<PathBuf>,
}

impl TextDocument {
    pub fn new(content: &str, path: Option<PathBuf>, language_id: Option<&str>) -> Self {
        let lang = language_id
            .map(|s| s.to_string())
            .or_else(|| {
                path.as_ref()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .map(rustcode_base::map_ext_to_lang_id)
            })
            .unwrap_or_else(|| "plaintext".to_string());

        Self {
            buffer: Arc::new(RwLock::new(PieceTable::from_str(content))),
            path,
            metadata: None,
            is_dirty: false,
            version_id: 1,
            saved_version_id: 1,
            language_id: lang,
            encoding: TextEncoding::Utf8,
            on_did_change_dirty: Emitter::new(),
            on_did_save: Emitter::new(),
        }
    }

    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    pub fn set_encoding(&mut self, encoding: TextEncoding) {
        self.encoding = encoding;
    }

    /// Open an existing file from disk synchronously with encoding auto-detection.
    pub fn open_sync<P: AsRef<Path>>(
        fs: &FileSystemService,
        path: P,
    ) -> Result<Self, FileSystemError> {
        Self::open_with_encoding_sync(fs, path, None)
    }

    /// Open an existing file from disk synchronously with an optional explicit encoding.
    pub fn open_with_encoding_sync<P: AsRef<Path>>(
        _fs: &FileSystemService,
        path: P,
        encoding_hint: Option<TextEncoding>,
    ) -> Result<Self, FileSystemError> {
        let p = path.as_ref();
        if !p.exists() {
            return Err(FileSystemError::NotFound(p.to_path_buf()));
        }
        let raw_bytes = std::fs::read(p)?;
        let (content, encoding) = TextEncoding::decode(&raw_bytes, encoding_hint)?;
        let meta = FileSystemService::compute_metadata(p, &content)?;

        let mut doc = Self::new(&content, Some(p.to_path_buf()), None);
        doc.encoding = encoding;
        doc.metadata = Some(meta);
        doc.is_dirty = false;
        doc.saved_version_id = doc.version_id;
        Ok(doc)
    }

    /// Open an existing file from disk asynchronously with encoding auto-detection.
    pub async fn open_async<P: AsRef<Path>>(path: P) -> Result<Self, FileSystemError> {
        Self::open_with_encoding_async(path, None).await
    }

    /// Open an existing file from disk asynchronously with an optional explicit encoding.
    pub async fn open_with_encoding_async<P: AsRef<Path>>(
        path: P,
        encoding_hint: Option<TextEncoding>,
    ) -> Result<Self, FileSystemError> {
        let fs = FileSystemService::new();
        Self::open_with_encoding_sync(&fs, path, encoding_hint)
    }

    pub fn text(&self) -> String {
        self.buffer.read().to_string()
    }

    /// Notify document that buffer content has changed.
    pub fn mark_edited(&mut self) {
        self.version_id = self.version_id.wrapping_add(1);
        let was_dirty = self.is_dirty;
        self.is_dirty = self.version_id != self.saved_version_id;

        if was_dirty != self.is_dirty {
            self.on_did_change_dirty.fire(self.is_dirty);
        }
    }

    /// Mark document as saved with fresh disk metadata.
    pub fn mark_saved(&mut self, metadata: FileMetadata) {
        self.metadata = Some(metadata.clone());
        self.saved_version_id = self.version_id;
        if self.is_dirty {
            self.is_dirty = false;
            self.on_did_change_dirty.fire(false);
        }
        self.on_did_save.fire(metadata.path);
    }

    /// Check if file on disk has diverged from our saved metadata.
    pub fn check_conflict(
        &self,
        fs: &FileSystemService,
    ) -> Result<Option<FileConflict>, FileSystemError> {
        if let (Some(path), Some(meta)) = (&self.path, &self.metadata) {
            fs.check_conflict(path, meta)
        } else {
            Ok(None)
        }
    }

    /// Save document to disk synchronously with atomic write in `self.encoding`.
    pub fn save_sync(&mut self, fs: &FileSystemService) -> Result<FileMetadata, FileSystemError> {
        self.save_with_encoding_sync(fs, self.encoding)
    }

    /// Save document to disk synchronously, converting to `encoding`.
    pub fn save_with_encoding_sync(
        &mut self,
        fs: &FileSystemService,
        encoding: TextEncoding,
    ) -> Result<FileMetadata, FileSystemError> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?
            .clone();
        let content = self.text();
        let meta = if encoding == TextEncoding::Utf8 {
            fs.write_atomic_sync(&path, &content)?
        } else {
            let bytes = encoding.encode(&content);
            write_atomic_bytes_sync(fs, &path, &bytes, &content)?
        };
        self.encoding = encoding;
        self.mark_saved(meta.clone());
        Ok(meta)
    }

    /// Save document to disk asynchronously with atomic write in `self.encoding`.
    pub async fn save_async(
        &mut self,
        fs: &FileSystemService,
    ) -> Result<FileMetadata, FileSystemError> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?
            .clone();
        let content = self.text();
        let meta = if self.encoding == TextEncoding::Utf8 {
            fs.write_atomic(&path, &content).await?
        } else {
            let bytes = self.encoding.encode(&content);
            write_atomic_bytes_async(fs, &path, &bytes, &content).await?
        };
        self.mark_saved(meta.clone());
        Ok(meta)
    }

    /// Revert document content to disk version synchronously.
    pub fn revert_sync(&mut self, _fs: &FileSystemService) -> Result<(), FileSystemError> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?
            .clone();
        if !path.exists() {
            return Err(FileSystemError::NotFound(path));
        }
        let raw_bytes = std::fs::read(&path)?;
        let (content, encoding) = TextEncoding::decode(&raw_bytes, None)?;
        let meta = FileSystemService::compute_metadata(&path, &content)?;

        *self.buffer.write() = PieceTable::from_str(&content);
        self.encoding = encoding;
        self.metadata = Some(meta);
        self.saved_version_id = self.version_id;
        if self.is_dirty {
            self.is_dirty = false;
            self.on_did_change_dirty.fire(false);
        }
        Ok(())
    }
}

fn write_atomic_bytes_sync(
    fs: &FileSystemService,
    target: &Path,
    bytes: &[u8],
    decoded_content: &str,
) -> Result<FileMetadata, FileSystemError> {
    use std::io::Write;
    let parent = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if parent != Path::new(".") {
        std::fs::create_dir_all(parent)?;
    }
    let file_stem = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document");
    let temp_path = parent.join(format!(".{}.tmp.{}", file_stem, uuid::Uuid::new_v4().simple()));
    let write_res = (|| -> Result<(), std::io::Error> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&temp_path);
        return Err(FileSystemError::Io(e));
    }
    if let Err(e) = std::fs::rename(&temp_path, target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(FileSystemError::Io(e));
    }
    let meta = std::fs::metadata(target)?;
    let metadata = FileMetadata {
        path: target.to_path_buf(),
        mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        size: meta.len(),
        hash: hash_content(decoded_content),
    };
    fs.on_did_change.fire(FileChangeEvent {
        path: target.to_path_buf(),
        change_type: FileChangeType::Changed,
    });
    Ok(metadata)
}

async fn write_atomic_bytes_async(
    fs: &FileSystemService,
    target: &Path,
    bytes: &[u8],
    decoded_content: &str,
) -> Result<FileMetadata, FileSystemError> {
    write_atomic_bytes_sync(fs, target, bytes, decoded_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_dirty_tracking_and_atomic_save() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_doc_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("test_doc.rs");

        let fs = FileSystemService::new();
        let mut doc = TextDocument::new("fn original() {}", Some(path.clone()), Some("rust"));
        assert!(!doc.is_dirty);

        // Edit
        doc.buffer.write().insert(16, "\n// added");
        doc.mark_edited();
        assert!(doc.is_dirty);

        // Save
        let meta = doc.save_sync(&fs).unwrap();
        assert!(!doc.is_dirty);
        assert_eq!(meta.path, path);
        assert_eq!(doc.text(), "fn original() {}\n// added");

        // External conflict test
        std::fs::write(&path, "external change").unwrap();
        let conflict = doc.check_conflict(&fs).unwrap();
        assert!(matches!(conflict, Some(FileConflict::ModifiedOnDisk { .. })));

        // Revert
        doc.revert_sync(&fs).unwrap();
        assert_eq!(doc.text(), "external change");
        assert!(!doc.is_dirty);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_document_map_ext_to_lang_id_all_14_languages() {
        for (filename, expected) in [
            ("main.rs", "rust"),
            ("app.js", "javascript"),
            ("app.jsx", "javascript"),
            ("app.mjs", "javascript"),
            ("app.cjs", "javascript"),
            ("app.ts", "typescript"),
            ("app.tsx", "typescript"),
            ("app.mts", "typescript"),
            ("app.cts", "typescript"),
            ("script.py", "python"),
            ("script.pyw", "python"),
            ("stubs.pyi", "python"),
            ("data.json", "json"),
            ("settings.jsonc", "json"),
            ("README.md", "markdown"),
            ("notes.markdown", "markdown"),
            ("server.go", "go"),
            ("driver.c", "c"),
            ("driver.h", "c"),
            ("widget.cpp", "cpp"),
            ("widget.cc", "cpp"),
            ("widget.cxx", "cpp"),
            ("widget.hpp", "cpp"),
            ("widget.hh", "cpp"),
            ("widget.hxx", "cpp"),
            ("page.html", "html"),
            ("page.htm", "html"),
            ("style.css", "css"),
            ("theme.scss", "css"),
            ("base.less", "css"),
            ("Cargo.toml", "toml"),
            ("deploy.yaml", "yaml"),
            ("deploy.yml", "yaml"),
            ("notes.txt", "plaintext"),
            ("notes.text", "plaintext"),
            ("output.log", "plaintext"),
            ("run.sh", "shellscript"),
            ("init.bash", "shellscript"),
            ("MAIN.RS", "rust"),
            ("NOTES.TXT", "plaintext"),
            ("WIDGET.CPP", "cpp"),
            ("archive.xyz", "plaintext"),
            ("Makefile", "plaintext"),
        ] {
            let doc = TextDocument::new("", Some(PathBuf::from(filename)), None);
            assert_eq!(doc.language_id, expected, "mismatch for {filename}");
        }
        assert_eq!(TextDocument::new("", None, None).language_id, "plaintext");
    }

    #[test]
    fn test_document_encodings_and_binary_rejection_m3_11() {
        let tmp_dir = std::env::temp_dir()
            .join(format!("rustcode_enc_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let fs = FileSystemService::new();

        // 1. UTF-8 with BOM round-trip
        let p_utf8_bom = tmp_dir.join("bom.txt");
        std::fs::write(&p_utf8_bom, b"\xEF\xBB\xBFhello BOM").unwrap();
        let mut doc_bom = TextDocument::open_sync(&fs, &p_utf8_bom).unwrap();
        assert_eq!(doc_bom.encoding, TextEncoding::Utf8Bom);
        assert_eq!(doc_bom.text(), "hello BOM");
        doc_bom.save_sync(&fs).unwrap();
        assert_eq!(std::fs::read(&p_utf8_bom).unwrap(), b"\xEF\xBB\xBFhello BOM");

        // 2. UTF-16LE BOM round-trip
        let p_u16le = tmp_dir.join("u16le.txt");
        let mut le_bytes = vec![0xFF, 0xFE];
        for cu in "hi🦀".encode_utf16() {
            le_bytes.extend_from_slice(&cu.to_le_bytes());
        }
        std::fs::write(&p_u16le, &le_bytes).unwrap();
        let mut doc_le = TextDocument::open_sync(&fs, &p_u16le).unwrap();
        assert_eq!(doc_le.encoding, TextEncoding::Utf16Le);
        assert_eq!(doc_le.text(), "hi🦀");
        doc_le.save_sync(&fs).unwrap();
        assert_eq!(std::fs::read(&p_u16le).unwrap(), le_bytes);

        // 3. UTF-16BE BOM round-trip
        let p_u16be = tmp_dir.join("u16be.txt");
        let mut be_bytes = vec![0xFE, 0xFF];
        for cu in "BE café".encode_utf16() {
            be_bytes.extend_from_slice(&cu.to_be_bytes());
        }
        std::fs::write(&p_u16be, &be_bytes).unwrap();
        let doc_be = TextDocument::open_sync(&fs, &p_u16be).unwrap();
        assert_eq!(doc_be.encoding, TextEncoding::Utf16Be);
        assert_eq!(doc_be.text(), "BE café");

        // 4. Windows-1252 single-byte 0xE9 -> 'é'
        let p_win1252 = tmp_dir.join("win1252.txt");
        std::fs::write(&p_win1252, b"caf\xE9").unwrap();
        let mut doc_1252 = TextDocument::open_sync(&fs, &p_win1252).unwrap();
        assert_eq!(doc_1252.encoding, TextEncoding::Windows1252);
        assert_eq!(doc_1252.text(), "café");
        doc_1252.save_sync(&fs).unwrap();
        assert_eq!(std::fs::read(&p_win1252).unwrap(), b"caf\xE9");

        // 5. Shift-JIS explicit decode + save round-trip
        let p_sjis = tmp_dir.join("sjis.txt");
        let sjis_bytes = TextEncoding::ShiftJis.encode("日本語");
        std::fs::write(&p_sjis, &sjis_bytes).unwrap();
        let mut doc_sjis =
            TextDocument::open_with_encoding_sync(&fs, &p_sjis, Some(TextEncoding::ShiftJis))
                .unwrap();
        assert_eq!(doc_sjis.encoding, TextEncoding::ShiftJis);
        assert_eq!(doc_sjis.text(), "日本語");
        doc_sjis.save_sync(&fs).unwrap();
        assert_eq!(std::fs::read(&p_sjis).unwrap(), sjis_bytes);

        // 6. Binary file with NUL bytes without UTF-16 BOM rejected cleanly
        let p_bin = tmp_dir.join("blob.bin");
        std::fs::write(&p_bin, b"\x7FELF\x00\x01\x02\x00binary").unwrap();
        let err = TextDocument::open_sync(&fs, &p_bin).err().unwrap();
        assert!(matches!(
            err,
            FileSystemError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::InvalidData
        ));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
