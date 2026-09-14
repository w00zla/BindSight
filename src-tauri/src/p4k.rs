//! Reader for Star Citizen's `Data.p4k`: a Zip64 archive with CIG's own
//! twists (a custom local-header signature, custom central-directory extra
//! fields, AES-128-CBC encrypted entries, zstd as compression method 100).
//! Only what BindSight needs: parse the central directory once and read
//! single entries by name.
//!
//! Ported from StarBreaker (https://github.com/diogotr7/StarBreaker) by
//! diogotr7, MIT licensed — see THIRD-PARTY-LICENSES.md. The archive layout
//! and CIG's extra-field handling follow its `starbreaker-p4k` crate.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use aes::cipher::{BlockDecryptMut, KeyIvInit};

const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4B50;
const EOCD64_SIGNATURE: u32 = 0x0606_4B50;
const CENTRAL_DIR_SIGNATURE: u32 = 0x0201_4B50;
const LOCAL_FILE_SIGNATURE: u32 = 0x0403_4B50;
/// CIG's local file header signature.
const LOCAL_FILE_CIG_SIGNATURE: u32 = 0x1403_4B50;

const EOCD_SIZE: usize = 22;
const ZIP64_LOCATOR_SIZE: usize = 20;
const EOCD64_SIZE: usize = 56;
const LOCAL_FILE_HEADER_SIZE: usize = 30;
/// A ZIP comment is at most 65535 bytes, so the EOCD starts within this
/// many bytes of the end.
const MAX_COMMENT: usize = 65535;

/// Compression method: stored.
const METHOD_STORED: u16 = 0;
/// Compression method: deflate.
const METHOD_DEFLATE: u16 = 8;
/// Compression method: zstd (CIG uses the registered id 100).
const METHOD_ZSTD: u16 = 100;

/// AES-128-CBC key CIG encrypts P4K entries with.
const AES_KEY: [u8; 16] = [
    0x5E, 0x7A, 0x20, 0x02, 0x30, 0x2E, 0xEB, 0x1A, 0x3B, 0xB6, 0x17, 0xC3, 0x0F, 0xDE, 0x1E, 0x47,
];
const AES_IV: [u8; 16] = [0; 16];

/// One entry of the central directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Path inside the archive with `/` separators (the archive itself is
    /// not consistent).
    pub name: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub compression_method: u16,
    pub encrypted: bool,
    /// Offset of the entry's local file header.
    pub offset: u64,
}

/// An opened archive: the file plus its parsed central directory.
#[derive(Debug)]
pub struct Archive {
    file: File,
    entries: Vec<Entry>,
}

impl Archive {
    /// Open the archive and parse its central directory (the tail of the
    /// file only; a `Data.p4k` is far too large to read whole).
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let entries = read_central_directory(&mut file).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Archive { file, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Look an entry up by its path, either separator, case-insensitive
    /// (the archive names are mixed-case but the game's file system is
    /// not).
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        let wanted = name.replace('\\', "/");
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(&wanted))
    }

    /// Read and decode one entry (decrypt, then decompress).
    pub fn read(&mut self, entry: &Entry) -> Result<Vec<u8>, String> {
        read_entry_data(&mut self.file, entry).map_err(|e| format!("{}: {e}", entry.name))
    }
}

/// Parse the central directory of a Zip / Zip64 archive from a seekable
/// reader.
fn read_central_directory(file: &mut (impl Read + Seek)) -> Result<Vec<Entry>, String> {
    let file_len = file.seek(SeekFrom::End(0)).map_err(|e| format!("seek: {e}"))?;
    let tail_size = (file_len as usize).min(EOCD_SIZE + MAX_COMMENT + EOCD64_SIZE + ZIP64_LOCATOR_SIZE);
    let tail_offset = file_len - tail_size as u64;
    file.seek(SeekFrom::Start(tail_offset)).map_err(|e| format!("seek: {e}"))?;
    let mut tail = vec![0; tail_size];
    file.read_exact(&mut tail).map_err(|e| format!("read archive tail: {e}"))?;

    let (total_entries, cd_offset, cd_size, zip64) = locate_central_directory(&tail, tail_offset)?;
    file.seek(SeekFrom::Start(cd_offset)).map_err(|e| format!("seek: {e}"))?;
    let cd_size = usize::try_from(cd_size).map_err(|_| "central directory too large".to_string())?;
    let mut cd = vec![0; cd_size];
    file.read_exact(&mut cd).map_err(|e| format!("read central directory: {e}"))?;
    parse_entries(&cd, total_entries, zip64)
}

