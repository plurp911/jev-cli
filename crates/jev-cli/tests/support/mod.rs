//! Shared harness for the integration tests.
//!
//! # What this provides
//!
//! * [`MockApi`] — a real HTTP server on loopback, so the tests exercise the actual
//!   `ureq` transport, real sockets, real headers, and real status codes rather than a
//!   trait double. That is the layer the unit tests cannot reach.
//! * [`jev`] — the compiled binary with a hermetic environment: no inherited
//!   credential, no inherited configuration, no colour.
//!
//! Nothing here contacts the internet. Every test binds `127.0.0.1:0` and talks to
//! itself, which is also why the CLI's loopback exception to the HTTPS rule exists.
#![allow(
    unreachable_pub,
    dead_code,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use assert_cmd::Command;

/// A canary that must never appear in any output stream.
pub const CANARY_KEY: &str = "sk-canary-must-never-be-printed-0123456789";

/// One queued HTTP reply.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Bytes to send in the body regardless of `body`, for malformed-response tests.
    pub raw_body: Option<Vec<u8>>,
}

impl Reply {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            raw_body: None,
        }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            raw_body: None,
        }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    #[must_use]
    pub fn raw(mut self, bytes: Vec<u8>) -> Self {
        self.raw_body = Some(bytes);
        self
    }
}

/// What the server saw.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// A loopback HTTP server that returns queued replies and records requests.
pub struct MockApi {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    served: Arc<AtomicUsize>,
    shutdown: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl MockApi {
    /// Starts a server that serves `replies` in order, then repeats the last one.
    pub fn start(replies: Vec<Reply>) -> Self {
        Self::start_with_delay(replies, std::time::Duration::ZERO)
    }

    /// The same, but each reply is held back by `delay`.
    ///
    /// This is what makes a signal test possible: the child has to still be running,
    /// mid-batch, when the signal arrives, and a server that answers instantly races
    /// the test to completion.
    pub fn slow(replies: Vec<Reply>, delay: std::time::Duration) -> Self {
        Self::start_with_delay(replies, delay)
    }

    fn start_with_delay(replies: Vec<Reply>, delay: std::time::Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::new(AtomicUsize::new(0));

        let worker_seen = Arc::clone(&seen);
        let worker_served = Arc::clone(&served);
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let index = worker_served.fetch_add(1, Ordering::SeqCst);
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
                let reply = replies
                    .get(index)
                    .or_else(|| replies.last())
                    .cloned()
                    .unwrap_or_else(|| Reply::ok("{}"));
                serve(stream, &reply, |request| {
                    worker_seen
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(request);
                });
            }
        });

        Self {
            port,
            seen,
            served,
            shutdown: Arc::new(Mutex::new(Some(handle))),
        }
    }

    /// A server that never answers, for timeout tests.
    pub fn hanging() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let served = Arc::new(AtomicUsize::new(0));
        let worker_served = Arc::clone(&served);
        let handle = std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                worker_served.fetch_add(1, Ordering::SeqCst);
                // Held open, never written to: the client must hit its own deadline.
                held.push(stream);
            }
        });
        Self {
            port,
            seen: Arc::new(Mutex::new(Vec::new())),
            served,
            shutdown: Arc::new(Mutex::new(Some(handle))),
        }
    }

    /// The base URL to pass to `--endpoint`.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A port nothing is listening on, for connection-refused tests.
    pub fn dead_endpoint() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    /// Every request the server received.
    pub fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// How many connections were accepted.
    pub fn hits(&self) -> usize {
        self.served.load(Ordering::SeqCst)
    }
}

impl Drop for MockApi {
    fn drop(&mut self) {
        // Unblocks `incoming()` so the worker thread can exit with the test.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Ok(mut guard) = self.shutdown.lock() {
            guard.take();
        }
    }
}

