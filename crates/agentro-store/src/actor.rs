use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rusqlite::Connection;
use thiserror::Error;

use crate::{Migration, migration, repository};

/// Hard maximum number of queued database operations.
pub const MAX_QUEUE_CAPACITY: usize = 4_096;
/// Hard maximum SQLite busy timeout.
pub const MAX_BUSY_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(60);
const ACTOR_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// SQLite journal policy selected by the owning service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum JournalMode {
    /// Write-ahead logging for a database already confirmed to be local.
    Wal,
    /// Rollback journal for filesystems where WAL is not explicitly allowed.
    Delete,
}

/// Validated actor queue and SQLite connection settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreConfig {
    queue_capacity: usize,
    busy_timeout: Duration,
    journal_mode: JournalMode,
}

impl StoreConfig {
    /// Constructs bounded store settings.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InvalidConfiguration`] for a zero/excessive queue
    /// or busy timeout.
    pub fn new(
        queue_capacity: usize,
        busy_timeout: Duration,
        journal_mode: JournalMode,
    ) -> Result<Self, StoreError> {
        if queue_capacity == 0 || queue_capacity > MAX_QUEUE_CAPACITY {
            return Err(StoreError::InvalidConfiguration {
                field: "queue capacity",
            });
        }
        if busy_timeout < Duration::from_millis(1) || busy_timeout > MAX_BUSY_TIMEOUT {
            return Err(StoreError::InvalidConfiguration {
                field: "busy timeout",
            });
        }
        Ok(Self {
            queue_capacity,
            busy_timeout,
            journal_mode,
        })
    }
}

/// Stable failure classes for actor lifecycle, migration, and SQLite work.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A bounded setting or lifecycle timeout was invalid.
    #[error("invalid store configuration: {field}")]
    InvalidConfiguration {
        /// Name of the invalid field.
        field: &'static str,
    },
    /// The database path must be absolute.
    #[error("database path must be absolute")]
    DatabasePathNotAbsolute,
    /// The database actor thread could not be created.
    #[error("failed to start the database actor thread")]
    ThreadSpawn {
        /// Underlying thread creation error.
        #[source]
        source: std::io::Error,
    },
    /// SQLite open, pragma, query, or transaction failure.
    #[error("SQLite operation failed")]
    Sqlite {
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// SQLite accepted a PRAGMA statement but did not apply the requested value.
    #[error("SQLite PRAGMA {pragma} was not applied (actual: {actual})")]
    PragmaNotApplied {
        /// Stable PRAGMA name.
        pragma: &'static str,
        /// Bounded value read back from SQLite.
        actual: String,
    },
    /// Startup did not complete by the caller's deadline.
    #[error("database actor startup deadline exceeded")]
    StartupDeadlineExceeded,
    /// The bounded command queue rejected admission immediately.
    #[error("database actor queue is full at capacity {capacity}")]
    Overloaded {
        /// Configured command capacity.
        capacity: usize,
    },
    /// Admission was closed or the writer exited.
    #[error("database actor is closed")]
    Closed,
    /// A submitted operation did not reply by its caller deadline.
    #[error("database operation reply deadline exceeded")]
    ReplyDeadlineExceeded,
    /// Explicit shutdown did not finish by its caller deadline.
    #[error("database actor shutdown deadline exceeded")]
    ShutdownDeadlineExceeded,
    /// The writer thread panicked.
    #[error("database actor panicked")]
    ActorPanicked,
    /// Migration count or numbering was invalid.
    #[error("migration plan must contain at most {maximum} consecutive versions starting at one")]
    InvalidMigrationPlan {
        /// Maximum accepted migration count.
        maximum: usize,
    },
    /// The database schema is newer than this binary supports.
    #[error("database schema version {database} is newer than supported version {supported}")]
    DatabaseSchemaTooNew {
        /// Version found in SQLite.
        database: u32,
        /// Latest supplied migration version.
        supported: u32,
    },
    /// An applied migration row was missing below the schema maximum.
    #[error("applied migration version {version} is missing")]
    MissingAppliedMigration {
        /// Missing migration version.
        version: u32,
    },
    /// An immutable migration name no longer matched the applied row.
    #[error("applied migration version {version} has a different name")]
    MigrationNameMismatch {
        /// Conflicting migration version.
        version: u32,
    },
    /// An immutable migration checksum no longer matched the applied row.
    #[error("applied migration version {version} has a different checksum")]
    MigrationChecksumMismatch {
        /// Conflicting migration version.
        version: u32,
    },
    /// SQLite quick-check reported a bounded corruption diagnostic.
    #[error("SQLite quick-check failed: {diagnostic}")]
    QuickCheckFailed {
        /// Bounded first quick-check result.
        diagnostic: String,
    },
}

