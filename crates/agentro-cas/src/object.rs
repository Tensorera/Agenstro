use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::ObjectDigest;

/// Hard maximum object bytes accepted by this initial CAS implementation.
pub const MAX_OBJECT_BYTES: u64 = 1 << 40;
const COPY_BUFFER_BYTES: usize = 64 * 1_024;

/// Per-write object byte budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PutBudget {
    max_bytes: u64,
}

impl PutBudget {
    /// Constructs a non-zero object budget under the hard maximum.
    ///
    /// # Errors
    ///
    /// Returns [`CasError::InvalidBudget`] for zero or excessive values.
    pub fn new(max_bytes: u64) -> Result<Self, CasError> {
        if max_bytes == 0 || max_bytes > MAX_OBJECT_BYTES {
            return Err(CasError::InvalidBudget {
                maximum: MAX_OBJECT_BYTES,
            });
        }
        Ok(Self { max_bytes })
    }
}

/// Whether this call created the immutable object or found a valid duplicate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PublishOutcome {
    /// A new immutable object was atomically published.
    Created,
    /// An object with the same digest and length already existed.
    AlreadyPresent,
    /// A corrupt object was quarantined before publishing the replacement.
    ReplacedCorrupt,
}

/// Durability level actually available after atomic publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PublishDurability {
    /// Both file contents and the containing directory were synchronized.
    FileAndDirectory,
    /// File contents were synchronized, but portable directory sync is absent.
    FileOnly,
}

/// Result of one bounded streaming object write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectPut {
    digest: ObjectDigest,
    length: u64,
    outcome: PublishOutcome,
    durability: PublishDurability,
}

impl ObjectPut {
    /// Returns the SHA-256 content identity.
    #[must_use]
    pub fn digest(self) -> ObjectDigest {
        self.digest
    }

    /// Returns the streamed object length.
    #[must_use]
    pub fn length(self) -> u64 {
        self.length
    }

    /// Returns whether publication created, reused, or repaired an object.
    #[must_use]
    pub fn outcome(self) -> PublishOutcome {
        self.outcome
    }

    /// Returns the synchronization level achieved on this platform.
    #[must_use]
    pub fn durability(self) -> PublishDurability {
        self.durability
    }
}

/// Bounded CAS input, integrity, or filesystem failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CasError {
    /// CAS roots must be explicit absolute paths.
    #[error("CAS root must be absolute")]
    RootNotAbsolute,
    /// An object budget was zero or above the hard maximum.
    #[error("object budget must be non-zero and no greater than {maximum}")]
    InvalidBudget {
        /// Hard maximum object bytes.
        maximum: u64,
    },
    /// The input stream exceeded its declared byte budget.
    #[error("object exceeded byte budget {maximum}")]
    BudgetExceeded {
        /// Declared maximum object bytes.
        maximum: u64,
    },
    /// A CAS filesystem operation failed.
    #[error("CAS filesystem operation failed during {operation}")]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// An existing digest path had the wrong type, length, or content digest.
    #[error("existing CAS object is corrupt for digest {digest}")]
    ExistingObjectCorrupt {
        /// Digest whose immutable object was invalid.
        digest: ObjectDigest,
    },
    /// A corrupt object could not be isolated without overwriting prior evidence.
    #[error("could not reserve quarantine for digest {digest}")]
    QuarantineUnavailable {
        /// Digest whose quarantine slot was unavailable.
        digest: ObjectDigest,
    },
}

/// One service-private SHA-256 content-addressed namespace.
#[derive(Debug)]
pub struct Cas {
    root: PathBuf,
    objects: PathBuf,
    temporary: PathBuf,
    quarantine: PathBuf,
}

impl Cas {
    /// Creates or opens a service-private CAS directory structure.
    ///
    /// # Errors
    ///
    /// Returns a typed root or filesystem error.
    pub fn open(root: PathBuf) -> Result<Self, CasError> {
        if !root.is_absolute() {
            return Err(CasError::RootNotAbsolute);
        }
        let objects = root.join("objects").join("sha256");
        let temporary = root.join("objects").join(".tmp");
        let quarantine = root.join("objects").join("quarantine");
        create_directory(&objects)?;
        create_directory(&temporary)?;
        create_directory(&quarantine)?;
        Ok(Self {
            root,
            objects,
            temporary,
            quarantine,
        })
    }

