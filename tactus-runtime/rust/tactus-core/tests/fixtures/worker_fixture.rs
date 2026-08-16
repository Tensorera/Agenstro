use std::{env, io, io::Write, thread, time::Duration};

use tactus_core::{RunId, encode_worker_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().collect();
    let run_id = argument_value(&arguments, "--run-id")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing run ID"))?;
    let run_id = RunId::parse(run_id)?;
    if argument_value(&arguments, "--fixture-mode") == Some("sleep") {
        thread::sleep(Duration::from_secs(30));
        return Ok(());
    }

    let mut stdout = io::stdout().lock();
    for (sequence, kind, body) in [
        (1_u64, 1_u8, &b""[..]),
        (2, 2, &b""[..]),
        (3, 3, &b"fixture output\n"[..]),
        (4, 4, &b""[..]),
    ] {
        let mut payload = sequence.to_be_bytes().to_vec();
        payload.push(kind);
        payload.extend_from_slice(body);
        stdout.write_all(&encode_worker_frame(run_id, &payload)?)?;
    }
    stdout.flush()?;
    Ok(())
}

fn argument_value<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}
