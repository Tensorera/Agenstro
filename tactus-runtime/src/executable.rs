//! Deterministic executable discovery shared by diagnostics and native hosts.

use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::ffi::OsStr;

#[cfg(windows)]
const DEFAULT_WINDOWS_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// A deterministic executable lookup failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExecutableResolutionError {
    /// No executable matched the requested command or explicit path.
    NotFound {
        /// The configured command.
        command: String,
        /// Whether the command named an explicit absolute or relative path.
        explicit: bool,
    },
    /// More than one distinct executable matched a bare command.
    Ambiguous {
        /// The configured command.
        command: String,
        /// Every distinct matching executable in deterministic search order.
        candidates: Vec<PathBuf>,
    },
}

impl ExecutableResolutionError {
    /// Stable error code suitable for provider protocol failures.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "provider_not_found",
            Self::Ambiguous { .. } => "provider_executable_ambiguous",
        }
    }

    /// Candidate paths, when lookup was ambiguous.
    pub(crate) fn candidates(&self) -> &[PathBuf] {
        match self {
            Self::NotFound { .. } => &[],
            Self::Ambiguous { candidates, .. } => candidates,
        }
    }
}

impl fmt::Display for ExecutableResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound {
                command,
                explicit: true,
            } => write!(
                formatter,
                "explicit executable {command:?} does not name a file"
            ),
            Self::NotFound {
                command,
                explicit: false,
            } => write!(formatter, "executable {command:?} was not found on PATH"),
            Self::Ambiguous {
                command,
                candidates,
            } => {
                write!(
                    formatter,
                    "executable {command:?} is ambiguous; pin one exact path"
                )?;
                for candidate in candidates {
                    write!(formatter, "; {}", candidate.display())?;
                }
                Ok(())
            }
        }
    }
}

/// An environment-independent executable resolver.
///
/// Tests inject PATH/PATHEXT directly. Production callers use [`Self::environment`]
/// so `doctor` and provider hosts observe exactly the same search domain.
#[derive(Clone, Debug)]
pub(crate) struct ExecutableResolver {
    search_path: OsString,
    #[cfg(windows)]
    path_extensions: Option<OsString>,
    working_directory: PathBuf,
}

impl ExecutableResolver {
    /// Capture the effective process environment for one working directory.
    pub(crate) fn environment(working_directory: impl AsRef<Path>) -> Self {
        Self::new(effective_path(), env::var_os("PATHEXT"), working_directory)
    }

    /// Construct a resolver from explicit inputs.
    pub(crate) fn new(
        search_path: impl Into<OsString>,
        path_extensions: Option<OsString>,
        working_directory: impl AsRef<Path>,
    ) -> Self {
        #[cfg(not(windows))]
        let _ = path_extensions;
        Self {
            search_path: search_path.into(),
            #[cfg(windows)]
            path_extensions,
            working_directory: working_directory.as_ref().to_path_buf(),
        }
    }

    /// Resolve one command, rejecting both missing and ambiguous matches.
    pub(crate) fn resolve(&self, command: &str) -> Result<PathBuf, ExecutableResolutionError> {
        let command_path = Path::new(command);
        let explicit = command_path.is_absolute() || command_path.components().count() > 1;
        let candidates = if explicit {
            self.explicit_candidates(command_path)
        } else {
            self.path_candidates(command_path)
        };
        match candidates.as_slice() {
            [candidate] => Ok(candidate.clone()),
            [] => Err(ExecutableResolutionError::NotFound {
                command: command.to_owned(),
                explicit,
            }),
            _ => Err(ExecutableResolutionError::Ambiguous {
                command: command.to_owned(),
                candidates,
            }),
        }
    }

    fn explicit_candidates(&self, command: &Path) -> Vec<PathBuf> {
        let candidate = if command.is_absolute() {
            command.to_path_buf()
        } else {
            self.working_directory.join(command)
        };
        #[cfg(windows)]
        {
            if candidate.extension().is_none() {
                return normalize_candidates(
                    self.windows_extensions()
                        .into_iter()
                        .map(|extension| append_extension(&candidate, &extension))
                        .filter(|path| path.is_file()),
                );
            }
        }
        candidate
            .is_file()
            .then(|| normalize_candidates([candidate]))
            .unwrap_or_default()
    }