    /// Returns the namespace root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Streams, hashes, synchronizes, and atomically publishes one object.
    ///
    /// The reader is consumed in fixed-size chunks; this API never requires an
    /// object-sized `Vec<u8>`.
    ///
    /// # Errors
    ///
    /// Returns typed budget, stream, filesystem, or existing-object integrity
    /// errors. Failed writes leave no published partial object.
    pub fn put<R: Read>(&self, mut reader: R, budget: PutBudget) -> Result<ObjectPut, CasError> {
        let mut temporary = NamedTempFile::new_in(&self.temporary)
            .map_err(|source| io_error("create temporary object", source))?;
        let mut hasher = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; COPY_BUFFER_BYTES];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|source| io_error("read object stream", source))?;
            if read == 0 {
                break;
            }
            let read_u64 = u64::try_from(read).unwrap_or(u64::MAX);
            length = length
                .checked_add(read_u64)
                .ok_or(CasError::BudgetExceeded {
                    maximum: budget.max_bytes,
                })?;
            if length > budget.max_bytes {
                return Err(CasError::BudgetExceeded {
                    maximum: budget.max_bytes,
                });
            }
            hasher.update(&buffer[..read]);
            temporary
                .write_all(&buffer[..read])
                .map_err(|source| io_error("write temporary object", source))?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| io_error("synchronize temporary object", source))?;

        let digest_bytes: [u8; 32] = hasher.finalize().into();
        let digest = ObjectDigest::from_bytes(digest_bytes);
        let destination = self.object_path(digest);
        let parent = destination.parent().ok_or_else(|| {
            io_error(
                "resolve object directory",
                io::Error::other("digest path has no parent"),
            )
        })?;
        create_directory(parent)?;
        let (outcome, published_file) = self.publish(temporary, &destination, digest, length)?;
        if let Some(published_file) = published_file {
            published_file
                .sync_all()
                .map_err(|source| io_error("synchronize published object", source))?;
        }
        let durability = sync_publish_directories(parent, &self.objects)?;
        Ok(ObjectPut {
            digest,
            length,
            outcome,
            durability,
        })
    }

    /// Opens an immutable object after verifying its expected length and digest.
    ///
    /// # Errors
    ///
    /// Returns a typed filesystem or corruption error.
    pub fn open_object(
        &self,
        digest: ObjectDigest,
        expected_length: u64,
    ) -> Result<File, CasError> {
        let path = self.object_path(digest);
        verify_existing(&path, digest, expected_length)?;
        File::open(path).map_err(|source| io_error("open object", source))
    }

    /// Returns the internal immutable path for a digest.
    ///
    /// The path is suitable for diagnostics and service-internal file access;
    /// callers must not mutate it.
    #[must_use]
    pub fn object_path(&self, digest: ObjectDigest) -> PathBuf {
        let encoded = digest.to_hex();
        self.objects.join(&encoded[..2]).join(encoded)
    }

    fn publish(
        &self,
        temporary: NamedTempFile,
        destination: &Path,
        digest: ObjectDigest,
        length: u64,
    ) -> Result<(PublishOutcome, Option<File>), CasError> {
        if destination.exists() {
            match verify_existing(destination, digest, length) {
                Ok(()) => {
                    return Ok((PublishOutcome::AlreadyPresent, None));
                }
                Err(CasError::ExistingObjectCorrupt { .. }) => {
                    self.quarantine(destination, digest)?;
                    return persist_new(temporary, destination, PublishOutcome::ReplacedCorrupt);
                }
                Err(error) => return Err(error),
            }
        }

        match temporary.persist_noclobber(destination) {
            Ok(file) => Ok((PublishOutcome::Created, Some(file))),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let temporary = error.file;
                match verify_existing(destination, digest, length) {
                    Ok(()) => Ok((PublishOutcome::AlreadyPresent, None)),
                    Err(CasError::ExistingObjectCorrupt { .. }) => {
                        self.quarantine(destination, digest)?;
                        persist_new(temporary, destination, PublishOutcome::ReplacedCorrupt)
                    }
                    Err(other) => Err(other),
                }
            }
            Err(error) => Err(io_error("publish object", error.error)),
        }
    }

    fn quarantine(&self, object: &Path, digest: ObjectDigest) -> Result<(), CasError> {
        let slot = self.quarantine.join(digest.to_hex());
        match fs::create_dir(&slot) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(CasError::QuarantineUnavailable { digest });
            }
            Err(source) => return Err(io_error("reserve quarantine", source)),
        }
        fs::rename(object, slot.join("object")).map_err(|source| {
            let _ = fs::remove_dir(&slot);
            io_error("quarantine corrupt object", source)
        })
    }
}

fn persist_new(
    temporary: NamedTempFile,
    destination: &Path,
    outcome: PublishOutcome,
) -> Result<(PublishOutcome, Option<File>), CasError> {
    temporary
        .persist_noclobber(destination)
        .map(|file| (outcome, Some(file)))
        .map_err(|error| io_error("publish replacement object", error.error))
}

fn verify_existing(
    path: &Path,
    digest: ObjectDigest,
    expected_length: u64,
) -> Result<(), CasError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error("inspect existing object", source))?;
    if !metadata.file_type().is_file() || metadata.len() != expected_length {
        return Err(CasError::ExistingObjectCorrupt { digest });
    }

    let mut object = File::open(path).map_err(|source| io_error("open existing object", source))?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = object
            .read(&mut buffer)
            .map_err(|source| io_error("read existing object", source))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or(CasError::ExistingObjectCorrupt { digest })?;
        if length > expected_length {
            return Err(CasError::ExistingObjectCorrupt { digest });
        }
        hasher.update(&buffer[..read]);
    }

    let actual_digest = ObjectDigest::from_bytes(hasher.finalize().into());
    if length == expected_length && actual_digest == digest {
        Ok(())
    } else {
        Err(CasError::ExistingObjectCorrupt { digest })
    }
}

