//! Tests of the real HTTP transport against a real socket.
//!
//! The unit tests in `jev-client` use `MockTransport`, which cannot catch a
//! misconfiguration of the HTTP library itself. These can: they bind a loopback
//! listener and speak HTTP/1.1 at it.
//!
//! Nothing here contacts the internet.
#![cfg(feature = "http")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use jev_client::{
    Client, Credential, Endpoint, HttpTransport, Request, RetryPolicy, Transport, TransportError,
};

/// Serves one canned response and returns the request line and headers it saw.
fn serve_once(
    status: u16,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, BTreeMap<String, String>) {
    serve_with_credential(
        status,
        extra_headers,
        body,
        &Credential::new("sk-transport-canary".to_owned()),
    )
}

fn serve_with_credential(
    status: u16,
    extra_headers: &[(&str, &str)],
    body: &[u8],
    credential: &Credential,
) -> (u16, BTreeMap<String, String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let headers_seen: std::sync::Arc<std::sync::Mutex<BTreeMap<String, String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));

    let worker_headers = std::sync::Arc::clone(&headers_seen);
    let body = body.to_vec();
    let extra: Vec<(String, String)> = extra_headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut length = 0_usize;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap() == 0 {
                break;
            }
            let trimmed = header.trim_end();
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                if name == "content-length" {
                    length = value.trim().parse().unwrap_or(0);
                }
                worker_headers
                    .lock()
                    .unwrap()
                    .insert(name, value.trim().to_owned());
            }
        }
        let mut discard = vec![0_u8; length];
        let _ = reader.read_exact(&mut discard);

        let mut response = format!(
            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in &extra {
            use std::fmt::Write as _;
            let _ = write!(response, "{name}: {value}\r\n");
        }
        response.push_str("\r\n");
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
    });

    let transport = HttpTransport::new(Duration::from_secs(5));
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/systemone"),
        method: "POST",
        headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        body: b"{}".to_vec(),
    };
    let response = transport
        .execute(&request, credential)
        .expect("the transport should return a response");
    handle.join().unwrap();

    let seen = headers_seen.lock().unwrap().clone();
    assert_eq!(response.status, status);
    (response.status, seen)
}

#[test]
fn a_non_2xx_status_comes_back_as_a_response_not_a_transport_error() {
    // The `Transport` contract: HTTP status is data, so the caller can decode the API's
    // structured error body and tell a 401 from a 422 from a 529. A transport that
    // turned these into errors would collapse every one of them into "could not reach
    // the API", which is both wrong and unactionable.
    for status in [400_u16, 401, 403, 404, 422, 429, 500, 503, 529] {
        let (observed, _) = serve_once(status, &[], br#"{"detail":"nope"}"#);
        assert_eq!(observed, status);
    }
}

#[test]
fn the_authorization_header_is_sent_and_nothing_else_carries_the_key() {
    let (_, headers) = serve_once(200, &[], b"{}");
    assert_eq!(headers["authorization"], "Bearer sk-transport-canary");
    for (name, value) in &headers {
        if name != "authorization" {
            assert!(
                !value.contains("sk-transport-canary"),
                "the credential also appeared in `{name}`"
            );
        }
    }
}

#[test]
fn anonymous_transport_omits_authorization_while_empty_keys_remain_explicit() {
    let (_, anonymous) = serve_with_credential(200, &[], b"{}", &Credential::anonymous());
    assert!(!anonymous.contains_key("authorization"));
    let (_, empty) = serve_with_credential(200, &[], b"{}", &Credential::new(String::new()));
    assert_eq!(
        empty.get("authorization").map(String::as_str),
        Some("Bearer")
    );
}

#[test]
fn the_user_agent_identifies_this_cli() {
    let (_, headers) = serve_once(200, &[], b"{}");
    assert!(
        headers["user-agent"].starts_with("jev-cli/"),
        "unexpected user agent: {}",
        headers["user-agent"]
    );
}

#[test]
fn response_headers_are_lowercased_so_retry_after_is_found() {
    // The retry policy looks headers up by lowercase name. A transport that preserved
    // the server's casing would silently stop honouring `Retry-After`.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            if line.trim_end().is_empty() {
                break;
            }
            line.clear();
        }
        stream
            .write_all(
                b"HTTP/1.1 429 X\r\nRetry-After: 3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });

    let transport = HttpTransport::new(Duration::from_secs(5));
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/models"),
        method: "GET",
        headers: BTreeMap::new(),
        body: Vec::new(),
    };
    let response = transport
        .execute(&request, &Credential::new("sk-x".to_owned()))
        .unwrap();
    handle.join().unwrap();

    assert_eq!(response.header("retry-after"), Some("3"));
    assert_eq!(
        jev_client::parse_retry_after(&response),
        Some(Duration::from_secs(3))
    );
}

