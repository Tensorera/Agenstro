//! Optional loopback viewer. Assistant text is transient and never enters the journal.

use std::{
    collections::{BTreeMap, VecDeque},
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{process::CancellationToken, studio, workspace::Workspace};

/// Default port of the explicitly started local stream viewer.
pub const DEFAULT_PORT: u16 = 47831;
const API: &str = "tactus.watch/v1";
const DESCRIPTOR: &str = "watch.json";
const MAX_HEADER: usize = 8192;
const MAX_TEXT: usize = 4096;
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const MAX_BUFFER_EVENTS: usize = 4096;
const MAX_PAGE_EVENTS: usize = 256;
const MAX_CLIENTS: usize = 8;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    api: String,
    port: u16,
    token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    token: String,
    run_id: String,
    parent_run_id: Option<String>,
    provider: String,
    source_seq: u64,
    kind: String,
    text: String,
    truncated: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveEvent {
    seq: String,
    at_unix_ms: String,
    run_id: String,
    parent_run_id: Option<String>,
    provider: String,
    kind: String,
    text: String,
    truncated: bool,
    missed_packets: u64,
}

#[derive(Default)]
struct LiveBuffer {
    events: VecDeque<LiveEvent>,
    bytes: usize,
    sequence: u64,
    sources: BTreeMap<String, u64>,
}

impl LiveBuffer {
    fn push(&mut self, packet: Packet) {
        let previous = self.sources.get(&packet.run_id).copied().unwrap_or(0);
        // UDP duplicates and late packets cannot duplicate displayed text.
        if packet.source_seq <= previous {
            return;
        }
        if self.sources.len() >= MAX_BUFFER_EVENTS && !self.sources.contains_key(&packet.run_id) {
            self.sources.pop_first();
        }
        self.sources
            .insert(packet.run_id.clone(), packet.source_seq);
        self.sequence = self.sequence.saturating_add(1);
        self.bytes += packet.text.len();
        self.events.push_back(LiveEvent {
            seq: self.sequence.to_string(),
            at_unix_ms: unix_millis().to_string(),
            run_id: packet.run_id,
            parent_run_id: packet.parent_run_id,
            provider: packet.provider,
            kind: packet.kind,
            text: packet.text,
            truncated: packet.truncated,
            missed_packets: packet.source_seq.saturating_sub(previous).saturating_sub(1),
        });
        while self.events.len() > MAX_BUFFER_EVENTS || self.bytes > MAX_BUFFER_BYTES {
            if let Some(event) = self.events.pop_front() {
                self.bytes -= event.text.len();
            }
        }
    }

    fn page(&self, after: u64, run: Option<&str>) -> Value {
        let oldest = self
            .events
            .front()
            .and_then(|event| event.seq.parse::<u64>().ok());
        let mut next_after = after;
        let mut events = Vec::new();
        for event in &self.events {
            let sequence = event.seq.parse::<u64>().unwrap_or(0);
            if sequence <= after {
                continue;
            }
            next_after = sequence;
            if run.is_none_or(|run| {
                event.run_id == run || event.parent_run_id.as_deref() == Some(run)
            }) {
                events.push(event);
                if events.len() == MAX_PAGE_EVENTS {
                    break;
                }
            }
        }
        json!({"api": API, "events": events, "nextAfter": next_after.to_string(),
            "historyLost": oldest.is_some_and(|oldest| after.saturating_add(1) < oldest),
            "latest": self.sequence.to_string()})
    }
}

/// Best-effort, nonblocking native-provider output publication.
///
/// A publisher exists only when this workspace has an active viewer descriptor.
/// Its UDP socket has no effect on provider stdout, result values, or journals.
pub struct Publisher {
    socket: UdpSocket,
    descriptor: Descriptor,
    run_id: String,
    parent_run_id: Option<String>,
    provider: String,
    sequence: u64,
}

impl Publisher {
    /// Connect to an explicitly started workspace viewer, if available.
    #[must_use]
    pub fn connect(start: &Path, fallback_id: &str, provider: &str) -> Option<Self> {
        let workspace = Workspace::discover(start).ok()?;
        let path = workspace.root.join(".tactus").join(DESCRIPTOR);
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() > 4096 {
            return None;
        }
        let mut bytes = Vec::new();
        File::open(path)
            .ok()?
            .take(4097)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 4096 {
            return None;
        }
        let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
        if descriptor.api != API || !valid_token(&descriptor.token) || descriptor.port == 0 {
            return None;
        }
        let run_id = env::var("TACTUS_WATCH_RUN_ID").unwrap_or_else(|_| fallback_id.to_owned());
        if !valid_id(&run_id) {
            return None;
        }
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        socket
            .connect((Ipv4Addr::LOCALHOST, descriptor.port))
            .ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self {
            socket,
            descriptor,
            run_id,
            parent_run_id: env::var("TACTUS_RUN_ID").ok().filter(|id| valid_id(id)),
            provider: provider.to_owned(),
            sequence: 0,
        })
    }

    /// Publish only recognized visible assistant text or a fixed lifecycle marker.
    /// Oversized messages are explicitly truncated to keep observation bounded.
    pub fn emit(&mut self, kind: &str, text: &str) {
        let mut offset = 0;
        for part in 0..64 {
            let mut end = (offset + MAX_TEXT).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.sequence = self.sequence.saturating_add(1);
            let packet = Packet {
                token: self.descriptor.token.clone(),
                run_id: self.run_id.clone(),
                parent_run_id: self.parent_run_id.clone(),
                provider: self.provider.clone(),
                source_seq: self.sequence,
                kind: kind.to_owned(),
                text: text[offset..end].to_owned(),
                truncated: part == 63 && end < text.len(),
            };
            if let Ok(bytes) = serde_json::to_vec(&packet) {
                let _ = self.socket.send(&bytes);
            }
            offset = end;
            if offset == text.len() {
                break;
            }
        }
    }
}