/// Find the central directory from the archive's tail: `(total entries,
/// offset, size, zip64)`. `tail_offset` is where `tail` starts in the file.
fn locate_central_directory(tail: &[u8], tail_offset: u64) -> Result<(u64, u64, u64, bool), String> {
    let eocd_at = rfind_signature(tail, EOCD_SIGNATURE, tail.len().saturating_sub(EOCD_SIZE))
        .ok_or("end of central directory record not found")?;
    let mut r = Cursor::at(tail, eocd_at);
    r.u32()?; // signature
    let disk = r.u16()?;
    let start_disk = r.u16()?;
    let entries_on_disk = r.u16()?;
    let total_entries = r.u16()?;
    let cd_size = r.u32()?;
    let cd_offset = r.u32()?;
    let zip64 = disk == 0xFFFF
        || start_disk == 0xFFFF
        || entries_on_disk == 0xFFFF
        || total_entries == 0xFFFF
        || cd_size == 0xFFFF_FFFF
        || cd_offset == 0xFFFF_FFFF;
    if !zip64 {
        return Ok((total_entries as u64, cd_offset as u64, cd_size as u64, false));
    }

    let locator_at = rfind_signature(tail, ZIP64_LOCATOR_SIGNATURE, eocd_at.saturating_sub(ZIP64_LOCATOR_SIZE))
        .ok_or("zip64 end of central directory locator not found")?;
    let mut r = Cursor::at(tail, locator_at);
    r.u32()?; // signature
    r.u32()?; // disk with the EOCD64
    let eocd64_abs = r.u64()?;
    let eocd64_at = eocd64_abs
        .checked_sub(tail_offset)
        .and_then(|o| usize::try_from(o).ok())
        .ok_or("zip64 end of central directory record outside the archive tail")?;
    let mut r = Cursor::at(tail, eocd64_at);
    if r.u32()? != EOCD64_SIGNATURE {
        return Err("zip64 end of central directory record has a wrong signature".into());
    }
    r.u64()?; // size of record
    r.u16()?; // version made by
    r.u16()?; // version needed
    r.u32()?; // disk number
    r.u32()?; // start disk
    r.u64()?; // entries on disk
    let total_entries = r.u64()?;
    let cd_size = r.u64()?;
    let cd_offset = r.u64()?;
    Ok((total_entries, cd_offset, cd_size, true))
}

/// Search backwards from `from` for the little-endian `signature`.
fn rfind_signature(data: &[u8], signature: u32, from: usize) -> Option<usize> {
    let magic = signature.to_le_bytes();
    if data.len() < 4 {
        return None;
    }
    let from = from.min(data.len() - 4);
    (0..=from).rev().find(|&i| data[i..i + 4] == magic)
}

fn parse_entries(cd: &[u8], total_entries: u64, zip64: bool) -> Result<Vec<Entry>, String> {
    let mut r = Cursor::at(cd, 0);
    let mut entries = Vec::with_capacity(usize::try_from(total_entries).unwrap_or(0).min(1 << 20));
    for i in 0..total_entries {
        entries.push(parse_entry(&mut r, zip64).map_err(|e| format!("central directory entry {i}: {e}"))?);
    }
    Ok(entries)
}