#[test]
fn an_oversized_response_is_refused_rather_than_buffered() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            if line.trim_end().is_empty() {
                break;
            }
            line.clear();
        }
        let payload = vec![b'x'; 4096];
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            )
            .as_bytes(),
        );
        let _ = stream.write_all(&payload);
    });

    let transport = HttpTransport::new(Duration::from_secs(5)).with_max_response_bytes(64);
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/models"),
        method: "GET",
        headers: BTreeMap::new(),
        body: Vec::new(),
    };
    let error = transport
        .execute(&request, &Credential::new("sk-x".to_owned()))
        .unwrap_err();
    handle.join().unwrap();

    assert!(matches!(
        error,
        TransportError::ResponseTooLarge { limit: 64 }
    ));
    // And it is not retried: the same endpoint would send the same thing again.
    assert!(!error.is_transient());
}

#[test]
fn a_redirect_is_not_followed() {
    // Following a redirect from an API endpoint would send the credential to whatever
    // host the `Location` header names.
    let (status, _) = serve_once(302, &[("Location", "https://evil.example/steal")], b"");
    assert_eq!(status, 302, "the redirect was followed instead of returned");
}

#[test]
fn a_connection_refused_is_a_transient_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let transport = HttpTransport::new(Duration::from_secs(2));
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/models"),
        method: "GET",
        headers: BTreeMap::new(),
        body: Vec::new(),
    };
    let error = transport
        .execute(&request, &Credential::new("sk-x".to_owned()))
        .unwrap_err();
    assert!(error.is_transient());
    assert!(!error.to_string().contains("sk-x"));
}

#[test]
fn a_body_that_stalls_after_the_headers_is_reported_as_a_timeout() {
    // The headers arrive, then the body trickles to a stop. The read path used to map
    // any `io::Error` straight to `Unreachable` without going through `classify`, and
    // `ureq` wraps its own `Timeout` as `ErrorKind::Other` -- so a stalled body was
    // reported as "could not reach the API endpoint: other error". The endpoint *was*
    // reached, and the one thing that would help, `--timeout`, never occurred to the
    // reader.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            if line.trim_end().is_empty() {
                break;
            }
            line.clear();
        }
        // Promise sixteen bytes, send four, then hold the connection open.
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\n{\"a\"")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_secs(5));
    });

    let transport = HttpTransport::new(Duration::from_secs(1));
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/systemone"),
        method: "POST",
        headers: BTreeMap::new(),
        body: b"{}".to_vec(),
    };
    let error = transport
        .execute(&request, &Credential::new("sk-x".to_owned()))
        .expect_err("a stalled body must not succeed");

    assert!(
        matches!(error, TransportError::Timeout { .. }),
        "a stalled body was reported as {error:?} rather than as a timeout"
    );
    drop(handle);
}