struct Server {
    listener: TcpListener,
    datagrams: UdpSocket,
    root: PathBuf,
    descriptor: Descriptor,
    descriptor_path: PathBuf,
    _lease: File,
    buffer: Arc<Mutex<LiveBuffer>>,
}

impl Server {
    fn bind(start: &Path, port: u16) -> Result<Self, WatchError> {
        let workspace = Workspace::discover(start)?;
        let directory = workspace.root.join(".tactus");
        let lock_path = directory.join("watch.lock");
        reject_special_file(&lock_path)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        fs4::FileExt::try_lock(&lease).map_err(|_| WatchError::AlreadyRunning)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let port = listener.local_addr()?.port();
        let datagrams = UdpSocket::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        datagrams.set_nonblocking(true)?;
        // tempfile uses OS randomness; its default owner-only permissions also
        // keep the bearer token private while replacing a crashed server's file.
        let mut temporary = tempfile::Builder::new()
            .prefix("watch-")
            .rand_bytes(32)
            .tempfile_in(&directory)?;
        let token = temporary
            .path()
            .file_name()
            .ok_or(WatchError::InvalidDescriptor)?
            .to_string_lossy()
            .into_owned();
        let descriptor = Descriptor {
            api: API.to_owned(),
            port,
            token,
        };
        let descriptor_path = directory.join(DESCRIPTOR);
        reject_special_file(&descriptor_path)?;
        serde_json::to_writer(&mut temporary, &descriptor)?;
        temporary.flush()?;
        temporary
            .persist(&descriptor_path)
            .map_err(|error| WatchError::Io(error.error))?;
        Ok(Self {
            listener,
            datagrams,
            root: workspace.root,
            descriptor,
            descriptor_path,
            _lease: lease,
            buffer: Arc::new(Mutex::new(LiveBuffer::default())),
        })
    }