/// Reads one request and writes one reply. Minimal on purpose: it speaks exactly the
/// subset of HTTP/1.1 that `ureq` and these tests need.
/// Reads one request, records it, then replies.
///
/// Recorded *before* the reply is written. Recorded after, a client that got its answer
/// and exited could be observed by the test before the server thread had logged the
/// request, and `requests()` came back empty: a race that failed only under load.
fn serve(mut stream: TcpStream, reply: &Reply, record: impl FnOnce(Seen)) -> Option<()> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);

    let mut start = String::new();
    if reader.read_line(&mut start).ok()? == 0 {
        return None;
    }
    let mut parts = start.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    if length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    record(Seen {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    });

    let payload = reply
        .raw_body
        .clone()
        .unwrap_or_else(|| reply.body.clone().into_bytes());
    let mut response = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
        reply.status,
        payload.len()
    );
    for (name, value) in &reply.headers {
        use std::fmt::Write as _;
        let _ = write!(response, "{name}: {value}\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).ok()?;
    stream.write_all(&payload).ok()?;
    stream.flush().ok()?;

    Some(())
}

/// The compiled binary, with a hermetic environment.
///
/// Every credential and configuration variable is cleared, and the configuration
/// directory is pointed somewhere that does not exist, so a test never reads or writes
/// a developer's real configuration or keychain.
pub fn jev() -> Command {
    let mut command = Command::cargo_bin("jev").expect("the `jev` binary should build");
    command
        .env_remove("JEV_API_KEY")
        .env_remove("JEV_API_KEY_FILE")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("CLOUDFLARE_ACCOUNT_ID")
        .env_remove("JEV_CUSTOM_API_KEY")
        .env_remove("JEV_CUSTOM_API_KEY_FILE")
        .env_remove("NO_COLOR")
        // `jev` disables proxy inheritance in the transport, but a developer or a CI
        // runner with these exported would otherwise have every loopback request in
        // this suite routed through their proxy. Clearing them keeps the suite
        // hermetic, and `a_proxy_in_the_environment_does_not_reroute_the_request`
        // asserts the transport ignores them even when they are set.
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env("JEV_CONFIG_DIR", "/nonexistent/jev-integration-test")
        .env("NO_COLOR", "1")
        // Without this the spawned binary queries the developer's live Secret Service
        // or Keychain, so `auth status` and the missing-credential tests would fail on
        // a machine where the maintainer had ever run `jev auth login` -- a test that
        // depends on the developer's environment, which `AGENTS.md` §9 forbids. It can
        // only make secure storage report as unavailable; it supplies no credential.
        .env("JEV_NO_KEYCHAIN", "1");
    command
}

/// The same hermetic environment as [`jev`], but as a `std::process::Command`.
///
/// `assert_cmd` runs a child to completion, which is exactly what a signal test cannot
/// do: the child has to be spawned, observed, signalled, and only then reaped.
pub fn jev_spawnable() -> std::process::Command {
    let binary = assert_cmd::cargo::cargo_bin("jev");
    let mut command = std::process::Command::new(binary);
    command
        .env_remove("JEV_API_KEY")
        .env_remove("JEV_API_KEY_FILE")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("CLOUDFLARE_ACCOUNT_ID")
        .env_remove("JEV_CUSTOM_API_KEY")
        .env_remove("JEV_CUSTOM_API_KEY_FILE")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env("JEV_CONFIG_DIR", "/nonexistent/jev-integration-test")
        .env("NO_COLOR", "1")
        .env("JEV_NO_KEYCHAIN", "1")
        .env("JEV_API_KEY", CANARY_KEY)
        .env("JEV_CUSTOM_API_KEY", CANARY_KEY);
    command
}

/// Blocks until `condition` holds or `timeout` elapses. Returns whether it held.
///
/// Polling rather than sleeping a fixed time: a fixed sleep is either flaky on a loaded
/// CI runner or slow on an idle one, and usually both.
pub fn wait_until(timeout: std::time::Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    condition()
}

/// The binary with a canary credential in the environment.
///
/// Both the official and the custom-endpoint variables are set, because the mock server
/// runs on loopback and loopback is, correctly, not the official endpoint. The tests
/// that assert the isolation between those two namespaces set the variables themselves.
pub fn jev_authed() -> Command {
    let mut command = jev();
    command
        .env("JEV_API_KEY", CANARY_KEY)
        .env("JEV_CUSTOM_API_KEY", CANARY_KEY);
    command
}

/// A successful single-Noul response body, as the API reference documents it.
pub fn noul_body() -> String {
    r#"{"model":"jev-1.13.0","answers":{"answer":{"type":"noul","noul":0.92}},"usage":{"input_tokens":312,"output_tokens":48}}"#.to_owned()
}

/// Asserts the canary appears in neither stream.
pub fn assert_no_canary(output: &std::process::Output) {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains(CANARY_KEY),
        "credential leaked:\n{combined}"
    );
}

/// Parses stdout as a single JSON document.
pub fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|error| {
        panic!("stdout is not one JSON document ({error}):\n{text}");
    })
}
