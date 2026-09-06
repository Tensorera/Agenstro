//! Linux isolation backend. No fallback to an unsandboxed command is allowed.

use std::{
    collections::BTreeSet,
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        process::ExitStatusExt,
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Instant,
};

use serde_json::{Value, json};
use tactus_runtime::PluginRequest;

use crate::{LOG_LIMIT, Result, RunParams, emit, failure, io_failure, outcome_metadata};

const START_MARKER: &[u8] = b"\x01MOTIVO_TEST_STARTED\x01\n";

struct Capture {
    truncated: bool,
    started: bool,
}

fn safe_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(io_failure)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(failure(
                    "unsafe_path",
                    format!("{} must be a real directory", path.display()),
                ))
            }
        }
        Err(error) => Err(io_failure(error)),
    }
}

fn visit_tree(
    root: &Path,
    mut visit: impl FnMut(&Path, &fs::Metadata) -> Result<()>,
) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(io_failure)? {
            let entry = entry.map_err(io_failure)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(io_failure)?;
            visit(&path, &metadata)?;
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

fn decode_mount_path(encoded: &str) -> PathBuf {
    // Linux mountinfo uses octal escapes for these four characters.
    PathBuf::from(
        encoded
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\"),
    )
}

fn validate_sample(sample: &Path) -> Result<()> {
    let mounts = fs::read_to_string("/proc/self/mountinfo").map_err(io_failure)?;
    for line in mounts.lines() {
        if let Some(mount) = line.split_ascii_whitespace().nth(4)
            && decode_mount_path(mount).starts_with(sample)
        {
            return Err(failure(
                "unsafe_path",
                "the sample directory must not contain a host mount",
            ));
        }
    }
    visit_tree(sample, |path, metadata| {
        if metadata.file_type().is_symlink()
            || (!metadata.is_dir() && !metadata.is_file())
            || (metadata.is_file() && metadata.nlink() != 1)
        {
            return Err(failure(
                "unsafe_path",
                format!(
                    "sample fixtures must be regular files with one link or real directories: {}",
                    path.display()
                ),
            ));
        }
        Ok(())
    })
}

fn log_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io_failure)
}

/// Drains all output, but never retains more than one MiB per stream. The
/// launch marker comes from a fixed trusted shell wrapper, before argv starts.
fn capture(reader: impl Read, mut log: File, expect_start: bool) -> io::Result<Capture> {
    let mut reader = BufReader::new(reader);
    let mut retained = 0_u64;
    let mut truncated = false;
    let mut started = !expect_start;
    if expect_start {
        let mut first = Vec::new();
        reader.by_ref().take(4096).read_until(b'\n', &mut first)?;
        if first == START_MARKER {
            started = true;
        } else {
            log.write_all(&first)?;
            retained = first.len() as u64;
        }
    }
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let amount = reader.read(&mut buffer)?;
        if amount == 0 {
            break;
        }
        let keep = usize::try_from(LOG_LIMIT.saturating_sub(retained))
            .unwrap_or(usize::MAX)
            .min(amount);
        log.write_all(&buffer[..keep])?;
        retained += keep as u64;
        truncated |= keep < amount;
    }
    log.flush()?;
    Ok(Capture { truncated, started })
}

fn system_mounts(command: &mut Command) -> Result<()> {
    command.args(["--ro-bind", "/usr", "/usr"]);
    for name in ["bin", "sbin", "lib", "lib64"] {
        let path = PathBuf::from(format!("/{name}"));
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                command
                    .arg("--symlink")
                    .arg(fs::read_link(&path).map_err(io_failure)?)
                    .arg(&path);
            }
            Ok(_) => {
                command.arg("--ro-bind").arg(&path).arg(&path);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_failure(error)),
        }
    }
    for path in ["/etc/ld.so.cache", "/etc/localtime"] {
        if Path::new(path).is_file() {
            command.args(["--ro-bind", path, path]);
        }
    }
    Ok(())
}