impl From<rusqlite::Error> for StoreError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Sqlite { source }
    }
}

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

enum Command {
    Run(Job),
    Shutdown(SyncSender<Result<(), StoreError>>),
}

struct Shared {
    sender: SyncSender<Command>,
    accepting: AtomicBool,
    abandon_requested: AtomicBool,
    queued: AtomicUsize,
    capacity: usize,
}

/// Cloneable admission handle without authority to shut down the writer.
#[derive(Clone)]
pub struct StoreHandle {
    shared: Arc<Shared>,
}

impl StoreHandle {
    /// Submits one typed operation without waiting for queue capacity.
    ///
    /// The operation runs on the connection-owning actor thread. If the reply
    /// deadline elapses, the already-admitted operation still completes; only
    /// its reply is discarded. Operations must therefore be short and must not
    /// contain network, process, hash, or filesystem work.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Overloaded`] when the bounded queue is full,
    /// [`StoreError::Closed`] after shutdown starts, or a typed operation/
    /// deadline error.
    pub fn call<T, F>(&self, reply_timeout: Duration, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        validate_lifecycle_timeout(reply_timeout, "reply timeout")?;
        if !self.shared.accepting.load(Ordering::Acquire) {
            return Err(StoreError::Closed);
        }

        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        let job = Box::new(move |connection: &mut Connection| {
            let result = operation(connection);
            let _ = reply_sender.send(result);
        });
        self.shared.queued.fetch_add(1, Ordering::AcqRel);
        match self.shared.sender.try_send(Command::Run(job)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.shared.queued.fetch_sub(1, Ordering::AcqRel);
                return Err(StoreError::Overloaded {
                    capacity: self.shared.capacity,
                });
            }
            Err(TrySendError::Disconnected(_)) => {
                self.shared.queued.fetch_sub(1, Ordering::AcqRel);
                return Err(StoreError::Closed);
            }
        }

        match reply_receiver.recv_timeout(reply_timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(StoreError::ReplyDeadlineExceeded),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(StoreError::Closed),
        }
    }

    /// Reads the latest applied migration version through the actor.
    ///
    /// # Errors
    ///
    /// Returns actor admission, deadline, or SQLite errors.
    pub fn schema_version(&self, reply_timeout: Duration) -> Result<u32, StoreError> {
        self.call(reply_timeout, |connection| {
            migration::schema_version(connection)
        })
    }

    /// Returns the current number of admitted operations not yet started.
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.shared.queued.load(Ordering::Acquire)
    }
}

/// Unique owner of a SQLite writer connection and actor thread.
pub struct StoreActor {
    shared: Arc<Shared>,
    interrupt: rusqlite::InterruptHandle,
    join: Option<JoinHandle<()>>,
    shutdown_reply: Option<Receiver<Result<(), StoreError>>>,
}