/// One central directory file header plus, on Zip64, CIG's extra fields:
/// `0x0001` (the standard Zip64 sizes), `0x5000`, `0x5002` (holds the
/// encryption flag) and `0x5003` — always in this order, the CIG fields'
/// size counting their own 4-byte tag + size header.
fn parse_entry(r: &mut Cursor, zip64: bool) -> Result<Entry, String> {
    if r.u32()? != CENTRAL_DIR_SIGNATURE {
        return Err("wrong signature".into());
    }
    r.u16()?; // version made by
    r.u16()?; // version needed
    r.u16()?; // flags
    let compression_method = r.u16()?;
    r.u32()?; // last modified
    r.u32()?; // crc32
    let compressed_size32 = r.u32()?;
    let uncompressed_size32 = r.u32()?;
    let name_len = r.u16()? as usize;
    let extra_len = r.u16()? as usize;
    let comment_len = r.u16()? as usize;
    let disk_start = r.u16()?;
    r.u16()?; // internal attributes
    r.u32()?; // external attributes
    let offset32 = r.u32()?;

    let name = String::from_utf8(r.bytes(name_len)?.to_vec())
        .map_err(|_| "entry name is not UTF-8".to_string())?
        .replace('\\', "/");

    let mut compressed_size = compressed_size32 as u64;
    let mut uncompressed_size = uncompressed_size32 as u64;
    let mut offset = offset32 as u64;
    let mut encrypted = false;
    if zip64 {
        let mut x = Cursor::at(r.bytes(extra_len)?, 0);
        expect_tag(&mut x, 0x0001)?;
        x.u16()?; // data size
        if uncompressed_size32 == 0xFFFF_FFFF {
            uncompressed_size = x.u64()?;
        }
        if compressed_size32 == 0xFFFF_FFFF {
            compressed_size = x.u64()?;
        }
        if offset32 == 0xFFFF_FFFF {
            offset = x.u64()?;
        }
        if disk_start == 0xFFFF {
            x.u32()?;
        }
        expect_tag(&mut x, 0x5000)?;
        let size = x.u16()? as usize;
        x.skip(size.saturating_sub(4))?;
        expect_tag(&mut x, 0x5002)?;
        if x.u16()? != 6 {
            return Err("extra field 0x5002 has an unexpected size".into());
        }
        encrypted = x.u16()? == 1;
        expect_tag(&mut x, 0x5003)?;
        let size = x.u16()? as usize;
        x.skip(size.saturating_sub(4))?;
    } else {
        r.skip(extra_len)?;
    }
    r.skip(comment_len)?;

    Ok(Entry { name, compressed_size, uncompressed_size, compression_method, encrypted, offset })
}

fn expect_tag(x: &mut Cursor, tag: u16) -> Result<(), String> {
    let got = x.u16()?;
    if got != tag {
        return Err(format!("extra field {tag:#06x} expected, got {got:#06x}"));
    }
    Ok(())
}

/// Read the entry's raw bytes behind its local file header and decode them.
fn read_entry_data(file: &mut (impl Read + Seek), entry: &Entry) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(entry.offset)).map_err(|e| format!("seek: {e}"))?;
    let mut header = [0; LOCAL_FILE_HEADER_SIZE];
    file.read_exact(&mut header).map_err(|e| format!("read local header: {e}"))?;
    let mut r = Cursor::at(&header, 0);
    let signature = r.u32()?;
    if signature != LOCAL_FILE_SIGNATURE && signature != LOCAL_FILE_CIG_SIGNATURE {
        return Err(format!("local header has a wrong signature {signature:#010x}"));
    }
    r.skip(22)?; // version .. uncompressed size
    let name_len = r.u16()? as u64;
    let extra_len = r.u16()? as u64;
    file.seek(SeekFrom::Current((name_len + extra_len) as i64)).map_err(|e| format!("seek: {e}"))?;
    let size = usize::try_from(entry.compressed_size).map_err(|_| "entry too large".to_string())?;
    let mut raw = vec![0; size];
    file.read_exact(&mut raw).map_err(|e| format!("read data: {e}"))?;
    decode(raw, entry)
}

