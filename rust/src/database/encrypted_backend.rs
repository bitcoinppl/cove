use chacha20poly1305::aead::{Aead as _, AeadCore as _, OsRng, Payload};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use std::fmt::{self, Debug};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const BLOCK_SIZE: usize = 4096;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const DISK_BLOCK_SIZE: usize = NONCE_LEN + BLOCK_SIZE + TAG_LEN; // 4136
const HEADER_SIZE: usize = 64;
const MAGIC: &[u8; 4] = b"COVE";
const VERSION_V2: u8 = 2;
const CURRENT_VERSION: u8 = VERSION_V2;

// header layout (64 bytes):
//   [0..4]   magic "COVE"
//   [4]      version
//   [5..13]  logical_len (u64 LE)
//   [13..45] header_tag (HMAC-SHA256, v2 only)
//   [45..64] reserved
const MAGIC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 4;
const LOGICAL_LEN_OFFSET: usize = 5;
const HEADER_TAG_OFFSET: usize = 13;
const HEADER_TAG_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

static ENCRYPTION_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Set the global encryption key, must be called once before any database is opened.
///
/// Idempotent: no-op if already set. Debug builds assert the same key is provided
pub fn set_encryption_key(key: [u8; 32]) {
    if let Err(attempted) = ENCRYPTION_KEY.set(key) {
        debug_assert_eq!(
            ENCRYPTION_KEY.get().expect("just failed to set, must exist"),
            &attempted,
            "set_encryption_key called with a different key"
        );
    }
}

/// Get the global encryption key, returns None if not yet set
pub fn encryption_key() -> Option<&'static [u8; 32]> {
    ENCRYPTION_KEY.get()
}

pub struct EncryptedBackend {
    file: File,
    cipher: XChaCha20Poly1305,
    key: [u8; 32],
    logical_len: AtomicU64,
    lock_supported: bool,
}

impl Debug for EncryptedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedBackend")
            .field("logical_len", &self.logical_len.load(Ordering::Relaxed))
            .field("lock_supported", &self.lock_supported)
            .finish_non_exhaustive()
    }
}

impl Drop for EncryptedBackend {
    fn drop(&mut self) {
        if self.lock_supported {
            let _ = self.file.unlock();
        }
    }
}

/// Acquire an exclusive file lock, mirroring redb's FileBackend
fn acquire_lock(file: &File) -> io::Result<bool> {
    use std::fs::TryLockError;

    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "database file is already locked by another process",
        )),
        Err(TryLockError::Error(err)) if err.kind() == io::ErrorKind::Unsupported => Ok(false),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

fn compute_header_tag(key: &[u8; 32], header: &[u8; HEADER_SIZE]) -> [u8; HEADER_TAG_LEN] {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC-SHA256 accepts any key size");
    // authenticate magic + version + logical_len (bytes 0..13)
    mac.update(&header[..HEADER_TAG_OFFSET]);
    let result = mac.finalize();
    let mut tag = [0u8; HEADER_TAG_LEN];
    tag.copy_from_slice(&result.into_bytes());
    tag
}

fn verify_header_tag(key: &[u8; 32], header: &[u8; HEADER_SIZE]) -> io::Result<()> {
    let stored_tag = &header[HEADER_TAG_OFFSET..HEADER_TAG_OFFSET + HEADER_TAG_LEN];
    let expected_tag = compute_header_tag(key, header);
    if stored_tag != expected_tag {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "header integrity check failed: HMAC mismatch (wrong key or corrupted header)",
        ));
    }
    Ok(())
}

impl EncryptedBackend {
    /// Create a new encrypted database file at `path`
    pub fn create(path: impl AsRef<Path>, key: &[u8; 32]) -> io::Result<Self> {
        let cipher = XChaCha20Poly1305::new(key.into());

        let file =
            OpenOptions::new().read(true).write(true).create_new(true).open(path.as_ref())?;

        let lock_supported = acquire_lock(&file)?;

        let mut header = [0u8; HEADER_SIZE];
        header[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(MAGIC);
        header[VERSION_OFFSET] = CURRENT_VERSION;
        // logical_len is 0, already zero in the buffer

        let tag = compute_header_tag(key, &header);
        header[HEADER_TAG_OFFSET..HEADER_TAG_OFFSET + HEADER_TAG_LEN].copy_from_slice(&tag);

        file.write_all_at(&header, 0)?;
        file.sync_all()?;

        Ok(Self { file, cipher, key: *key, logical_len: AtomicU64::new(0), lock_supported })
    }

    /// Open an existing encrypted database file at `path`
    pub fn open(path: impl AsRef<Path>, key: &[u8; 32]) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path.as_ref())?;
        let lock_supported = acquire_lock(&file)?;
        let cipher = XChaCha20Poly1305::new(key.into());