impl StoreActor {
    /// Opens, checks, migrates, and starts one database actor.
    ///
    /// `JournalMode::Wal` is an explicit assertion by the caller that the
    /// database resides on a supported local filesystem.
    ///
    /// # Errors
    ///
    /// Returns typed path, startup, SQLite, migration, or thread errors.
    pub fn start(
        path: PathBuf,
        config: StoreConfig,
        migrations: Vec<Migration>,
        startup_timeout: Duration,
    ) -> Result<Self, StoreError> {
        if !path.is_absolute() {
            return Err(StoreError::DatabasePathNotAbsolute);
        }
        validate_lifecycle_timeout(startup_timeout, "startup timeout")?;
        migration::validate_plan(&migrations)?;

        let (sender, receiver) = mpsc::sync_channel(config.queue_capacity);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            sender,
            accepting: AtomicBool::new(true),
            abandon_requested: AtomicBool::new(false),
            queued: AtomicUsize::new(0),
            capacity: config.queue_capacity,
        });
        let thread_shared = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name("agentro-sqlite-writer".to_owned())
            .spawn(move || {
                actor_main(
                    path,
                    config,
                    migrations,
                    receiver,
                    thread_shared,
                    startup_sender,
                )
            })
            .map_err(|source| StoreError::ThreadSpawn { source })?;

        match startup_receiver.recv_timeout(startup_timeout) {
            Ok(Ok(interrupt)) => Ok(Self {
                shared,
                interrupt,
                join: Some(join),
                shutdown_reply: None,
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                shared.accepting.store(false, Ordering::Release);
                shared.abandon_requested.store(true, Ordering::Release);
                let _ = join.join();
                Err(StoreError::StartupDeadlineExceeded)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => match join.join() {
                Ok(()) => Err(StoreError::Closed),
                Err(_) => Err(StoreError::ActorPanicked),
            },
        }
    }

    /// Returns a cloneable operation handle without shutdown authority.
    #[must_use]
    pub fn handle(&self) -> StoreHandle {
        StoreHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Closes admission, interrupts active SQLite work, and joins the writer.
    ///
    /// A timeout leaves ownership in this value so the caller can retry
    /// shutdown rather than silently detaching the actor.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ShutdownDeadlineExceeded`] when the actor has not
    /// stopped by the deadline, or a typed SQLite/panic error from shutdown.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), StoreError> {
        validate_lifecycle_timeout(timeout, "shutdown timeout")?;
        self.shared.accepting.store(false, Ordering::Release);
        self.interrupt.interrupt();
        let deadline = Instant::now() + timeout;

        if self.shutdown_reply.is_none() && !self.send_shutdown(deadline)? {
            return self.finish_join();
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let reply = self.shutdown_reply.as_ref().ok_or(StoreError::Closed)?;
        match reply.recv_timeout(remaining) {
            Ok(result) => {
                self.shutdown_reply = None;
                let joined = self.finish_join();
                result.and(joined)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(StoreError::ShutdownDeadlineExceeded),
            Err(mpsc::RecvTimeoutError::Disconnected) => self.finish_join(),
        }
    }

    fn send_shutdown(&mut self, deadline: Instant) -> Result<bool, StoreError> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        let mut command = Command::Shutdown(reply_sender);
        loop {
            match self.shared.sender.try_send(command) {
                Ok(()) => {
                    self.shutdown_reply = Some(reply_receiver);
                    return Ok(true);
                }
                Err(TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        return Err(StoreError::ShutdownDeadlineExceeded);
                    }
                    command = returned;
                    thread::sleep(ACTOR_POLL_INTERVAL);
                }
                Err(TrySendError::Disconnected(_)) => return Ok(false),
            }
        }
    }

    fn finish_join(&mut self) -> Result<(), StoreError> {
        match self.join.take() {
            Some(join) => join.join().map_err(|_| StoreError::ActorPanicked),
            None => Ok(()),
        }
    }
}

impl Drop for StoreActor {
    fn drop(&mut self) {
        self.shared.accepting.store(false, Ordering::Release);
        self.shared.abandon_requested.store(true, Ordering::Release);
        self.interrupt.interrupt();
        if self.shutdown_reply.is_none() {
            let (reply_sender, _reply_receiver) = mpsc::sync_channel(1);
            let _ = self.shared.sender.try_send(Command::Shutdown(reply_sender));
        }
    }
}

fn validate_lifecycle_timeout(timeout: Duration, field: &'static str) -> Result<(), StoreError> {
    if timeout.is_zero() || timeout > MAX_LIFECYCLE_TIMEOUT {
        return Err(StoreError::InvalidConfiguration { field });
    }
    Ok(())
}

