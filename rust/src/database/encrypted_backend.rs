use chacha20poly1305::aead::{Aead as _, AeadCore as _, OsRng, Payload};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};

use std::fmt::{self, Debug};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const BLOCK_SIZE: usize = 4096;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const DISK_BLOCK_SIZE: usize = NONCE_LEN + BLOCK_SIZE + TAG_LEN; // 4136
const HEADER_SIZE: usize = 64;
const MAGIC: &[u8; 4] = b"COVE";
const VERSION: u8 = 1;

// header layout (64 bytes):
//   [0..4]   magic "COVE"
//   [4]      version
//   [5..13]  logical_len (u64 LE)
//   [13..64] reserved
const MAGIC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 4;
const LOGICAL_LEN_OFFSET: usize = 5;

static ENCRYPTION_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Set the global encryption key, must be called once before any database is opened.
/// Panics if called more than once with different keys
pub fn set_encryption_key(key: [u8; 32]) {
    ENCRYPTION_KEY.set(key).expect("encryption key already set");
}

/// Get the global encryption key, returns None if not yet set
pub fn encryption_key() -> Option<&'static [u8; 32]> {
    ENCRYPTION_KEY.get()
}

#[cfg(test)]
pub fn set_test_encryption_key() {
    let _ = ENCRYPTION_KEY.set([0xAB; 32]);
}

pub struct EncryptedBackend {
    file: Mutex<File>,
    cipher: XChaCha20Poly1305,
    logical_len: AtomicU64,
}