    fn path_candidates(&self, command: &Path) -> Vec<PathBuf> {
        let directories = env::split_paths(&self.search_path);
        #[cfg(windows)]
        {
            if command.extension().is_some() {
                return normalize_candidates(
                    directories
                        .map(|directory| directory.join(command))
                        .filter(|path| path.is_file()),
                );
            }
            let extensions = self.windows_extensions();
            return normalize_candidates(
                directories
                    .flat_map(|directory| {
                        extensions.iter().map(move |extension| {
                            append_extension(&directory.join(command), extension)
                        })
                    })
                    .filter(|path| path.is_file()),
            );
        }
        #[cfg(not(windows))]
        {
            normalize_candidates(
                directories
                    .map(|directory| directory.join(command))
                    .filter(|path| path.is_file()),
            )
        }
    }

    #[cfg(windows)]
    fn windows_extensions(&self) -> Vec<OsString> {
        let configured = self
            .path_extensions
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsStr::new(DEFAULT_WINDOWS_PATHEXT));
        let mut extensions = configured
            .to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.starts_with('.') {
                    OsString::from(value)
                } else {
                    OsString::from(format!(".{value}"))
                }
            })
            .collect::<Vec<_>>();
        if extensions.is_empty() {
            extensions = DEFAULT_WINDOWS_PATHEXT
                .split(';')
                .map(OsString::from)
                .collect();
        }
        extensions
    }
}

#[cfg(windows)]
fn append_extension(path: &Path, extension: &OsStr) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(extension);
    PathBuf::from(value)
}

fn normalize_candidates(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for candidate in candidates {
        let normalized = dunce::canonicalize(&candidate).unwrap_or(candidate);
        #[cfg(windows)]
        let key = normalized.to_string_lossy().to_lowercase();
        #[cfg(not(windows))]
        let key = normalized.as_os_str().to_os_string();
        if seen.insert(key) {
            output.push(normalized);
        }
    }
    output
}