/// Decrypt (if flagged) and decompress an entry's raw bytes.
fn decode(raw: Vec<u8>, entry: &Entry) -> Result<Vec<u8>, String> {
    let hint = usize::try_from(entry.uncompressed_size).unwrap_or(0);
    match (entry.encrypted, entry.compression_method) {
        (true, METHOD_ZSTD) => zstd_decompress(&decrypt(raw)?, hint),
        (true, method) => Err(format!("encrypted entry with compression method {method}")),
        (false, METHOD_ZSTD) => zstd_decompress(&raw, hint),
        (false, METHOD_DEFLATE) => deflate_decompress(&raw, hint),
        (false, METHOD_STORED) => Ok(raw),
        (false, method) => Err(format!("unsupported compression method {method}")),
    }
}

/// AES-128-CBC with CIG's key, zero IV and zero padding (trailing zero
/// bytes are trimmed, not PKCS#7).
fn decrypt(mut buf: Vec<u8>) -> Result<Vec<u8>, String> {
    if buf.is_empty() {
        return Ok(buf);
    }
    let dec = cbc::Decryptor::<aes::Aes128>::new(&AES_KEY.into(), &AES_IV.into());
    dec.decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
        .map_err(|e| format!("decrypt: {e}"))?;
    let end = buf.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    buf.truncate(end);
    Ok(buf)
}

fn zstd_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(std::io::Cursor::new(data)).map_err(|e| format!("zstd: {e}"))?;
    let mut out = Vec::with_capacity(size_hint);
    decoder.read_to_end(&mut out).map_err(|e| format!("zstd: {e}"))?;
    Ok(out)
}

fn deflate_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::read::DeflateDecoder::new(std::io::Cursor::new(data));
    let mut out = Vec::with_capacity(size_hint);
    decoder.read_to_end(&mut out).map_err(|e| format!("deflate: {e}"))?;
    Ok(out)
}