fn toolchain_mounts(command: &mut Command) -> Result<String> {
    let mut roots = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        // Installations only. Do not mount .cargo, .config, provider state,
        // SSH material, or the surrounding user home.
        for suffix in [".local/share/mise/installs", ".rustup/toolchains"] {
            let path = home.join(suffix);
            if path.is_dir() {
                let root = fs::canonicalize(&path).map_err(io_failure)?;
                command.arg("--ro-bind").arg(&root).arg(&root);
                roots.push(root);
            }
        }
    }
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        if !entry.is_absolute() {
            continue;
        }
        if let Ok(path) = fs::canonicalize(&entry)
            && path.is_dir()
            && (path.starts_with("/usr") || roots.iter().any(|root| path.starts_with(root)))
            && seen.insert(path.clone())
        {
            entries.push(path);
        }
    }
    for path in [PathBuf::from("/usr/bin"), PathBuf::from("/bin")] {
        if seen.insert(path.clone()) {
            entries.push(path);
        }
    }
    env::join_paths(entries)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| failure("invalid_environment", error.to_string()))
}

fn mask_project_ipc(command: &mut Command, root: &Path) -> Result<()> {
    visit_tree(root, |path, metadata| {
        let kind = metadata.file_type();
        if kind.is_socket() || kind.is_fifo() || kind.is_block_device() || kind.is_char_device() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| failure("unsafe_path", error.to_string()))?;
            command
                .arg("--ro-bind")
                .arg("/dev/null")
                .arg(Path::new("/project").join(relative));
        }
        Ok(())
    })
}

fn executable_exists(argv0: &str, path: &str, root: &Path, sample: &Path) -> bool {
    let translate = |candidate: &Path| -> PathBuf {
        if let Ok(relative) = candidate.strip_prefix("/project") {
            root.join(relative)
        } else if let Ok(relative) = candidate.strip_prefix("/work") {
            sample.join(relative)
        } else {
            candidate.to_owned()
        }
    };
    let executable = |candidate: &Path| {
        fs::metadata(translate(candidate))
            .is_ok_and(|metadata| metadata.is_file() && metadata.mode() & 0o111 != 0)
    };
    if argv0.contains('/') {
        let candidate = Path::new(argv0);
        if candidate
            .components()
            .any(|part| part == Component::ParentDir)
        {
            return false;
        }
        if candidate.is_absolute() {
            executable(candidate)
        } else {
            executable(&Path::new("/work").join(candidate))
        }
    } else {
        env::split_paths(path).any(|directory| executable(&directory.join(argv0)))
    }
}