    fn run(&self, cancellation: &CancellationToken) -> Result<(), WatchError> {
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
        let mut bytes = [0_u8; 65536];
        while !cancellation.is_cancelled() {
            workers.retain(|worker| !worker.is_finished());
            for _ in 0..256 {
                match self.datagrams.recv_from(&mut bytes) {
                    Ok((length, peer)) if peer.ip().is_loopback() => {
                        if let Ok(packet) = serde_json::from_slice::<Packet>(&bytes[..length])
                            && packet.token == self.descriptor.token
                            && valid_packet(&packet)
                        {
                            self.buffer
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(packet);
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error.into()),
                }
            }
            match self.listener.accept() {
                Ok((stream, peer)) if peer.ip().is_loopback() && workers.len() < MAX_CLIENTS => {
                    let root = self.root.clone();
                    let descriptor = self.descriptor.clone();
                    let buffer = Arc::clone(&self.buffer);
                    workers.push(thread::spawn(move || {
                        let _ = handle_client(stream, &root, &descriptor, &buffer);
                    }));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            thread::sleep(Duration::from_millis(10));
        }
        for worker in workers {
            let _ = worker.join();
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Keep the lease locked until after descriptor removal. A second watcher
        // cannot publish a replacement which this instance then accidentally removes.
        let _ = fs::remove_file(&self.descriptor_path);
    }
}

/// Serve the read-only viewer until cancellation. Binding is always loopback.
pub fn serve(start: &Path, port: u16, cancellation: &CancellationToken) -> Result<(), WatchError> {
    let server = Server::bind(start, port)?;
    println!(
        "[info] Tactus live output: http://127.0.0.1:{}/#token={}",
        server.descriptor.port, server.descriptor.token
    );
    println!(
        "[info] Start provider tasks while this viewer is running. Output stays in bounded memory; Ctrl-C stops the viewer."
    );
    server.run(cancellation)
}

fn reject_special_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(io::Error::other("viewer metadata must be a regular file"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_token(value: &str) -> bool {
    value.len() >= 32 && valid_id(value)
}

fn valid_packet(packet: &Packet) -> bool {
    valid_id(&packet.run_id)
        && packet.parent_run_id.as_deref().is_none_or(valid_id)
        && valid_id(&packet.provider)
        && packet.text.len() <= MAX_TEXT
        && packet.source_seq > 0
        && matches!(
            packet.kind.as_str(),
            "started" | "text.delta" | "text.message" | "text.final" | "finished"
        )
}

fn unix_millis() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

struct Request {
    target: String,
    headers: BTreeMap<String, String>,
}

fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline exceeded"))?;
        stream.set_read_timeout(Some(remaining))?;
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(io::Error::other("incomplete request"));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_HEADER {
            return Err(io::Error::other("request too large"));
        }
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let (header, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| io::Error::other("invalid request"))?;
    if !body.is_empty() {
        return Err(io::Error::other("request body is unsupported"));
    }
    let mut lines = header.split("\r\n");
    let first = lines
        .next()
        .unwrap_or_default()
        .split(' ')
        .collect::<Vec<_>>();
    if first.len() != 3 || first[0] != "GET" || first[2] != "HTTP/1.1" || !first[1].starts_with('/')
    {
        return Err(io::Error::other("expected HTTP/1.1 GET"));
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::other("invalid header"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || headers
                .insert(name.to_ascii_lowercase(), value.trim().to_owned())
                .is_some()
        {
            return Err(io::Error::other("invalid or duplicate header"));
        }
    }
    if headers.contains_key("transfer-encoding")
        || headers
            .get("content-length")
            .is_some_and(|value| value != "0")
    {
        return Err(io::Error::other("request body is unsupported"));
    }
    Ok(Request {
        target: first[1].to_owned(),
        headers,
    })
}

fn handle_client(
    mut stream: TcpStream,
    root: &Path,
    descriptor: &Descriptor,
    buffer: &Mutex<LiveBuffer>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(_) => {
            return respond(
                &mut stream,
                400,
                "text/plain",
                b"Invalid or oversized HTTP request.",
            );
        }
    };
    let hosts = [
        format!("127.0.0.1:{}", descriptor.port),
        format!("localhost:{}", descriptor.port),
    ];
    if !request
        .headers
        .get("host")
        .is_some_and(|host| hosts.contains(host))
        || request
            .headers
            .get("origin")
            .is_some_and(|origin| !hosts.iter().any(|host| origin == &format!("http://{host}")))
    {
        return respond(&mut stream, 403, "text/plain", b"Local origin required.");
    }
    match request.target.as_str() {
        "/" => {
            return respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                include_bytes!("watch/index.html"),
            );
        }
        "/app.js" => {
            return respond(
                &mut stream,
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("watch/app.js"),
            );
        }
        "/style.css" => {
            return respond(
                &mut stream,
                200,
                "text/css; charset=utf-8",
                include_bytes!("watch/style.css"),
            );
        }
        _ => {}
    }
    if request.headers.get("authorization") != Some(&format!("Bearer {}", descriptor.token)) {
        return respond(
            &mut stream,
            401,
            "text/plain",
            b"Open the URL printed by tactus watch.",
        );
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    let query = match parse_query(query) {
        Ok(query) => query,
        Err(()) => return respond(&mut stream, 400, "text/plain", b"Invalid query."),
    };
    let payload = match path {
        "/api/runs" if query.is_empty() => match studio::inspect(root, 50) {
            Ok(snapshot) => {
                json!({"api": API, "workspace": snapshot.workspace, "runs": snapshot.runs})
            }
            Err(_) => {
                return respond(
                    &mut stream,
                    500,
                    "text/plain",
                    b"Cannot read workspace run history.",
                );
            }
        },
        "/api/live" if query.keys().all(|key| matches!(*key, "after" | "run")) => {
            let after = match query.get("after").copied().unwrap_or("0").parse::<u64>() {
                Ok(after) => after,
                Err(_) => return respond(&mut stream, 400, "text/plain", b"Invalid sequence."),
            };
            buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .page(after, query.get("run").copied())
        }
        "/api/events" if query.keys().all(|key| matches!(*key, "after" | "run")) => {
            let Some(run) = query.get("run") else {
                return respond(&mut stream, 400, "text/plain", b"Run required.");
            };
            let Ok(after) = query.get("after").copied().unwrap_or("0").parse::<u64>() else {
                return respond(&mut stream, 400, "text/plain", b"Invalid sequence.");
            };
            match studio::events(root, run, after, 100, 20 * 1024 * 1024) {
                Ok(page) => json!({"api": API, "run": page.run, "nextAfter": page.next_after,
                    "complete": page.complete, "integrity": page.integrity,
                    "events": page.events.into_iter().map(|event| json!({"seq": event.seq,
                        "atUnixMs": event.at_unix_ms, "kind": event.kind,
                        "presentation": event.presentation})).collect::<Vec<_>>() }),
                Err(_) => {
                    return respond(
                        &mut stream,
                        404,
                        "text/plain",
                        b"Run history unavailable or exceeds read limits.",
                    );
                }
            }
        }
        _ => return respond(&mut stream, 404, "text/plain", b"Not found."),
    };
    let bytes = serde_json::to_vec(&payload).map_err(io::Error::other)?;
    respond(&mut stream, 200, "application/json; charset=utf-8", &bytes)
}

fn parse_query(query: &str) -> Result<BTreeMap<&str, &str>, ()> {
    let mut result = BTreeMap::new();
    if query.is_empty() {
        return Ok(result);
    }
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or(())?;
        // Run IDs and decimal cursors do not need percent decoding. Rejecting
        // every other character keeps path/query interpretation unambiguous.
        if !valid_id(key) || !valid_id(value) || result.insert(key, value).is_some() {
            return Err(());
        }
    }
    Ok(result)
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\n\r\n",
        body.len()
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    for mut bytes in [header.as_bytes(), body] {
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "response deadline exceeded")
                })?;
            stream.set_write_timeout(Some(remaining))?;
            let written = stream.write(bytes)?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            bytes = &bytes[written..];
        }
    }
    Ok(())
}