fn actor_main(
    path: PathBuf,
    config: StoreConfig,
    migrations: Vec<Migration>,
    receiver: Receiver<Command>,
    shared: Arc<Shared>,
    startup_sender: SyncSender<Result<rusqlite::InterruptHandle, StoreError>>,
) {
    let mut connection =
        match repository::open(&path, config.busy_timeout, config.journal_mode, &migrations) {
            Ok(connection) => connection,
            Err(error) => {
                shared.accepting.store(false, Ordering::Release);
                let _ = startup_sender.send(Err(error));
                return;
            }
        };
    let interrupt = connection.get_interrupt_handle();
    if startup_sender.send(Ok(interrupt)).is_err() {
        shared.accepting.store(false, Ordering::Release);
        return;
    }

    while let Ok(command) = receiver.recv() {
        match command {
            Command::Run(job) => {
                shared.queued.fetch_sub(1, Ordering::AcqRel);
                job(&mut connection);
                if shared.abandon_requested.load(Ordering::Acquire) {
                    return;
                }
            }
            Command::Shutdown(reply) => {
                shared.accepting.store(false, Ordering::Release);
                let result = repository::checkpoint_on_shutdown(&connection, config.journal_mode);
                let _ = reply.send(result);
                return;
            }
        }
    }
    shared.accepting.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        io,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::*;

    fn config(capacity: usize) -> Result<StoreConfig, StoreError> {
        StoreConfig::new(capacity, Duration::from_millis(250), JournalMode::Wal)
    }

    #[test]
    fn full_queue_rejects_without_unbounded_wait() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let database = temporary.path().join("service.db");
        let mut actor =
            StoreActor::start(database, config(1)?, Vec::new(), Duration::from_secs(2))?;
        let first = actor.handle();
        let second = actor.handle();
        let rejected = actor.handle();
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let first_worker = thread::spawn(move || {
            first.call(Duration::from_secs(2), move |_connection| {
                let _ = started_sender.send(());
                release_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| StoreError::ReplyDeadlineExceeded)?;
                Ok(1_u32)
            })
        });
        started_receiver.recv_timeout(Duration::from_secs(1))?;
        let second_worker =
            thread::spawn(move || second.call(Duration::from_secs(2), |_connection| Ok(2_u32)));

        let depth_deadline = Instant::now() + Duration::from_secs(1);
        while rejected.queue_depth() != 1 && Instant::now() < depth_deadline {
            thread::yield_now();
        }
        assert_eq!(rejected.queue_depth(), 1);
        let error = match rejected.call(Duration::from_secs(1), |_connection| Ok(())) {
            Ok(()) => return Err("the third operation was unexpectedly admitted".into()),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::Overloaded { capacity: 1 }));
        release_sender.send(())?;
        assert_eq!(
            first_worker
                .join()
                .map_err(|_| io::Error::other("first writer panicked"))??,
            1
        );
        assert_eq!(
            second_worker
                .join()
                .map_err(|_| io::Error::other("second writer panicked"))??,
            2
        );
        actor.shutdown(Duration::from_secs(2))?;
        Ok(())
    }

    #[test]
    fn reply_timeout_drops_only_the_reply() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let database = temporary.path().join("service.db");
        let mut actor =
            StoreActor::start(database, config(4)?, Vec::new(), Duration::from_secs(2))?;
        let handle = actor.handle();
        let error = match handle.call(Duration::from_millis(10), |_connection| {
            thread::sleep(Duration::from_millis(50));
            Ok(())
        }) {
            Ok(()) => return Err("reply unexpectedly arrived before its deadline".into()),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::ReplyDeadlineExceeded));
        assert_eq!(handle.schema_version(Duration::from_secs(1))?, 0);
        actor.shutdown(Duration::from_secs(2))?;
        assert!(matches!(
            handle.schema_version(Duration::from_secs(1)),
            Err(StoreError::Closed)
        ));
        Ok(())
    }

    #[test]
    fn shutdown_closes_admission_and_can_be_repeated() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let database = temporary.path().join("service.db");
        let mut actor =
            StoreActor::start(database, config(4)?, Vec::new(), Duration::from_secs(2))?;
        let handle = actor.handle();
        actor.shutdown(Duration::from_secs(2))?;
        assert!(matches!(
            handle.call(Duration::from_secs(1), |_connection| Ok(())),
            Err(StoreError::Closed)
        ));
        actor.shutdown(Duration::from_secs(2))?;
        Ok(())
    }
}