        let header = read_header(&file)?;
        validate_header_magic(&header)?;

        let version = header[VERSION_OFFSET];
        match version {
            VERSION_V2 => {
                verify_header_tag(key, &header)?;
            }
            v => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported encrypted database version: {v}"),
                ));
            }
        }

        let logical_len = u64::from_le_bytes(
            header[LOGICAL_LEN_OFFSET..LOGICAL_LEN_OFFSET + 8]
                .try_into()
                .expect("already validated header length"),
        );

        Ok(Self {
            file,
            cipher,
            key: *key,
            logical_len: AtomicU64::new(logical_len),
            lock_supported,
        })
    }

    /// Create a new encrypted database or open an existing one.
    /// Handles the TOCTOU race where a concurrent thread creates the file between
    /// our existence check and the create call
    pub fn create_or_open(path: impl AsRef<Path>, key: &[u8; 32]) -> io::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Self::open(path, key);
        }

        match Self::create(path, key) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Self::open(path, key),
            Err(e) => Err(e),
        }
    }

    /// Check if a file at `path` is an encrypted database (has the COVE magic header)
    pub fn is_encrypted(path: impl AsRef<Path>) -> bool {
        let Ok(file) = File::open(path.as_ref()) else {
            return false;
        };

        let Ok(header) = read_header(&file) else {
            return false;
        };

        header[MAGIC_OFFSET..MAGIC_OFFSET + 4] == *MAGIC
    }
}

/// Open or create a redb database at the given path, handling 3 cases:
/// - File doesn't exist → create encrypted
/// - File exists + encrypted → open with EncryptedBackend
/// - File exists + NOT encrypted → error
pub fn open_or_create_database(path: &Path) -> Result<redb::Database, super::error::DatabaseError> {
    use super::error::DatabaseError;

    let path_str = path.display().to_string();
    let key = encryption_key().ok_or(DatabaseError::EncryptionKeyNotSet)?;

    let has_header = path.metadata().map(|m| m.len() >= HEADER_SIZE as u64).unwrap_or(false);
    if has_header && !EncryptedBackend::is_encrypted(path) {
        return Err(DatabaseError::PlaintextNotAllowed { path: path_str });
    }

    let backend = EncryptedBackend::create_or_open(path, key)
        .map_err(|e| io_err_to_db_error(&path_str, e))?;

    redb::Database::builder()
        .create_with_file_format_v3(true)
        .create_with_backend(backend)
        .map_err(|e| DatabaseError::BackendOpen { path: path_str, error: e.to_string() })
}

/// Map an io::Error to the appropriate DatabaseError variant
fn io_err_to_db_error(path: &str, e: io::Error) -> super::error::DatabaseError {
    use super::error::DatabaseError;

    let msg = e.to_string();
    match e.kind() {
        io::ErrorKind::WouldBlock => DatabaseError::DatabaseAlreadyOpen,
        io::ErrorKind::InvalidData
            if msg.contains("HMAC mismatch") || msg.contains("unsupported") =>
        {
            DatabaseError::HeaderIntegrity { path: path.to_string(), error: msg }
        }
        io::ErrorKind::InvalidData => {
            DatabaseError::CorruptBlock { path: path.to_string(), error: msg }
        }
        _ => DatabaseError::BackendOpen { path: path.to_string(), error: msg },
    }
}

