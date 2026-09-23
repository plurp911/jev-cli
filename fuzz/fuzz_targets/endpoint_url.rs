//! Fuzzes the endpoint parser.
//!
//! This is a hand-written URL parser, which is exactly the kind of code that should be
//! fuzzed. Beyond totality, it asserts the security invariant the whole module exists
//! for: **anything that parses is TLS-protected or unambiguously loopback**, so a
//! credential can never cross a cleartext link to a remote host.
#![no_main]

use jev_client::Endpoint;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(endpoint) = Endpoint::parse(text) else {
        return;
    };

    if !endpoint.is_secure() {
        let host = endpoint.host();
        let loopback = host == "localhost"
            || host == "::1"
            || host == "[::1]"
            || host.starts_with("127.");
        assert!(
            loopback,
            "accepted a cleartext non-loopback endpoint: {}",
            endpoint.base()
        );
    }

    // Reparsing the normalized form must give the same endpoint, so what `jev doctor`
    // reports is what the transport will use.
    assert_eq!(
        Endpoint::parse(endpoint.base()).ok().as_ref(),
        Some(&endpoint),
        "normalization is not idempotent for {text:?}"
    );

    // A URL that parses must produce a URL that still starts with its own base.
    let joined = endpoint.url_for("/v1/systemone");
    assert!(joined.starts_with(endpoint.base()));
});