/// Failure starting or serving the local viewer.
#[derive(Debug, Error)]
pub enum WatchError {
    /// No valid initialized workspace was found.
    #[error(transparent)]
    Workspace(#[from] crate::workspace::WorkspaceError),
    /// Local listener or metadata access failed.
    #[error("stream viewer I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Descriptor encoding failed.
    #[error("stream viewer metadata is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// Only one viewer may own a workspace descriptor at a time.
    #[error("a stream viewer is already running for this workspace")]
    AlreadyRunning,
    /// Random token generation failed.
    #[error("cannot create stream viewer metadata")]
    InvalidDescriptor,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{journal::RunJournal, workspace::initialize_workspace};

    struct RunningServer {
        temporary: tempfile::TempDir,
        descriptor: Descriptor,
        cancellation: CancellationToken,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl RunningServer {
        fn new() -> Self {
            let temporary = tempfile::tempdir().expect("temporary workspace");
            initialize_workspace(temporary.path(), None).expect("initialize");
            let server = Server::bind(temporary.path(), 0).expect("bind");
            let descriptor = server.descriptor.clone();
            let cancellation = CancellationToken::new();
            let token = cancellation.clone();
            let worker = Some(thread::spawn(move || server.run(&token).expect("serve")));
            Self {
                temporary,
                descriptor,
                cancellation,
                worker,
            }
        }

        fn request(&self, target: &str, extra: &str, authenticated: bool) -> String {
            let authorization = if authenticated {
                format!("Authorization: Bearer {}\r\n", self.descriptor.token)
            } else {
                String::new()
            };
            self.raw_request(&format!(
                "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{authorization}{extra}\r\n",
                self.descriptor.port
            ))
        }

        fn raw_request(&self, request: &str) -> String {
            let mut stream =
                TcpStream::connect((Ipv4Addr::LOCALHOST, self.descriptor.port)).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .expect("timeout");
            stream.write_all(request.as_bytes()).expect("request");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("response");
            response
        }

        fn json(&self, target: &str) -> Value {
            let response = self.request(target, "", true);
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            serde_json::from_str(response.split_once("\r\n\r\n").expect("header").1).expect("JSON")
        }
    }

    impl Drop for RunningServer {
        fn drop(&mut self) {
            self.cancellation.cancel();
            self.worker
                .take()
                .expect("worker")
                .join()
                .expect("shutdown");
            assert!(!self.temporary.path().join(".tactus/watch.json").exists());
        }
    }

    #[test]
    fn http_requires_token_local_origin_and_bounded_well_formed_requests() {
        let server = RunningServer::new();
        let page = server.request("/", "", false);
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("Content-Security-Policy:"));
        assert!(!page.contains(&server.descriptor.token));
        assert!(
            server
                .request("/api/live", "", false)
                .starts_with("HTTP/1.1 401")
        );
        assert!(
            server
                .request("/api/live", "Origin: https://attacker.invalid\r\n", true)
                .starts_with("HTTP/1.1 403")
        );
        assert!(
            server
                .raw_request("GET / HTTP/1.1\r\nHost: attacker.invalid\r\n\r\n")
                .starts_with("HTTP/1.1 403")
        );
        assert!(
            server
                .request("/api/live?after=0&after=1", "", true)
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            server
                .request("/api/events?run=..%2Fsecret", "", true)
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            server
                .request("/api/live", "Content-Length: 1\r\n", true)
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            server
                .request("/api/live", "Host: duplicate\r\n", true)
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            server
                .request(
                    "/api/live",
                    &format!("X-Oversized: {}\r\n", "x".repeat(MAX_HEADER)),
                    true
                )
                .starts_with("HTTP/1.1 400")
        );
        assert!(
            server
                .raw_request("POST / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .starts_with("HTTP/1.1 400")
        );
    }

    #[test]
    fn live_output_arrives_before_completion_reconnects_and_never_enters_history() {
        let server = RunningServer::new();
        let workspace = Workspace::discover(server.temporary.path()).expect("workspace");
        let mut journal = RunJournal::create(&workspace).expect("journal");
        journal
            .record(
                "invocation.started",
                json!({"namespace":"provider", "plugin":"fake", "method":"invoke"}),
            )
            .expect("record");
        let mut publisher =
            Publisher::connect(&workspace.root, journal.run_id(), "fake").expect("publisher");
        let attacker = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("test datagram socket");
        let mut forged = packet(1, "unauthorized text");
        forged.token = "incorrect-token".to_owned();
        attacker
            .send_to(
                &serde_json::to_vec(&forged).expect("forged packet"),
                (Ipv4Addr::LOCALHOST, server.descriptor.port),
            )
            .expect("send forged packet");
        publisher.emit("text.delta", "<script>alert('visible text')</script>你好");
        let start = std::time::Instant::now();
        let page = loop {
            let page = server.json("/api/live?after=0");
            if !page["events"].as_array().expect("events").is_empty() {
                break page;
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "stream never arrived"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            page["events"][0]["text"],
            "<script>alert('visible text')</script>你好"
        );
        assert_eq!(page["events"][0]["runId"], journal.run_id());
        assert_eq!(page["events"].as_array().expect("events").len(), 1);
        assert_eq!(
            server.json("/api/live?after=0"),
            page,
            "reconnect replays retained text"
        );
        assert_eq!(server.json("/api/live?after=1")["events"], json!([]));
        let history = server.json(&format!("/api/events?run={}&after=0", journal.run_id()));
        assert_eq!(history["run"]["state"], "open");
        assert_eq!(history["complete"], false);
        assert!(!history.to_string().contains("visible text"));
        assert!(
            !fs::read_to_string(journal.run_path().join("events.jsonl"))
                .expect("journal")
                .contains("visible text")
        );
        assert_eq!(
            server.json("/api/runs")["runs"][0]["runId"],
            journal.run_id()
        );
        journal
            .finish(crate::process::ProcessOutcome {
                kind: crate::process::InvocationKind::Succeeded,
                exit_code: Some(0),
                terminal: Some(crate::protocol::TerminalResult::Success {
                    value: json!({"text":"private terminal text"}),
                }),
                frames_seen: 1,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms: 1,
                progress: None,
            })
            .expect("finish journal");
        let completed = server.json(&format!("/api/events?run={}&after=1", journal.run_id()));
        assert_eq!(completed["complete"], true);
        assert_eq!(completed["run"]["state"], "succeeded");
        assert_eq!(completed["events"], json!([]));
        assert!(!completed.to_string().contains("private terminal text"));
    }

    #[test]
    fn workspace_lease_blocks_competing_viewers_and_stale_descriptor_is_replaced() {
        let temporary = tempfile::tempdir().expect("temporary");
        initialize_workspace(temporary.path(), None).expect("initialize");
        fs::write(temporary.path().join(".tactus/watch.json"), "stale").expect("stale descriptor");
        let first = Server::bind(temporary.path(), 0).expect("first");
        assert!(matches!(
            Server::bind(temporary.path(), 0),
            Err(WatchError::AlreadyRunning)
        ));
        let token = first.descriptor.token.clone();
        assert!(valid_token(&token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&first.descriptor_path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        drop(first);
        let second = Server::bind(temporary.path(), 0).expect("second");
        assert_ne!(second.descriptor.token, token);
    }

    fn packet(sequence: u64, text: &str) -> Packet {
        Packet {
            token: "watch-test-token-01234567890123456789".to_owned(),
            run_id: "child".to_owned(),
            parent_run_id: Some("parent".to_owned()),
            provider: "fake".to_owned(),
            source_seq: sequence,
            kind: "text.delta".to_owned(),
            text: text.to_owned(),
            truncated: false,
        }
    }

    #[test]
    fn bounded_memory_reports_expired_history_and_packet_gaps() {
        let mut buffer = LiveBuffer::default();
        buffer.push(packet(1, "first"));
        buffer.push(packet(3, "gap"));
        buffer.push(packet(2, "late"));
        assert_eq!(
            buffer.page(0, Some("parent"))["events"][1]["missedPackets"],
            1
        );
        assert_eq!(buffer.events.len(), 2);
        let text = "x".repeat(MAX_TEXT);
        for sequence in 4..1500 {
            buffer.push(packet(sequence, &text));
        }
        assert!(buffer.bytes <= MAX_BUFFER_BYTES);
        assert!(buffer.events.len() <= MAX_BUFFER_EVENTS);
        assert_eq!(buffer.page(0, None)["historyLost"], true);
        assert_eq!(buffer.page(0, Some("unrelated"))["events"], json!([]));
        assert!(
            buffer.page(0, None)["events"]
                .as_array()
                .expect("events")
                .len()
                <= MAX_PAGE_EVENTS
        );
    }

    #[cfg(unix)]
    #[test]
    fn metadata_symlink_is_rejected_without_modifying_target() {
        let temporary = tempfile::tempdir().expect("temporary");
        initialize_workspace(temporary.path(), None).expect("initialize");
        let target = temporary.path().join("private");
        fs::write(&target, "unchanged").expect("target");
        std::os::unix::fs::symlink(&target, temporary.path().join(".tactus/watch.json"))
            .expect("symlink");
        assert!(Server::bind(temporary.path(), 0).is_err());
        assert_eq!(fs::read_to_string(target).expect("target"), "unchanged");
    }
}