fn create_directory(path: &Path) -> Result<(), CasError> {
    fs::create_dir_all(path).map_err(|source| io_error("create CAS directory", source))
}

fn io_error(operation: &'static str, source: io::Error) -> CasError {
    CasError::Io { operation, source }
}

#[cfg(unix)]
fn sync_publish_directories(
    shard: &Path,
    shard_parent: &Path,
) -> Result<PublishDurability, CasError> {
    for path in [shard, shard_parent] {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error("synchronize object directory", source))?;
    }
    Ok(PublishDurability::FileAndDirectory)
}

#[cfg(not(unix))]
fn sync_publish_directories(
    _shard: &Path,
    _shard_parent: &Path,
) -> Result<PublishDurability, CasError> {
    Ok(PublishDurability::FileOnly)
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, io, io::Read};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn stream_publish_is_deduplicated_and_readable() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let cas = Cas::open(temporary.path().join("cas"))?;
        let first = cas.put(&b"hello world"[..], PutBudget::new(64)?)?;
        let second = cas.put(&b"hello world"[..], PutBudget::new(64)?)?;

        assert_eq!(
            first.digest().to_hex(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert_eq!(first.length(), 11);
        assert_eq!(first.outcome(), PublishOutcome::Created);
        assert_eq!(second.outcome(), PublishOutcome::AlreadyPresent);
        let mut contents = Vec::new();
        cas.open_object(first.digest(), first.length())?
            .read_to_end(&mut contents)?;
        assert_eq!(contents, b"hello world");
        Ok(())
    }

    #[test]
    fn over_budget_stream_leaves_no_published_object() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let cas = Cas::open(temporary.path().join("cas"))?;
        let result = cas.put(&b"too large"[..], PutBudget::new(3)?);
        assert!(matches!(
            result,
            Err(CasError::BudgetExceeded { maximum: 3 })
        ));
        assert_eq!(
            fs::read_dir(cas.root().join("objects").join(".tmp"))?.count(),
            0
        );
        Ok(())
    }

    #[test]
    fn wrong_length_object_is_quarantined_before_replacement() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let cas = Cas::open(temporary.path().join("cas"))?;
        let initial = cas.put(&b"expected"[..], PutBudget::new(64)?)?;
        fs::write(cas.object_path(initial.digest()), b"bad")?;

        let repaired = cas.put(&b"expected"[..], PutBudget::new(64)?)?;
        assert_eq!(repaired.outcome(), PublishOutcome::ReplacedCorrupt);
        assert!(
            cas.root()
                .join("objects")
                .join("quarantine")
                .join(initial.digest().to_hex())
                .join("object")
                .is_file()
        );
        assert_eq!(fs::read(cas.object_path(initial.digest()))?, b"expected");
        Ok(())
    }

    #[test]
    fn same_length_corrupt_object_is_rejected_and_replaced() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let cas = Cas::open(temporary.path().join("cas"))?;
        let initial = cas.put(&b"expected"[..], PutBudget::new(64)?)?;
        fs::write(cas.object_path(initial.digest()), b"corrupt!")?;

        assert!(matches!(
            cas.open_object(initial.digest(), initial.length()),
            Err(CasError::ExistingObjectCorrupt { digest }) if digest == initial.digest()
        ));

        let repaired = cas.put(&b"expected"[..], PutBudget::new(64)?)?;
        assert_eq!(repaired.outcome(), PublishOutcome::ReplacedCorrupt);
        assert_eq!(
            fs::read(
                cas.root()
                    .join("objects")
                    .join("quarantine")
                    .join(initial.digest().to_hex())
                    .join("object")
            )?,
            b"corrupt!"
        );
        assert_eq!(fs::read(cas.object_path(initial.digest()))?, b"expected");
        Ok(())
    }

    #[test]
    fn reader_failure_removes_partial_temporary_object() -> Result<(), Box<dyn Error>> {
        struct FailingReader {
            emitted: bool,
        }

        impl Read for FailingReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.emitted {
                    return Err(io::Error::other("injected reader failure"));
                }
                self.emitted = true;
                let partial = b"partial";
                buffer[..partial.len()].copy_from_slice(partial);
                Ok(partial.len())
            }
        }

        let temporary = tempdir()?;
        let cas = Cas::open(temporary.path().join("cas"))?;
        assert!(matches!(
            cas.put(FailingReader { emitted: false }, PutBudget::new(64)?),
            Err(CasError::Io {
                operation: "read object stream",
                ..
            })
        ));
        assert_eq!(
            fs::read_dir(cas.root().join("objects").join(".tmp"))?.count(),
            0
        );
        Ok(())
    }
}