impl Debug for EncryptedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedBackend")
            .field("logical_len", &self.logical_len.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl EncryptedBackend {
    /// Create a new encrypted database file at `path`
    pub fn create(path: impl AsRef<Path>, key: &[u8; 32]) -> io::Result<Self> {
        use std::io::Write;

        let cipher = XChaCha20Poly1305::new(key.into());

        let mut file =
            OpenOptions::new().read(true).write(true).create_new(true).open(path.as_ref())?;

        let mut header = [0u8; HEADER_SIZE];
        header[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(MAGIC);
        header[VERSION_OFFSET] = VERSION;

        file.write_all(&header)?;
        file.sync_all()?;

        Ok(Self { file: Mutex::new(file), cipher, logical_len: AtomicU64::new(0) })
    }

    /// Open an existing encrypted database file at `path`
    pub fn open(path: impl AsRef<Path>, key: &[u8; 32]) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path.as_ref())?;
        let cipher = XChaCha20Poly1305::new(key.into());

        let header = read_header(&file)?;
        validate_header(&header)?;

        let logical_len = u64::from_le_bytes(
            header[LOGICAL_LEN_OFFSET..LOGICAL_LEN_OFFSET + 8].try_into().unwrap(),
        );

        Ok(Self { file: Mutex::new(file), cipher, logical_len: AtomicU64::new(logical_len) })
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

impl EncryptedBackend {
    fn encrypt_block(&self, block_index: u64, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        debug_assert!(plaintext.len() <= BLOCK_SIZE);

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

    fn read_disk_block(&self, file: &File, block_index: u64) -> io::Result<Vec<u8>> {
        use std::io::{Read as _, Seek as _, SeekFrom};

        let physical_offset = HEADER_SIZE as u64 + block_index * DISK_BLOCK_SIZE as u64;
        let file_len = file.metadata()?.len();

        // beyond the physical file — sparse, return zeros
        if physical_offset + DISK_BLOCK_SIZE as u64 > file_len {
            return Ok(vec![0u8; BLOCK_SIZE]);
        }

        let mut buf = vec![0u8; DISK_BLOCK_SIZE];
        let mut f = file;
        f.seek(SeekFrom::Start(physical_offset))?;
        f.read_exact(&mut buf)?;

        // all-zero disk block means unwritten (sparse)
        if buf.iter().all(|&b| b == 0) {
            return Ok(vec![0u8; BLOCK_SIZE]);
        }

        self.decrypt_block(block_index, &buf)
    }

    fn write_disk_block(&self, file: &File, block_index: u64, plaintext: &[u8]) -> io::Result<()> {
        use std::io::{Seek as _, SeekFrom, Write as _};

        debug_assert!(plaintext.len() == BLOCK_SIZE);

        let encrypted = self.encrypt_block(block_index, plaintext)?;
        let physical_offset = HEADER_SIZE as u64 + block_index * DISK_BLOCK_SIZE as u64;

        let mut f = file;
        f.seek(SeekFrom::Start(physical_offset))?;
        f.write_all(&encrypted)?;
        Ok(())
    }

    fn write_header_logical_len(&self, file: &File) -> io::Result<()> {
        use std::io::{Seek as _, SeekFrom, Write as _};

        let len = self.logical_len.load(Ordering::Acquire);
        let mut f = file;
        f.seek(SeekFrom::Start(LOGICAL_LEN_OFFSET as u64))?;
        f.write_all(&len.to_le_bytes())?;
        Ok(())
    }
}

fn read_header(file: &File) -> io::Result<[u8; HEADER_SIZE]> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    let mut f = file;
    f.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; HEADER_SIZE];
    f.read_exact(&mut header)?;
    Ok(header)
}

fn validate_header(header: &[u8; HEADER_SIZE]) -> io::Result<()> {
    if header[MAGIC_OFFSET..MAGIC_OFFSET + 4] != *MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid magic bytes, not a COVE encrypted database",
        ));
    }

    if header[VERSION_OFFSET] != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported version: {}", header[VERSION_OFFSET]),
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

        let file = self.file.lock().unwrap();
        let mut result = vec![0u8; len];
        let mut bytes_read = 0usize;

        while bytes_read < len {
            let current_offset = offset + bytes_read as u64;
            let block_index = current_offset / BLOCK_SIZE as u64;
            let offset_in_block = (current_offset % BLOCK_SIZE as u64) as usize;

            let plaintext = self.read_disk_block(&file, block_index)?;

            let available = BLOCK_SIZE - offset_in_block;
            let to_copy = available.min(len - bytes_read);
            result[bytes_read..bytes_read + to_copy]
                .copy_from_slice(&plaintext[offset_in_block..offset_in_block + to_copy]);

            bytes_read += to_copy;
        }

        Ok(result)
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        let file = self.file.lock().unwrap();

        if len < self.logical_len.load(Ordering::Acquire) {
            let last_block = if len == 0 { 0 } else { (len - 1) / BLOCK_SIZE as u64 + 1 };
            let physical_len = HEADER_SIZE as u64 + last_block * DISK_BLOCK_SIZE as u64;
            file.set_len(physical_len)?;
        }

        self.logical_len.store(len, Ordering::Release);
        self.write_header_logical_len(&file)?;

        Ok(())
    }

    fn sync_data(&self, eventual: bool) -> Result<(), io::Error> {
        if !eventual {
            let file = self.file.lock().unwrap();
            file.sync_data()?;
        }
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        if data.is_empty() {
            return Ok(());
        }

        let file = self.file.lock().unwrap();
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
                self.read_disk_block(&file, block_index)?
            };

            block_buf.resize(BLOCK_SIZE, 0);

            block_buf[offset_in_block..offset_in_block + to_write_in_block]
                .copy_from_slice(&data[bytes_written..bytes_written + to_write_in_block]);

            self.write_disk_block(&file, block_index, &block_buf)?;
            bytes_written += to_write_in_block;
        }

        let new_end = offset + data.len() as u64;
        let current_len = self.logical_len.load(Ordering::Acquire);
        if new_end > current_len {
            self.logical_len.store(new_end, Ordering::Release);
            self.write_header_logical_len(&file)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use redb::{ReadableTableMetadata as _, StorageBackend as _};
    use tempfile::TempDir;

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
        use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

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
            let file = backend.file.lock().unwrap();
            let mut f = &*file;

            let mut disk_block_0 = vec![0u8; DISK_BLOCK_SIZE];
            let mut disk_block_1 = vec![0u8; DISK_BLOCK_SIZE];

            f.seek(SeekFrom::Start(HEADER_SIZE as u64)).unwrap();
            f.read_exact(&mut disk_block_0).unwrap();
            f.read_exact(&mut disk_block_1).unwrap();

            f.seek(SeekFrom::Start(HEADER_SIZE as u64)).unwrap();
            f.write_all(&disk_block_1).unwrap();
            f.write_all(&disk_block_0).unwrap();
            f.sync_all().unwrap();
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

        // create via the same open/create logic used by database.rs
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

        // reopen via the same open logic
        {
            let backend = EncryptedBackend::open(&path, key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(table_def).unwrap();
            assert_eq!(table.get("key1").unwrap().unwrap().value(), "value1");
            assert_eq!(table.get("key2").unwrap().unwrap().value(), "value2");
        }

        // file on disk has COVE magic header
        assert!(EncryptedBackend::is_encrypted(&path));

        // raw file does not contain plaintext values
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"value1"));
    }

    #[test]
    fn wrong_key_cannot_read() {
        let dir = TempDir::new().unwrap();
        let path = test_path(&dir, "wrong_key.enc");
        let key = test_key();

        let table_def: redb::TableDefinition<&str, &str> = redb::TableDefinition::new("secret");

        {
            let backend = EncryptedBackend::create(&path, &key).unwrap();
            let db = redb::Database::builder().create_with_backend(backend).unwrap();

            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(table_def).unwrap();
                table.insert("secret", "data").unwrap();
            }
            write_txn.commit().unwrap();
        }

        // opening with a different key should fail during redb init
        let wrong_key = [0xFF; 32];
        let backend = EncryptedBackend::open(&path, &wrong_key).unwrap();
        let result = redb::Database::builder().create_with_backend(backend);
        assert!(result.is_err(), "should fail to open database with wrong key");
    }
}
