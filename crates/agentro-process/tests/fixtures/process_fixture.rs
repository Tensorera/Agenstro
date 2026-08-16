use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    process::{Command, ExitCode, Stdio},
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(io::stderr(), "fixture error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<u8> {
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    let mode = arguments
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing fixture mode"))?;
    match mode {
        "echo" => {
            let value = arguments
                .get(1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing echo value"))?;
            writeln!(io::stdout(), "{}", value.to_string_lossy())?;
            writeln!(io::stderr(), "fixture-stderr")?;
            Ok(0)
        }
        "sleep" => {
            let milliseconds = parse_u64(arguments.get(1), "sleep milliseconds")?;
            thread::sleep(Duration::from_millis(milliseconds));
            Ok(0)
        }
        "flood" => {
            let bytes = parse_u64(arguments.get(1), "flood bytes")?;
            flood(bytes)?;
            Ok(0)
        }
        "spawn-child" => {
            let milliseconds = parse_u64(arguments.get(1), "child sleep milliseconds")?;
            let executable = env::current_exe()?;
            let _child = Command::new(executable)
                .arg("sleep")
                .arg(milliseconds.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()?;
            writeln!(io::stdout(), "child-started")?;
            Ok(0)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown fixture mode",
        )),
    }
}

fn parse_u64(value: Option<&OsString>, field: &str) -> io::Result<u64> {
    value
        .and_then(|item| item.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("missing {field}")))?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn flood(bytes: u64) -> io::Result<()> {
    let chunk = [b'x'; 4_096];
    let mut remaining = bytes;
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    while remaining > 0 {
        let count = usize::try_from(remaining.min(chunk.len() as u64)).unwrap_or(chunk.len());
        stdout.write_all(&chunk[..count])?;
        stderr.write_all(&chunk[..count])?;
        remaining -= count as u64;
    }
    stdout.flush()?;
    stderr.flush()
}