pub(super) fn execute(request: &PluginRequest) -> Result<Value> {
    let started = Instant::now();
    let params = RunParams::parse(request.params.clone())?;
    if !Path::new("/usr/bin/bwrap").is_file() {
        return Err(failure(
            "backend_unavailable",
            "/usr/bin/bwrap is not installed; no unsandboxed fallback",
        ));
    }
    let root = fs::canonicalize(env::current_dir().map_err(io_failure)?).map_err(io_failure)?;
    if let Some(workspace) = &params.workspace
        && fs::canonicalize(workspace).map_err(io_failure)? != root
    {
        return Err(failure(
            "invalid_workspace",
            "injected workspace must identify the plugin's working directory",
        ));
    }
    if root == Path::new("/")
        || env::var_os("HOME")
            .and_then(|home| fs::canonicalize(home).ok())
            .is_some_and(|home| home.starts_with(&root))
    {
        return Err(failure(
            "unsafe_workspace",
            "initialize a project folder, not a filesystem root or a directory containing your user home",
        ));
    }
    let tactus = root.join(".tactus");
    if !fs::symlink_metadata(&tactus)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err(failure(
            "invalid_workspace",
            "cwd must be an initialized workspace with a real .tactus directory",
        ));
    }
    let mut sample = tactus;
    for component in ["motivotest", &params.run_id, &params.sample_id] {
        sample.push(component);
        safe_directory(&sample)?;
    }
    validate_sample(&sample)?;
    let logs = sample.join(".motivo-logs");
    fs::create_dir(&logs).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            failure(
                "sample_exists",
                "sample already has an execution record; choose a new sample_id",
            )
        } else {
            io_failure(error)
        }
    })?;
    let stdout = log_file(&logs.join("stdout.log"))?;
    let stderr = log_file(&logs.join("stderr.log"))?;
    for directory in ["home", "tmp", "cache"] {
        safe_directory(&sample.join(directory))?;
    }

    let mut command = Command::new("/usr/bin/bwrap");
    command.env_clear().args([
        "--unshare-all",
        "--unshare-user",
        "--die-with-parent",
        "--new-session",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--clearenv",
    ]);
    system_mounts(&mut command)?;
    let path = toolchain_mounts(&mut command)?;
    if !executable_exists(&params.argv[0], &path, &root, &sample) {
        return Err(failure(
            "command_unavailable",
            format!(
                "experiment executable {:?} is missing or not executable",
                params.argv[0]
            ),
        ));
    }
    command.arg("--ro-bind").arg(&root).arg("/project");
    mask_project_ipc(&mut command, &root)?;
    command
        .arg("--bind")
        .arg(&sample)
        .arg("/work")
        .arg("--ro-bind")
        .arg(&logs)
        .arg("/work/.motivo-logs")
        .args([
            "--symlink",
            "/work/tmp",
            "/tmp",
            "--proc",
            "/proc",
            "--remount-ro",
            "/proc",
            "--dev",
            "/dev",
            "--remount-ro",
            "/dev",
        ])
        .args(["--setenv", "PATH", &path, "--setenv", "HOME", "/work/home"])
        .args([
            "--setenv",
            "TMPDIR",
            "/work/tmp",
            "--setenv",
            "TMP",
            "/work/tmp",
            "--setenv",
            "TEMP",
            "/work/tmp",
        ])
        .args([
            "--setenv",
            "XDG_CACHE_HOME",
            "/work/cache",
            "--setenv",
            "XDG_CONFIG_HOME",
            "/work/home/.config",
        ])
        .args([
            "--setenv",
            "CARGO_HOME",
            "/work/cache/cargo",
            "--setenv",
            "CABAL_DIR",
            "/work/cache/cabal",
        ])
        .args([
            "--setenv",
            "LANG",
            "C.UTF-8",
            "--setenv",
            "MOTIVO_PROJECT",
            "/project",
        ])
        .args([
            "--chdir",
            "/work",
            "--remount-ro",
            "/",
            "--",
            "/bin/sh",
            "-c",
            r#"printf '\001MOTIVO_TEST_STARTED\001\n' >&2; exec "$@""#,
            "motivo.test",
        ])
        .args(&params.argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    emit(&json!({"type":"event","id":request.id,"event":{
        "type":"experiment_started","run_id":params.run_id,"sample_id":params.sample_id,
        "timeout_seconds":params.timeout_seconds,"workspace_dir":params.relative_dir()
    }}))
    .map_err(io_failure)?;
    let mut child = command
        .spawn()
        .map_err(|error| failure("sandbox_start_failed", error.to_string()))?;
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| failure("io_error", "missing stdout pipe"))?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| failure("io_error", "missing stderr pipe"))?;
    let stdout_worker = thread::spawn(move || capture(stdout_pipe, stdout, false));
    let stderr_worker = thread::spawn(move || capture(stderr_pipe, stderr, true));
    let status = child.wait().map_err(io_failure)?;
    let stdout_capture = stdout_worker
        .join()
        .map_err(|_| failure("io_error", "stdout drain failed"))?
        .map_err(io_failure)?;
    let stderr_capture = stderr_worker
        .join()
        .map_err(|_| failure("io_error", "stderr drain failed"))?
        .map_err(io_failure)?;
    let mut value = outcome_metadata(&params, "exited", started);
    value["exit_code"] = json!(status.code());
    value["signal"] = json!(status.signal());
    value["stdout_truncated"] = json!(stdout_capture.truncated);
    value["stderr_truncated"] = json!(stderr_capture.truncated);
    value["logs_may_be_partial"] = json!(false);
    if !stderr_capture.started {
        value["status"] = json!("execution_error");
        let mut error = failure(
            "sandbox_unavailable",
            "Bubblewrap could not create the required isolation; inspect stderr_path; no fallback was executed",
        );
        error.details = Some(value);
        return Err(error);
    }
    Ok(value)
}