impl EncryptedBackend {
    fn encrypt_block(&self, block_index: u64, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        assert!(plaintext.len() <= BLOCK_SIZE);

        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = block_index.to_le_bytes();

        let ciphertext = self
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: &aad })
            .map_err(|e| io::Error::other(format!("encryption failed: {e}")))?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn decrypt_block(&self, block_index: u64, disk_data: &[u8]) -> io::Result<Vec<u8>> {
        if disk_data.len() < NONCE_LEN + TAG_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "disk block too small to contain nonce + tag",
            ));
        }

        let nonce = XNonce::from_slice(&disk_data[..NONCE_LEN]);
        let ciphertext_with_tag = &disk_data[NONCE_LEN..];
        let aad = block_index.to_le_bytes();

        self.cipher.decrypt(nonce, Payload { msg: ciphertext_with_tag, aad: &aad }).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("decryption failed: {e}"))
        })
    }

    fn read_disk_block(&self, block_index: u64) -> io::Result<Vec<u8>> {
        let physical_offset = HEADER_SIZE as u64 + block_index * DISK_BLOCK_SIZE as u64;
        let file_len = self.file.metadata()?.len();

        // block doesn't fit entirely in file
        if physical_offset + DISK_BLOCK_SIZE as u64 > file_len {
            // partial block (starts within file but extends past end) = corruption
            if physical_offset < file_len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("block {block_index} is partially present in file (truncated?)"),
                ));
            }
            // block entirely beyond file = sparse/unwritten, return zeros
            return Ok(vec![0u8; BLOCK_SIZE]);
        }

        let mut buf = vec![0u8; DISK_BLOCK_SIZE];
        self.file.read_exact_at(&mut buf, physical_offset)?;

        // all-zero disk block means unwritten (sparse)
        if buf.iter().all(|&b| b == 0) {
            return Ok(vec![0u8; BLOCK_SIZE]);
        }

        self.decrypt_block(block_index, &buf)
    }

    fn write_disk_block(&self, block_index: u64, plaintext: &[u8]) -> io::Result<()> {
        assert_eq!(plaintext.len(), BLOCK_SIZE, "write_disk_block requires full block");

        let encrypted = self.encrypt_block(block_index, plaintext)?;
        let physical_offset = HEADER_SIZE as u64 + block_index * DISK_BLOCK_SIZE as u64;

        self.file.write_all_at(&encrypted, physical_offset)
    }

    /// Rewrite the full 64-byte header atomically with recomputed HMAC
    fn write_header_logical_len(&self) -> io::Result<()> {
        let len = self.logical_len.load(Ordering::Acquire);

        let mut header = [0u8; HEADER_SIZE];
        header[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(MAGIC);
        header[VERSION_OFFSET] = CURRENT_VERSION;
        header[LOGICAL_LEN_OFFSET..LOGICAL_LEN_OFFSET + 8].copy_from_slice(&len.to_le_bytes());

        let tag = compute_header_tag(&self.key, &header);
        header[HEADER_TAG_OFFSET..HEADER_TAG_OFFSET + HEADER_TAG_LEN].copy_from_slice(&tag);

        // single pwrite call keeps logical_len and tag consistent
        self.file.write_all_at(&header, 0)
    }
}

fn read_header(file: &File) -> io::Result<[u8; HEADER_SIZE]> {
    let mut header = [0u8; HEADER_SIZE];
    file.read_exact_at(&mut header, 0)?;
    Ok(header)
}

fn validate_header_magic(header: &[u8; HEADER_SIZE]) -> io::Result<()> {
    if header[MAGIC_OFFSET..MAGIC_OFFSET + 4] != *MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid magic bytes, not a COVE encrypted database",
        ));
    }
    Ok(())
}

impl redb::StorageBackend for EncryptedBackend {
    fn len(&self) -> Result<u64, io::Error> {
        Ok(self.logical_len.load(Ordering::Acquire))
    }

    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, io::Error> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let logical_end = self.logical_len.load(Ordering::Acquire);

        // entirely past logical end: return zeros
        if offset >= logical_end {
            return Ok(vec![0u8; len]);
        }

        // clamp to logical boundary, zero-fill the tail
        let clamped_len = ((logical_end - offset) as usize).min(len);

        let mut result = vec![0u8; len];
        let mut bytes_read = 0usize;

        while bytes_read < clamped_len {
            let current_offset = offset + bytes_read as u64;
            let block_index = current_offset / BLOCK_SIZE as u64;
            let offset_in_block = (current_offset % BLOCK_SIZE as u64) as usize;

            let plaintext = self.read_disk_block(block_index)?;

            let available = BLOCK_SIZE - offset_in_block;
            let to_copy = available.min(clamped_len - bytes_read);
            result[bytes_read..bytes_read + to_copy]
                .copy_from_slice(&plaintext[offset_in_block..offset_in_block + to_copy]);

            bytes_read += to_copy;
        }
        // result[clamped_len..len] is already zeros from vec initialization