/// Bounds-checked little-endian reads over a byte slice.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Cursor { data, pos }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&end| end <= self.data.len()).ok_or_else(|| {
            format!("truncated at offset {}: need {n} bytes, have {}", self.pos, self.data.len() - self.pos)
        })?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn skip(&mut self, n: usize) -> Result<(), String> {
        self.bytes(n).map(|_| ())
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        self.bytes(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        self.bytes(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, String> {
        self.u32().map(|v| v as i32)
    }

    pub fn u64(&mut self) -> Result<u64, String> {
        self.bytes(8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    /// One file to put into a test archive.
    struct TestFile {
        name: &'static str,
        method: u16,
        encrypted: bool,
        /// The bytes as stored (already compressed / encrypted).
        stored: Vec<u8>,
        uncompressed_size: u64,
        /// Write the sizes and offset as `0xFFFFFFFF` + Zip64 extra data.
        wide: bool,
    }

    fn le16(v: u16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn le32(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn le64(v: u64) -> [u8; 8] {
        v.to_le_bytes()
    }

    /// Build an archive the way CIG lays one out: CIG local headers, a
    /// Zip64 central directory with the four extra fields, locator + EOCD.
    /// With `zip64 == false` a plain Zip32 directory without extra fields.
    fn build_archive(files: &[TestFile], zip64: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for f in files {
            offsets.push(out.len() as u64);
            out.extend_from_slice(&le32(LOCAL_FILE_CIG_SIGNATURE));
            out.extend_from_slice(&le16(20)); // version needed
            out.extend_from_slice(&le16(0)); // flags
            out.extend_from_slice(&le16(f.method));
            out.extend_from_slice(&le16(0)); // time
            out.extend_from_slice(&le16(0)); // date
            out.extend_from_slice(&le32(0)); // crc
            out.extend_from_slice(&le32(f.stored.len() as u32));
            out.extend_from_slice(&le32(f.uncompressed_size as u32));
            out.extend_from_slice(&le16(f.name.len() as u16));
            out.extend_from_slice(&le16(4)); // extra: 4 junk bytes
            out.extend_from_slice(f.name.as_bytes());
            out.extend_from_slice(b"junk");
            out.extend_from_slice(&f.stored);
        }
        let cd_offset = out.len() as u64;
        for (f, &offset) in files.iter().zip(&offsets) {
            let mut extra = Vec::new();
            if zip64 {
                extra.extend_from_slice(&le16(0x0001));
                if f.wide {
                    extra.extend_from_slice(&le16(24));
                    extra.extend_from_slice(&le64(f.uncompressed_size));
                    extra.extend_from_slice(&le64(f.stored.len() as u64));
                    extra.extend_from_slice(&le64(offset));
                } else {
                    extra.extend_from_slice(&le16(0));
                }
                extra.extend_from_slice(&le16(0x5000));
                extra.extend_from_slice(&le16(8)); // 4 header + 4 data
                extra.extend_from_slice(b"cig1");
                extra.extend_from_slice(&le16(0x5002));
                extra.extend_from_slice(&le16(6));
                extra.extend_from_slice(&le16(f.encrypted as u16));
                extra.extend_from_slice(&le16(0x5003));
                extra.extend_from_slice(&le16(6)); // 4 header + 2 data
                extra.extend_from_slice(b"xx");
            }
            let wide = zip64 && f.wide;
            out.extend_from_slice(&le32(CENTRAL_DIR_SIGNATURE));
            out.extend_from_slice(&le16(45)); // version made by
            out.extend_from_slice(&le16(45)); // version needed
            out.extend_from_slice(&le16(0)); // flags
            out.extend_from_slice(&le16(f.method));
            out.extend_from_slice(&le32(0)); // last modified
            out.extend_from_slice(&le32(0)); // crc
            out.extend_from_slice(&le32(if wide { 0xFFFF_FFFF } else { f.stored.len() as u32 }));
            out.extend_from_slice(&le32(if wide { 0xFFFF_FFFF } else { f.uncompressed_size as u32 }));
            out.extend_from_slice(&le16(f.name.len() as u16));
            out.extend_from_slice(&le16(extra.len() as u16));
            out.extend_from_slice(&le16(3)); // comment length
            out.extend_from_slice(&le16(0)); // disk
            out.extend_from_slice(&le16(0)); // internal attributes
            out.extend_from_slice(&le32(0)); // external attributes
            out.extend_from_slice(&le32(if wide { 0xFFFF_FFFF } else { offset as u32 }));
            out.extend_from_slice(f.name.as_bytes());
            out.extend_from_slice(&extra);
            out.extend_from_slice(b"cmt");
        }
        let cd_size = out.len() as u64 - cd_offset;
        if zip64 {
            let eocd64_offset = out.len() as u64;
            out.extend_from_slice(&le32(EOCD64_SIGNATURE));
            out.extend_from_slice(&le64(44)); // size of record
            out.extend_from_slice(&le16(45));
            out.extend_from_slice(&le16(45));
            out.extend_from_slice(&le32(0));
            out.extend_from_slice(&le32(0));
            out.extend_from_slice(&le64(files.len() as u64));
            out.extend_from_slice(&le64(files.len() as u64));
            out.extend_from_slice(&le64(cd_size));
            out.extend_from_slice(&le64(cd_offset));
            out.extend_from_slice(&le32(ZIP64_LOCATOR_SIGNATURE));
            out.extend_from_slice(&le32(0));
            out.extend_from_slice(&le64(eocd64_offset));
            out.extend_from_slice(&le32(1));
        }
        out.extend_from_slice(&le32(EOCD_SIGNATURE));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(if zip64 { 0xFFFF } else { files.len() as u16 }));
        out.extend_from_slice(&le16(if zip64 { 0xFFFF } else { files.len() as u16 }));
        out.extend_from_slice(&le32(if zip64 { 0xFFFF_FFFF } else { cd_size as u32 }));
        out.extend_from_slice(&le32(if zip64 { 0xFFFF_FFFF } else { cd_offset as u32 }));
        out.extend_from_slice(&le16(7)); // comment
        out.extend_from_slice(b"comment");
        out
    }

    /// A zstd frame holding `data` in one raw block (no compression).
    fn zstd_raw_frame(data: &[u8]) -> Vec<u8> {
        assert!(data.len() < 256);
        let mut out = vec![0x28, 0xB5, 0x2F, 0xFD];
        out.push(0x20); // single segment, 1-byte frame content size, no checksum
        out.push(data.len() as u8);
        let header = 1 | ((data.len() as u32) << 3); // last block, raw
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(data);
        out
    }

    /// Encrypt like CIG: AES-128-CBC, zero IV, zero padding to 16 bytes.
    fn encrypt(data: &[u8]) -> Vec<u8> {
        let mut buf = data.to_vec();
        buf.resize(data.len().div_ceil(16) * 16, 0);
        let enc = cbc::Encryptor::<aes::Aes128>::new(&AES_KEY.into(), &AES_IV.into());
        let len = buf.len();
        enc.encrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf, len).unwrap();
        buf
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn archive_from(bytes: &[u8]) -> Archive {
        let path = std::env::temp_dir().join(format!("bindsight-p4k-{}.p4k", uuid::Uuid::new_v4()));
        std::fs::write(&path, bytes).unwrap();
        let archive = Archive::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        archive
    }

    fn cig_files() -> Vec<TestFile> {
        let secret = b"<profile version=\"1\"/>";
        vec![
            TestFile {
                name: "Data\\Libs\\Config\\defaultProfile.xml",
                method: METHOD_ZSTD,
                encrypted: true,
                stored: encrypt(&zstd_raw_frame(secret)),
                uncompressed_size: secret.len() as u64,
                wide: true,
            },
            TestFile {
                name: "Data/Localization/english/global.ini",
                method: METHOD_ZSTD,
                encrypted: false,
                stored: zstd_raw_frame(b"key=value\n"),
                uncompressed_size: 10,
                wide: false,
            },
            TestFile {
                name: "Data/stored.txt",
                method: METHOD_STORED,
                encrypted: false,
                stored: b"plain".to_vec(),
                uncompressed_size: 5,
                wide: true,
            },
            TestFile {
                name: "Data/deflated.txt",
                method: METHOD_DEFLATE,
                encrypted: false,
                stored: deflate(b"deflate me, deflate me, deflate me"),
                uncompressed_size: 34,
                wide: false,
            },
        ]
    }

    #[test]
    fn zip64_directory_with_cig_extras_parses() {
        let archive = archive_from(&build_archive(&cig_files(), true));
        let names: Vec<_> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Data/Libs/Config/defaultProfile.xml",
                "Data/Localization/english/global.ini",
                "Data/stored.txt",
                "Data/deflated.txt"
            ]
        );
        let profile = archive.entry("Data/Libs/Config/defaultProfile.xml").unwrap();
        assert!(profile.encrypted);
        assert_eq!(profile.compression_method, METHOD_ZSTD);
        assert_eq!(profile.uncompressed_size, 22);
        assert_eq!(profile.offset, 0);
        let ini = archive.entry("Data/Localization/english/global.ini").unwrap();
        assert!(!ini.encrypted);
        assert!(ini.offset > 0);
        let stored = archive.entry("Data/stored.txt").unwrap();
        assert_eq!(stored.compressed_size, 5);
    }

    #[test]
    fn entries_decode_every_method() {
        let mut archive = archive_from(&build_archive(&cig_files(), true));
        let read = |a: &mut Archive, name: &str| {
            let e = a.entry(name).unwrap().clone();
            a.read(&e).unwrap()
        };
        assert_eq!(read(&mut archive, "Data/Libs/Config/defaultProfile.xml"), b"<profile version=\"1\"/>");
        assert_eq!(read(&mut archive, "Data/Localization/english/global.ini"), b"key=value\n");
        assert_eq!(read(&mut archive, "Data/stored.txt"), b"plain");
        assert_eq!(read(&mut archive, "Data/deflated.txt"), b"deflate me, deflate me, deflate me");
    }

    #[test]
    fn lookup_ignores_case_and_separator() {
        let archive = archive_from(&build_archive(&cig_files(), true));
        assert!(archive.entry("data\\libs\\config\\DEFAULTPROFILE.XML").is_some());
        assert!(archive.entry("Data/Libs/Config/other.xml").is_none());
    }

    #[test]
    fn plain_zip32_directory_parses() {
        let files = vec![TestFile {
            name: "a.txt",
            method: METHOD_STORED,
            encrypted: false,
            stored: b"abc".to_vec(),
            uncompressed_size: 3,
            wide: false,
        }];
        let mut archive = archive_from(&build_archive(&files, false));
        assert_eq!(archive.entries().len(), 1);
        let e = archive.entry("a.txt").unwrap().clone();
        assert_eq!(archive.read(&e).unwrap(), b"abc");
    }

    #[test]
    fn encrypted_non_zstd_and_unknown_methods_are_errors() {
        let files = vec![
            TestFile {
                name: "enc-stored",
                method: METHOD_STORED,
                encrypted: true,
                stored: encrypt(b"x"),
                uncompressed_size: 1,
                wide: false,
            },
            TestFile {
                name: "lzma",
                method: 14,
                encrypted: false,
                stored: b"x".to_vec(),
                uncompressed_size: 1,
                wide: false,
            },
        ];
        let mut archive = archive_from(&build_archive(&files, true));
        let e = archive.entry("enc-stored").unwrap().clone();
        assert!(archive.read(&e).unwrap_err().contains("encrypted entry with compression method 0"));
        let e = archive.entry("lzma").unwrap().clone();
        assert!(archive.read(&e).unwrap_err().contains("unsupported compression method 14"));
    }

    #[test]
    fn broken_archives_are_errors_not_panics() {
        let path = std::env::temp_dir().join(format!("bindsight-p4k-{}.p4k", uuid::Uuid::new_v4()));
        for bytes in [&b""[..], b"PK", b"not an archive at all, just text", &[0; 100]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(Archive::open(&path).unwrap_err().contains("end of central directory"));
        }
        // A directory that claims more entries than it holds: the EOCD64's
        // total entries (the file ends with EOCD64, locator, EOCD + comment).
        let mut good = build_archive(&cig_files(), true);
        let eocd64 = good.len() - (EOCD_SIZE + 7) - ZIP64_LOCATOR_SIZE - EOCD64_SIZE;
        assert_eq!(&good[eocd64..eocd64 + 4], &le32(EOCD64_SIGNATURE));
        good[eocd64 + 32..eocd64 + 40].copy_from_slice(&le64(9));
        std::fs::write(&path, &good).unwrap();
        assert!(Archive::open(&path).unwrap_err().contains("central directory entry 4"));
        // A truncated local header / data.
        let mut short = build_archive(&cig_files(), true);
        let cd = archive_from(&short).entries().to_vec();
        short.truncate(4);
        std::fs::write(&path, &short).unwrap();
        assert!(Archive::open(&path).is_err());
        let mut archive = archive_from(&build_archive(&cig_files(), true));
        let mut e = cd[0].clone();
        e.compressed_size = 1 << 20;
        assert!(archive.read(&e).unwrap_err().contains("read data"));
        e.offset = 1 << 30;
        assert!(archive.read(&e).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_archive_is_an_error() {
        let err = Archive::open(Path::new("/nonexistent/Data.p4k")).unwrap_err();
        assert!(err.contains("Data.p4k"), "{err}");
    }

    #[test]
    fn decrypt_trims_zero_padding_and_rejects_odd_lengths() {
        assert_eq!(decrypt(encrypt(b"hello")).unwrap(), b"hello");
        assert_eq!(decrypt(encrypt(b"")).unwrap(), b"");
        assert!(decrypt(vec![1; 17]).unwrap_err().contains("decrypt"));
    }

    #[test]
    fn cursor_is_bounds_checked() {
        let mut c = Cursor::at(&[1, 0, 2, 0, 0, 0], 0);
        assert_eq!(c.u16().unwrap(), 1);
        assert_eq!(c.u32().unwrap(), 2);
        assert_eq!(c.position(), 6);
        assert!(c.u16().unwrap_err().contains("truncated at offset 6"));
        let mut c = Cursor::at(&[0; 4], 2);
        assert!(c.bytes(usize::MAX).is_err());
        assert!(c.skip(2).is_ok());
        assert!(c.skip(1).is_err());
    }
}