/// Return the inherited PATH plus newly persisted Windows user/machine PATH
/// entries. Long-lived agents otherwise cannot see tools installed later.
pub(crate) fn effective_path() -> OsString {
    #[cfg(not(windows))]
    {
        env::var_os("PATH").unwrap_or_default()
    }

    #[cfg(windows)]
    {
        use winreg::{
            RegKey,
            enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        };

        let mut values = Vec::new();
        if let Some(current) = env::var_os("PATH") {
            values.push(current);
        }
        let locations = [
            (HKEY_CURRENT_USER, "Environment"),
            (
                HKEY_LOCAL_MACHINE,
                r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            ),
        ];
        for (root, key) in locations {
            let Ok(key) = RegKey::predef(root).open_subkey(key) else {
                continue;
            };
            if let Ok(path) = key.get_value::<String, _>("Path") {
                values.push(path.into());
            }
        }
        let entries = values.iter().flat_map(env::split_paths);
        env::join_paths(entries).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn search_path(paths: &[&Path]) -> OsString {
        env::join_paths(paths).expect("test search path")
    }

    #[test]
    fn explicit_relative_paths_are_resolved_from_the_working_directory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let executable = temporary.path().join("工具 with space").join("agent.bin");
        fs::create_dir_all(executable.parent().expect("parent")).expect("create parent");
        fs::write(&executable, b"fixture").expect("write executable fixture");
        let resolver = ExecutableResolver::new(OsString::new(), None, temporary.path());

        let resolved = resolver
            .resolve("工具 with space/agent.bin")
            .expect("relative explicit executable");
        assert_eq!(
            resolved,
            dunce::canonicalize(executable).expect("canonical")
        );
    }

    #[test]
    fn explicit_absolute_paths_do_not_depend_on_path() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let executable = temporary.path().join("provider with space.bin");
        fs::write(&executable, b"fixture").expect("write executable fixture");
        let resolver =
            ExecutableResolver::new(OsString::new(), None, temporary.path().join("unrelated"));

        let resolved = resolver
            .resolve(executable.to_str().expect("UTF-8 fixture path"))
            .expect("absolute explicit executable");
        assert_eq!(
            resolved,
            dunce::canonicalize(executable).expect("canonical")
        );
    }

    #[test]
    fn distinct_path_matches_are_rejected_as_ambiguous() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs::create_dir_all(&first).expect("first directory");
        fs::create_dir_all(&second).expect("second directory");
        #[cfg(windows)]
        let name = "claude.EXE";
        #[cfg(not(windows))]
        let name = "claude";
        fs::write(first.join(name), b"first").expect("first executable");
        fs::write(second.join(name), b"second").expect("second executable");
        let resolver = ExecutableResolver::new(
            search_path(&[&first, &second]),
            Some(OsString::from(".EXE")),
            temporary.path(),
        );

        let error = resolver
            .resolve("claude")
            .expect_err("ambiguous executable");
        let ExecutableResolutionError::Ambiguous { candidates, .. } = error else {
            panic!("expected ambiguity")
        };
        assert_eq!(candidates.len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolution_handles_unicode_spaces_and_missing_pathext() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let directory = temporary.path().join("中文 provider tools");
        fs::create_dir_all(&directory).expect("command directory");
        let executable = directory.join("claude.CMD");
        fs::write(&executable, b"@echo off\r\n").expect("command fixture");
        let resolver = ExecutableResolver::new(search_path(&[&directory]), None, temporary.path());

        assert_eq!(
            resolver.resolve("claude").expect("default PATHEXT"),
            dunce::canonicalize(executable).expect("canonical")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_empty_pathext_entries_never_select_an_extensionless_npm_shim() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let shim = temporary.path().join("claude");
        let launcher = temporary.path().join("claude.cmd");
        fs::write(&shim, b"#!/bin/sh\n").expect("extensionless npm shim");
        fs::write(&launcher, b"@echo off\r\n").expect("Windows npm launcher");
        let resolver = ExecutableResolver::new(
            search_path(&[temporary.path()]),
            Some(OsString::from(";;.CMD;")),
            temporary.path(),
        );

        assert_eq!(
            resolver.resolve("claude").expect("cmd launcher"),
            dunce::canonicalize(launcher).expect("canonical")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_explicit_extensionless_paths_still_select_the_native_launcher() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let tools = temporary.path().join("tools");
        fs::create_dir_all(&tools).expect("tools directory");
        fs::write(tools.join("claude"), b"#!/bin/sh\n").expect("extensionless npm shim");
        let launcher = tools.join("claude.cmd");
        fs::write(&launcher, b"@echo off\r\n").expect("Windows npm launcher");
        let resolver = ExecutableResolver::new(
            OsString::new(),
            Some(OsString::from(".CMD")),
            temporary.path(),
        );

        assert_eq!(
            resolver
                .resolve("tools/claude")
                .expect("explicit cmd launcher"),
            dunce::canonicalize(launcher).expect("canonical")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_empty_pathext_uses_safe_defaults() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let executable = temporary.path().join("codex.exe");
        fs::write(&executable, b"fixture").expect("executable fixture");
        let resolver = ExecutableResolver::new(
            search_path(&[temporary.path()]),
            Some(OsString::new()),
            temporary.path(),
        );

        assert_eq!(
            resolver.resolve("codex").expect("default PATHEXT"),
            dunce::canonicalize(executable).expect("canonical")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_pathext_normalizes_variants_without_a_leading_dot() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let executable = temporary.path().join("opencode.bat");
        fs::write(&executable, b"@echo off\r\n").expect("executable fixture");
        let resolver = ExecutableResolver::new(
            search_path(&[temporary.path()]),
            Some(OsString::from("BAT;")),
            temporary.path(),
        );

        assert_eq!(
            resolver.resolve("opencode").expect("normalized PATHEXT"),
            dunce::canonicalize(executable).expect("canonical")
        );
    }
}