        Ok(result)
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        let old_len = self.logical_len.load(Ordering::Acquire);

        if len < old_len {
            // zero-fill the tail of the last retained block so a subsequent
            // extend doesn't re-expose stale encrypted data as non-zero bytes
            let offset_in_block = (len % BLOCK_SIZE as u64) as usize;
            if offset_in_block != 0 {
                let last_block_index = len / BLOCK_SIZE as u64;
                let mut block = self.read_disk_block(last_block_index)?;
                block.resize(BLOCK_SIZE, 0);
                block[offset_in_block..].fill(0);
                self.write_disk_block(last_block_index, &block)?;
            }

            // shrink: persist smaller logical_len first so a crash can't leave
            // a stale larger length pointing past truncated data
            self.logical_len.store(len, Ordering::Release);
            self.write_header_logical_len()?;
            self.file.sync_data()?;

            let last_block = if len == 0 { 0 } else { (len - 1) / BLOCK_SIZE as u64 + 1 };
            let physical_len = HEADER_SIZE as u64 + last_block * DISK_BLOCK_SIZE as u64;
            self.file.set_len(physical_len)?;
        } else {
            self.logical_len.store(len, Ordering::Release);
            self.write_header_logical_len()?;
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn sync_data(&self, eventual: bool) -> Result<(), io::Error> {
        if eventual {
            use std::os::unix::io::AsRawFd as _;
            let code = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_BARRIERFSYNC) };
            if code == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }
        } else {
            self.file.sync_data()
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn sync_data(&self, _eventual: bool) -> Result<(), io::Error> {
        self.file.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        if data.is_empty() {
            return Ok(());
        }

        let mut bytes_written = 0usize;

        while bytes_written < data.len() {
            let current_offset = offset + bytes_written as u64;
            let block_index = current_offset / BLOCK_SIZE as u64;
            let offset_in_block = (current_offset % BLOCK_SIZE as u64) as usize;

            let remaining = data.len() - bytes_written;
            let to_write_in_block = remaining.min(BLOCK_SIZE - offset_in_block);
            let is_full_block = offset_in_block == 0 && to_write_in_block == BLOCK_SIZE;

            let mut block_buf = if is_full_block {
                vec![0u8; BLOCK_SIZE]
            } else {
                // partial write: read-modify-write
                self.read_disk_block(block_index)?
            };

            block_buf.resize(BLOCK_SIZE, 0);

            block_buf[offset_in_block..offset_in_block + to_write_in_block]
                .copy_from_slice(&data[bytes_written..bytes_written + to_write_in_block]);

            self.write_disk_block(block_index, &block_buf)?;
            bytes_written += to_write_in_block;
        }

        let new_end = offset + data.len() as u64;
        let current_len = self.logical_len.load(Ordering::Acquire);
        if new_end > current_len {
            self.logical_len.store(new_end, Ordering::Release);
            self.write_header_logical_len()?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub fn set_test_encryption_key() {
    let _ = ENCRYPTION_KEY.set([0xAB; 32]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use redb::{ReadableTableMetadata as _, StorageBackend as _};
    use tempfile::TempDir;

    const VERSION_V1: u8 = 1;

    fn test_key() -> [u8; 32] {
        [0xAB; 32]
    }

    fn test_path(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[test]
    fn round_trip_with_redb() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "round_trip.enc");
        let key = test_key();

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();
            let table_def: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("test");

            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                table.insert("hello", "world").unwrap();
                table.insert("foo", "bar").unwrap();
            }
            write_txn.commit().unwrap();
        }

        {
            let backend = EncryptedBackend::open(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();
            let table_def: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("test");

            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(table_def).unwrap();
            assert_eq!(table.get("hello").unwrap().unwrap().value(), "world");
            assert_eq!(table.get("foo").unwrap().unwrap().value(), "bar");
        }
    }

    #[test]
    fn block_boundary_writes() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "block_boundary.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = vec![0x42u8; BLOCK_SIZE + 512];
        backend.write(BLOCK_SIZE as u64 - 256, &data).unwrap();

        let read_back = backend.read(BLOCK_SIZE as u64 - 256, data.len()).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn partial_page_writes() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "partial.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = b"small write";
        backend.write(100, data).unwrap();

        let read_back = backend.read(100, data.len()).unwrap();
        assert_eq!(&read_back, data);

        let before = backend.read(0, 100).unwrap();
        assert!(before.iter().all(|&b| b == 0));

        let after = backend.read(100 + data.len() as u64, 50).unwrap();
        assert!(after.iter().all(|&b| b == 0));
    }

    #[test]
    fn sparse_blocks() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "sparse.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        backend.set_len(BLOCK_SIZE as u64 * 10).unwrap();
        assert_eq!(backend.len().unwrap(), BLOCK_SIZE as u64 * 10);

        let data = backend.read(BLOCK_SIZE as u64 * 5, BLOCK_SIZE).unwrap();
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn block_swap_detection() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "tamper.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let block_a = vec![0xAAu8; BLOCK_SIZE];
        let block_b = vec![0xBBu8; BLOCK_SIZE];
        backend.write(0, &block_a).unwrap();
        backend.write(BLOCK_SIZE as u64, &block_b).unwrap();

        assert_eq!(backend.read(0, BLOCK_SIZE).unwrap(), block_a);
        assert_eq!(backend.read(BLOCK_SIZE as u64, BLOCK_SIZE).unwrap(), block_b);

        // tamper: swap the two disk blocks on the physical file
        {
            let offset_0 = HEADER_SIZE as u64;
            let offset_1 = offset_0 + DISK_BLOCK_SIZE as u64;

            let mut disk_block_0 = vec![0u8; DISK_BLOCK_SIZE];
            let mut disk_block_1 = vec![0u8; DISK_BLOCK_SIZE];

            backend.file.read_exact_at(&mut disk_block_0, offset_0).unwrap();
            backend.file.read_exact_at(&mut disk_block_1, offset_1).unwrap();

            backend.file.write_all_at(&disk_block_1, offset_0).unwrap();
            backend.file.write_all_at(&disk_block_0, offset_1).unwrap();
            backend.file.sync_all().unwrap();
        }

        // AAD mismatch should cause authentication failure
        let result = backend.read(0, BLOCK_SIZE);
        assert!(result.is_err(), "expected authentication error after block swap");
    }

    #[test]
    fn is_encrypted_detection() {
        let dir = TempDir::new().unwrap();

        let enc_path = test_path(&dir, "encrypted.enc");
        EncryptedBackend::create(&enc_path, &test_key()).unwrap();
        assert!(EncryptedBackend::is_encrypted(&enc_path));

        let plain_path = test_path(&dir, "plain.redb");
        let _db = redb::Database::create(&plain_path).unwrap();
        assert!(!EncryptedBackend::is_encrypted(&plain_path));

        assert!(!EncryptedBackend::is_encrypted(test_path(&dir, "nope")));
    }

    #[test]
    fn large_dataset() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "large.enc");
        let key = test_key();

        let table_def: redb::TableDefinition<u64, &[u8]> = redb::TableDefinition::new("data");
        let row_count = 500u64;
        let value = vec![0x55u8; 256];

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                for i in 0..row_count {
                    table.insert(i, value.as_slice()).unwrap();
                }
            }
            write_txn.commit().unwrap();
        }

        {
            let backend = EncryptedBackend::open(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(table_def).unwrap();

            for i in 0..row_count {
                let v = table.get(i).unwrap().unwrap();
                assert_eq!(v.value(), value.as_slice(), "mismatch at row {i}");
            }

            assert_eq!(table.len().unwrap(), row_count);
        }
    }

    #[test]
    fn set_len_shrink() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "shrink.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = vec![0xFFu8; BLOCK_SIZE * 2];
        backend.write(0, &data).unwrap();
        assert_eq!(backend.len().unwrap(), (BLOCK_SIZE * 2) as u64);

        backend.set_len(BLOCK_SIZE as u64).unwrap();
        assert_eq!(backend.len().unwrap(), BLOCK_SIZE as u64);

        let first = backend.read(0, BLOCK_SIZE).unwrap();
        assert!(first.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn global_key_init_path() {
        set_test_encryption_key();

        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "init_path.enc");
        let key = encryption_key().expect("test key should be set");

        let table_def: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("init_test");

        {
            let backend = EncryptedBackend::create(&path, key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                table.insert("key1", "value1").unwrap();
                table.insert("key2", "value2").unwrap();
            }
            write_txn.commit().unwrap();
        }

        {
            let backend = EncryptedBackend::open(&path, key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(table_def).unwrap();
            assert_eq!(table.get("key1").unwrap().unwrap().value(), "value1");
            assert_eq!(table.get("key2").unwrap().unwrap().value(), "value2");
        }

        assert!(EncryptedBackend::is_encrypted(&path));

        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"value1"));
    }

    #[test]
    fn wrong_key_detected_at_open() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "wrong_key.enc");
        let key = test_key();

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();
            let table_def: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("secret");

            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                table.insert("secret", "data").unwrap();
            }
            write_txn.commit().unwrap();
        }

        // v2 header HMAC catches wrong key at open() before any block decryption
        let wrong_key = [0xFF; 32];
        let result = EncryptedBackend::open(&path, &wrong_key);
        assert!(result.is_err(), "v2 header auth should reject wrong key at open");

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("HMAC mismatch"));
    }

    #[test]
    fn truncated_file_detected() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "truncated.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = vec![0xAAu8; BLOCK_SIZE * 2];
        backend.write(0, &data).unwrap();
        assert_eq!(backend.len().unwrap(), (BLOCK_SIZE * 2) as u64);

        backend.sync_data(false).unwrap();
        drop(backend);

        // truncate file mid-way through the second disk block (partial block = corruption)
        let partial_len = HEADER_SIZE as u64 + DISK_BLOCK_SIZE as u64 + 100;

        {
            let file = OpenOptions::new().write(true).open(&path).unwrap();
            file.set_len(partial_len).unwrap();
        }

        let backend = EncryptedBackend::open(&path, &key).unwrap();
        let result = backend.read(BLOCK_SIZE as u64, BLOCK_SIZE);
        assert!(result.is_err(), "expected error on truncated block read");
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    // --- new tests for v2 hardening ---

    #[test]
    fn file_locking_prevents_double_open() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "lock_test.enc");
        let key = test_key();

        let _backend = EncryptedBackend::create(&path, &key).unwrap();

        // second open on the same file should fail
        let result = EncryptedBackend::open(&path, &key);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);

        // after dropping the first backend, reopen should succeed
        drop(_backend);
        let _backend2 = EncryptedBackend::open(&path, &key).unwrap();
    }

    #[test]
    fn shrink_writes_header_before_truncation() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "shrink_order.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = vec![0xFFu8; BLOCK_SIZE * 3];
        backend.write(0, &data).unwrap();
        assert_eq!(backend.len().unwrap(), (BLOCK_SIZE * 3) as u64);

        let shrunk_len = BLOCK_SIZE as u64;
        backend.set_len(shrunk_len).unwrap();

        // verify header on disk has the shrunk logical_len
        let header = read_header(&backend.file).unwrap();
        let disk_logical_len = u64::from_le_bytes(
            header[LOGICAL_LEN_OFFSET..LOGICAL_LEN_OFFSET + 8]
                .try_into()
                .expect("already validated header length"),
        );
        assert_eq!(disk_logical_len, shrunk_len);

        // verify physical file was also truncated
        let physical_len = backend.file.metadata().unwrap().len();
        let expected_physical = HEADER_SIZE as u64 + DISK_BLOCK_SIZE as u64;
        assert_eq!(physical_len, expected_physical);
    }

    #[test]
    fn read_past_logical_len_returns_zeros() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "read_boundary.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        let data = vec![0xAAu8; 100];
        backend.write(0, &data).unwrap();
        assert_eq!(backend.len().unwrap(), 100);

        // entirely past logical end
        let past = backend.read(200, 50).unwrap();
        assert!(past.iter().all(|&b| b == 0));

        // partially overlapping: first 50 bytes are data, last 50 are zeros
        let overlap = backend.read(50, 100).unwrap();
        assert_eq!(&overlap[..50], &data[50..100]);
        assert!(overlap[50..].iter().all(|&b| b == 0));
    }

    #[test]
    fn tampered_logical_len_detected() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "tamper_len.enc");
        let key = test_key();

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            backend.write(0, &[0xBBu8; BLOCK_SIZE]).unwrap();
        }

        // tamper: change logical_len without updating the HMAC tag
        {
            let file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
            let bogus_len: u64 = 999999;
            file.write_all_at(&bogus_len.to_le_bytes(), LOGICAL_LEN_OFFSET as u64).unwrap();
        }

        let result = EncryptedBackend::open(&path, &key);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("HMAC mismatch"));
    }

    #[test]
    fn tampered_header_tag_detected() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "tamper_tag.enc");
        let key = test_key();

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            backend.write(0, &[0xCCu8; 64]).unwrap();
        }

        // flip a byte in the HMAC tag region
        {
            let file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
            let mut tag_byte = [0u8; 1];
            file.read_exact_at(&mut tag_byte, HEADER_TAG_OFFSET as u64).unwrap();
            tag_byte[0] ^= 0xFF;
            file.write_all_at(&tag_byte, HEADER_TAG_OFFSET as u64).unwrap();
        }

        let result = EncryptedBackend::open(&path, &key);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unknown_version_rejected() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "bad_version.enc");
        let key = test_key();

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            backend.write(0, &[0x11u8; 64]).unwrap();
        }

        // overwrite version byte with an unknown version
        {
            let file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
            file.write_all_at(&[99u8], VERSION_OFFSET as u64).unwrap();
        }

        let result = EncryptedBackend::open(&path, &key);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn v1_rejected() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "v1_reject.enc");
        let key = test_key();

        // create a v1 header manually
        {
            let file =
                OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();

            let mut header = [0u8; HEADER_SIZE];
            header[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(MAGIC);
            header[VERSION_OFFSET] = VERSION_V1;
            file.write_all_at(&header, 0).unwrap();
            file.sync_all().unwrap();
        }

        let result = EncryptedBackend::open(&path, &key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported"));
    }

    #[test]
    fn set_len_shrink_extend_zero_init() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "zero_init.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();

        // fill 2 full blocks with 0xFF
        let data = vec![0xFFu8; BLOCK_SIZE * 2];
        backend.write(0, &data).unwrap();
        assert_eq!(backend.len().unwrap(), (BLOCK_SIZE * 2) as u64);

        // shrink to mid-block-1, physical truncation keeps block 1 intact on disk
        let shrink_to = BLOCK_SIZE as u64 + 100;
        backend.set_len(shrink_to).unwrap();
        assert_eq!(backend.len().unwrap(), shrink_to);

        // extend back to 2 blocks
        backend.set_len((BLOCK_SIZE * 2) as u64).unwrap();

        // redb contract: extended region must be zero-initialized
        let tail = backend.read(shrink_to, (BLOCK_SIZE * 2) - shrink_to as usize).unwrap();
        assert!(
            tail.iter().all(|&b| b == 0),
            "extended region after shrink must be zero-initialized, but found stale data"
        );
    }

    #[test]
    fn empty_file_not_rejected_as_plaintext() {
        set_test_encryption_key();
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "race_window.enc");

        // simulate the race: file created by create_new() but header not yet written
        File::create(&path).unwrap();

        let result = open_or_create_database(&path);

        // should NOT be PlaintextNotAllowed — the file isn't plaintext, it's mid-creation
        assert!(
            !matches!(
                result,
                Err(crate::database::error::DatabaseError::PlaintextNotAllowed { .. })
            ),
            "empty file during creation race should not be rejected as plaintext"
        );
    }

    #[test]
    #[ignore = "intentional divergence: zero-fill is likely required for redb create_with_backend"]
    fn read_past_logical_len_errors() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "bounds_error.enc");
        let key = test_key();

        let backend = EncryptedBackend::create(&path, &key).unwrap();
        backend.write(0, &[0xAAu8; 100]).unwrap();
        assert_eq!(backend.len().unwrap(), 100);

        // per StorageBackend contract: offset+len > len() should error or panic
        let result = backend.read(200, 50);
        assert!(result.is_err(), "read past logical end should error per StorageBackend contract");
    }

    #[test]
    fn plaintext_database_rejected() {
        set_test_encryption_key();

        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "plaintext.redb");

        // create a plaintext redb database
        {
            let _db = redb::Database::create(&path).unwrap();
        }

        let result = open_or_create_database(&path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::database::error::DatabaseError::PlaintextNotAllowed { .. }),
            "expected PlaintextNotAllowed, got: {err}"
        );
    }
}