#[test]
fn a_connection_closed_mid_response_is_transient_and_retryable() {
    // The other truncation shape: the server promises a length, sends part of the body,
    // then closes the socket outright rather than stalling. A crash, a killed worker, a
    // load balancer dropping the connection.
    //
    // This must be classified as *transient*. The alternative -- treating a short body
    // as a malformed response -- would be wrong twice: it is not a decoding problem, and
    // a non-retryable classification turns a routine mid-deploy blip into a failed run.
    // Nothing pinned which way this went, and the two behaviours differ materially.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            if line.trim_end().is_empty() {
                break;
            }
            line.clear();
        }
        // Promise sixty-four bytes, send eight, then drop the socket.
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\n{\"model\":")
            .unwrap();
        stream.flush().unwrap();
        drop(stream);
    });

    let transport = HttpTransport::new(Duration::from_secs(5));
    let request = Request {
        url: format!("http://127.0.0.1:{port}/v1/systemone"),
        method: "POST",
        headers: BTreeMap::new(),
        body: b"{}".to_vec(),
    };
    let error = transport
        .execute(&request, &Credential::new("sk-x".to_owned()))
        .expect_err("a truncated body must not be returned as a successful response");
    handle.join().unwrap();

    assert!(
        error.is_transient(),
        "a connection closed mid-response was classified as permanent ({error:?}), so a \
         routine blip would end the run instead of being retried"
    );
    assert!(
        !error.to_string().contains("sk-x"),
        "the credential reached the error message"
    );
}

/// Captures complete request bytes at the actual HTTP boundary, without any external
/// network. Model response values here are hand-authored schema test cases.
fn provider_round_trip(cloudflare: bool) {
    use jev_core::{Content, EvaluationRequest, ModelId, Question, QuestionId, State};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut headers = BTreeMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            let (name, value) = line.trim().split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
        let length: usize = headers["content-length"].parse().unwrap();
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["model"], "clef");
        assert_eq!(body["questions"]["q"]["type"], "noul");
        if cloudflare {
            assert_eq!(
                request_line,
                "POST /client/v4/accounts/0123456789abcdef0123456789abcdef/ai/run/@cf/cloudflare/clef HTTP/1.1\r\n"
            );
            assert_eq!(headers["authorization"], "Bearer sk-local-transport-canary");
        } else {
            assert_eq!(request_line, "POST /v1/systemone HTTP/1.1\r\n");
            assert!(!headers.contains_key("authorization"));
        }
        let inner = serde_json::json!({"model":"clef","answers":{"q":{"type":"noul","noul":0.75}},"usage":{"input_tokens":5,"output_tokens":0}});
        let response = if cloudflare {
            serde_json::json!({"success":true,"result":inner,"errors":[],"messages":[]})
        } else {
            inner
        }
        .to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCf-Ray: test-ray\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
    });
    let base = Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let endpoint = if cloudflare {
        base.with_cloudflare_account("0123456789abcdef0123456789abcdef")
            .unwrap()
    } else {
        base.with_ollama()
    };
    let client = Client::new(HttpTransport::new(Duration::from_secs(5)), endpoint)
        .with_retry(RetryPolicy::none());
    let request = EvaluationRequest::new(
        State::text("s").unwrap(),
        ModelId::new("clef").unwrap(),
        vec![(
            QuestionId::new("q").unwrap(),
            Question::noul(Content::text("Is it?").unwrap(), None).unwrap(),
        )],
    )
    .unwrap();
    let (response, stats) = client.evaluate(
        &request,
        &Credential::new("sk-local-transport-canary".to_owned()),
    );
    assert_eq!(response.unwrap().model.as_str(), "clef");
    if cloudflare {
        assert_eq!(stats.request_id.as_deref(), Some("test-ray"));
    }
    assert_eq!(stats.attempts, 1);
    handle.join().unwrap();
}

#[test]
fn workers_ai_round_trip_uses_actual_routing_and_authorization() {
    provider_round_trip(true);
}

#[test]
fn ollama_round_trip_never_transmits_supplied_credentials() {
    provider_round_trip(false);
}
