//! Where requests go, and the rules that keep a TypeSafe key from going somewhere else.
//!
//! # Why this is a whole module
//!
//! A base-URL override is credential exfiltration wearing a feature's clothes
//! (`docs/threat-model.md` T4). `jev` therefore treats the endpoint as a security
//! boundary rather than a configuration string:
//!
//! * The default is the official API, and nothing else is reachable by accident.
//! * A non-default endpoint must be stated explicitly by the user, is reported by
//!   `jev doctor`, and warns on every use.
//! * A non-default endpoint uses a *different* credential variable, so a stored
//!   production key can never be sent to an arbitrary host.
//! * Plain HTTP is refused except on loopback, where there is no network to observe.
//!
//! The URL parser here is deliberately small and hand-written rather than pulled from a
//! dependency: the only URLs `jev` builds are `<scheme>://<host>[:<port>]<path>`, the
//! grammar is a few lines, and it is fuzzed and property-tested. A general URL crate
//! would be a larger attack surface for a smaller job.

use std::fmt;

/// The official TypeSafe API base URL.
///
/// From <https://docs.typesafe.ai/api>. The official SDKs use the same value.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";

/// Path of the System One evaluation endpoint, relative to the base URL.
pub const SYSTEM_ONE_PATH: &str = "/v1/systemone";

/// Path of the model-listing endpoint, relative to the base URL.
pub const MODELS_PATH: &str = "/v1/models";

/// Longest base URL accepted. A client-side bound, not an API rule.
const MAX_URL_LEN: usize = 2048;

/// Reasons a base URL is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointError {
    /// The string was empty.
    #[error("an endpoint URL must not be empty")]
    Empty,
    /// The string was longer than the client's URL length bound.
    #[error("an endpoint URL must be at most {MAX_URL_LEN} characters")]
    TooLong,
    /// The URL had no `scheme://` prefix, or one this client does not speak.
    #[error("an endpoint URL must start with `https://`, or `http://` for a loopback address")]
    UnsupportedScheme,
    /// The URL used `http://` for a host that is not loopback.
    #[error(
        "refusing to send a credential over plain HTTP to {host:?}; \
         use https://, or a loopback address for local development"
    )]
    InsecureScheme {
        /// The host that was rejected.
        host: String,
    },
    /// The URL carried `user:password@`, which would put a secret in a URL.
    #[error("an endpoint URL must not contain userinfo")]
    ContainsUserinfo,
    /// The URL carried a query string or fragment, which `jev` would have to drop.
    #[error("an endpoint URL must not contain a query string or fragment")]
    ContainsQueryOrFragment,
    /// The host part was missing or malformed.
    #[error("an endpoint URL must contain a host")]
    MissingHost,
    /// The host or path contained a character that is not allowed there.
    #[error("an endpoint URL contains an invalid character")]
    InvalidCharacter,
    /// The port was not a number in `1..=65535`.
    #[error("an endpoint URL contains an invalid port")]
    InvalidPort,
    /// A Cloudflare account must be one canonical path component.
    #[error("a Cloudflare account id must contain exactly 32 hexadecimal characters")]
    InvalidCloudflareAccount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Protocol {
    SystemOne,
    Cloudflare { account_id: String },
    Ollama,
    LlamaCpp,
    HuggingFace,
}

/// A validated API base URL.
///
/// # Examples
///
/// ```
/// use jev_client::Endpoint;
///
/// let official = Endpoint::official();
/// assert!(official.is_official());
/// assert_eq!(official.url_for("/v1/models"), "https://api.typesafe.ai/v1/models");
///
/// // Plain HTTP to a public host is refused: it would expose the credential.
/// assert!(Endpoint::parse("http://example.com").is_err());
/// // Loopback is allowed, for a local mock or proxy during development.
/// assert!(Endpoint::parse("http://127.0.0.1:8080").is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The normalized base, with no trailing slash.
    base: String,
    host: String,
    secure: bool,
    protocol: Protocol,
}

impl Endpoint {
    /// The official TypeSafe endpoint.
    #[must_use]
    pub fn official() -> Self {
        // The default is a compile-time constant known to parse; falling back to a
        // hand-built value keeps this function total rather than panicking.
        Self::parse(DEFAULT_BASE_URL).unwrap_or_else(|_| Self {
            base: DEFAULT_BASE_URL.to_owned(),
            host: "api.typesafe.ai".to_owned(),
            secure: true,
            protocol: Protocol::SystemOne,
        })
    }

    /// Parses and validates a base URL.
    ///
    /// # Errors
    ///
    /// Returns [`EndpointError`] for anything that is not an absolute `https` URL — or
    /// an `http` URL whose host is a loopback address.
    pub fn parse(raw: &str) -> Result<Self, EndpointError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(EndpointError::Empty);
        }
        if raw.len() > MAX_URL_LEN {
            return Err(EndpointError::TooLong);
        }
        if raw.chars().any(|c| c.is_control() || c == ' ') {
            return Err(EndpointError::InvalidCharacter);
        }

        let (scheme, rest) = split_scheme(raw)?;
        let secure = match scheme {
            "https" => true,
            "http" => false,
            _ => return Err(EndpointError::UnsupportedScheme),
        };

        // Split before any query or fragment so that a `?` inside them cannot be
        // mistaken for part of the authority.
        if rest.contains('?') || rest.contains('#') {
            return Err(EndpointError::ContainsQueryOrFragment);
        }

        let (authority, path) = match rest.find('/') {
            Some(index) => rest.split_at(index),
            None => (rest, ""),
        };

        if authority.contains('@') {
            return Err(EndpointError::ContainsUserinfo);
        }
        let (host, port) = split_host_port(authority)?;
        if host.is_empty() {
            return Err(EndpointError::MissingHost);
        }
        if !host_is_valid(host) {
            return Err(EndpointError::InvalidCharacter);
        }
        if let Some(port) = port {
            let parsed: u32 = port.parse().map_err(|_| EndpointError::InvalidPort)?;
            if !(1..=65_535).contains(&parsed) {
                return Err(EndpointError::InvalidPort);
            }
        }

        if !secure && !is_loopback(host) {
            return Err(EndpointError::InsecureScheme {
                host: host.to_owned(),
            });
        }

        if !path.bytes().all(is_allowed_path_byte) {
            return Err(EndpointError::InvalidCharacter);
        }

        // Normalized before anything compares it. A hostname is case-insensitive and
        // `:443` is the https default, so `https://API.TypeSafe.ai:443` is the official
        // endpoint — and treating it as a custom one would silently ignore the user's
        // `JEV_API_KEY` and tell them to set `JEV_CUSTOM_API_KEY`, which is wrong advice
        // for the host they actually named.
        let host = host.to_ascii_lowercase();
        let default_port = match scheme {
            "https" => Some("443"),
            _ => Some("80"),
        };
        // An IPv6 literal keeps its brackets. Without them the normalized form
        // `https://::1` re-parses with `:1` read as a port and `::` as the host, so the
        // endpoint `jev doctor` prints is not the one the transport would use.
        let authority_host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.clone()
        };
        let normalized_authority = match port {
            Some(port) if Some(port) == default_port => authority_host,
            Some(port) => format!("{authority_host}:{port}"),
            None => authority_host,
        };
        let normalized_path = path.trim_end_matches('/');
        Ok(Self {
            base: format!("{scheme}://{normalized_authority}{normalized_path}"),
            host,
            secure,
            protocol: Protocol::SystemOne,
        })
    }

    /// Returns `true` when this is exactly the official TypeSafe endpoint.
    ///
    /// Used to decide whether a warning is owed to the user and which credential
    /// namespace applies.
    #[must_use]
    pub fn is_official(&self) -> bool {
        self.base == DEFAULT_BASE_URL && self.protocol == Protocol::SystemOne
    }

    /// Builds a Workers AI endpoint with an explicit account.
    ///
    /// # Errors
    /// Returns [`EndpointError::InvalidCloudflareAccount`] for a malformed account id.
    pub fn cloudflare(account_id: &str) -> Result<Self, EndpointError> {
        Self::parse("https://api.cloudflare.com")?.with_cloudflare_account(account_id)
    }

    /// Uses Workers AI routing against this explicitly selected base URL.
    ///
    /// # Errors
    /// Returns [`EndpointError::InvalidCloudflareAccount`] for a malformed account id.
    pub fn with_cloudflare_account(mut self, account_id: &str) -> Result<Self, EndpointError> {
        if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(EndpointError::InvalidCloudflareAccount);
        }
        self.protocol = Protocol::Cloudflare {
            account_id: account_id.to_ascii_lowercase(),
        };
        Ok(self)
    }

    /// Whether requests use the Workers AI protocol.
    #[must_use]
    pub const fn is_cloudflare(&self) -> bool {
        matches!(self.protocol, Protocol::Cloudflare { .. })
    }

    /// The account used for Workers AI routing.
    #[must_use]
    pub fn cloudflare_account_id(&self) -> Option<&str> {
        match &self.protocol {
            Protocol::Cloudflare { account_id } => Some(account_id),
            _ => None,
        }
    }

    /// The selected protocol provider.
    #[must_use]
    pub const fn provider(&self) -> &'static str {
        match self.protocol {
            Protocol::SystemOne => "typesafe",
            Protocol::Cloudflare { .. } => "cloudflare",
            Protocol::Ollama => "ollama",
            Protocol::LlamaCpp => "llamacpp",
            Protocol::HuggingFace => "huggingface",
        }
    }

    /// Whether the selected provider runs an explicitly addressed local server.
    #[must_use]
    pub const fn is_local_provider(&self) -> bool {
        matches!(
            self.protocol,
            Protocol::Ollama | Protocol::LlamaCpp | Protocol::HuggingFace
        )
    }

    /// The conventional Ollama loopback endpoint.
    #[must_use]
    pub fn ollama() -> Self {
        Self {
            base: "http://127.0.0.1:11434".to_owned(),
            host: "127.0.0.1".to_owned(),
            secure: false,
            protocol: Protocol::Ollama,
        }
    }

    /// The conventional llama.cpp loopback endpoint.
    #[must_use]
    pub fn llama_cpp() -> Self {
        Self {
            base: "http://127.0.0.1:8080".to_owned(),
            host: "127.0.0.1".to_owned(),
            secure: false,
            protocol: Protocol::LlamaCpp,
        }
    }

    /// The loopback endpoint of this repository's explicit Python Clef bridge.
    #[must_use]
    pub fn huggingface() -> Self {
        Self {
            base: "http://127.0.0.1:8787".to_owned(),
            host: "127.0.0.1".to_owned(),
            secure: false,
            protocol: Protocol::HuggingFace,
        }
    }

    /// Uses this repository's Python Clef bridge protocol at an explicit endpoint.
    #[must_use]
    pub fn with_huggingface(mut self) -> Self {
        self.protocol = Protocol::HuggingFace;
        self
    }

    /// Uses the Ollama protocol against this explicitly selected endpoint.
    #[must_use]
    pub fn with_ollama(mut self) -> Self {
        self.protocol = Protocol::Ollama;
        self
    }

    /// Uses the llama.cpp System One protocol against this explicitly selected endpoint.
    #[must_use]
    pub fn with_llama_cpp(mut self) -> Self {
        self.protocol = Protocol::LlamaCpp;
        self
    }

    /// The normalized base URL, without a trailing slash.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The lowercased host.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether the endpoint names this machine unambiguously.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        is_loopback(&self.host)
    }

    /// Returns `true` when the transport will use TLS.
    #[must_use]
    pub const fn is_secure(&self) -> bool {
        self.secure
    }

    /// Joins an absolute path onto the base.
    #[must_use]
    pub fn url_for(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

impl Default for Endpoint {
    fn default() -> Self {
        Self::official()
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base)
    }
}

fn split_scheme(raw: &str) -> Result<(&str, &str), EndpointError> {
    let index = raw.find("://").ok_or(EndpointError::UnsupportedScheme)?;
    let scheme = raw.get(..index).ok_or(EndpointError::UnsupportedScheme)?;
    let rest = raw
        .get(index + 3..)
        .ok_or(EndpointError::UnsupportedScheme)?;
    Ok((scheme, rest))
}

/// Splits `host:port`, handling the bracketed IPv6 form.
fn split_host_port(authority: &str) -> Result<(&str, Option<&str>), EndpointError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']').ok_or(EndpointError::InvalidCharacter)?;
        let host = rest.get(..close).ok_or(EndpointError::InvalidCharacter)?;
        let after = rest
            .get(close + 1..)
            .ok_or(EndpointError::InvalidCharacter)?;
        if after.is_empty() {
            return Ok((host, None));
        }
        let port = after.strip_prefix(':').ok_or(EndpointError::InvalidPort)?;
        return Ok((host, Some(port)));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((host, Some(port))),
        None => Ok((authority, None)),
    }
}

/// Accepts DNS names and bare IP literals, and nothing that could be misread by a
/// downstream HTTP library as a different authority.
fn host_is_valid(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'_'))
}

fn is_allowed_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~' | b'%')
}

/// Whether a host refers to this machine.
///
/// Only the unambiguous forms are accepted. Names such as `localhost.evil.example`
/// resolve elsewhere, and treating them as local would defeat the check entirely.
/// Whether a host is *unambiguously* loopback.
///
/// "Unambiguously" is the whole point, and it is why this only accepts canonical
/// dotted-quad decimal. `jev` decides whether cleartext is safe by reading the host
/// string; the operating system then resolves that same string to decide where the
/// bytes actually go. If the two readings can disagree, the decision is worthless.
///
/// They can. `getaddrinfo` follows `inet_aton`, where a leading zero means **octal**:
///
/// ```text
/// host string                    this function (naively)   getaddrinfo
/// 0000127.00000000000012.077.2   127.12.77.2  (loopback)   87.10.63.2  (public)
/// ```
///
/// `"0000127".parse::<u8>()` is `Ok(127)` — Rust ignores leading zeros — so a naive
/// check calls that host loopback and permits plain HTTP, and the credential then
/// crosses the public internet in cleartext. Found by the `endpoint_url` fuzz target.
///
/// So an octet is accepted only as one to three ASCII digits with no leading zero
/// (except `"0"` itself), which is the one spelling every resolver reads the same way.
/// `0177.0.0.1` is therefore rejected even though it *does* resolve to loopback: a
/// false negative costs a developer one retype, and a false positive leaks a key.
fn is_loopback(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host == "::1" || host == "[::1]" {
        return true;
    }
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() != 4 {
        return false;
    }
    let mut parsed = [0_u8; 4];
    for (slot, text) in parsed.iter_mut().zip(octets) {
        if !is_canonical_octet(text) {
            return false;
        }
        match text.parse::<u8>() {
            Ok(value) => *slot = value,
            Err(_) => return false,
        }
    }
    // 127.0.0.0/8.
    parsed.first() == Some(&127)
}

/// One to three ASCII digits, with no leading zero unless the octet is exactly `"0"`.
fn is_canonical_octet(text: &str) -> bool {
    if text.is_empty() || text.len() > 3 {
        return false;
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    // `"0"` is canonical; `"00"` and `"0127"` are not, because a resolver reads the
    // leading zero as an octal marker.
    text == "0" || !text.starts_with('0')
}

#[cfg(test)]
mod tests {
    #[test]
    fn huggingface_bridge_stays_in_the_custom_credential_namespace() {
        let endpoint = Endpoint::huggingface();
        assert_eq!(endpoint.base(), "http://127.0.0.1:8787");
        assert_eq!(endpoint.provider(), "huggingface");
        assert!(endpoint.is_local_provider());
        assert!(endpoint.is_loopback());
        assert!(!endpoint.is_official());
        assert!(!Endpoint::official().with_huggingface().is_official());
    }
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn cloudflare_protocol_never_selects_typesafe_credentials() {
        let account = "0123456789abcdef0123456789ABCDEF";
        let endpoint = Endpoint::cloudflare(account).unwrap();
        assert!(endpoint.is_cloudflare());
        assert!(!endpoint.is_official());
        assert_eq!(endpoint.provider(), "cloudflare");
        assert_eq!(
            endpoint.cloudflare_account_id(),
            Some("0123456789abcdef0123456789abcdef")
        );
        let overridden = Endpoint::official()
            .with_cloudflare_account(account)
            .unwrap();
        assert!(!overridden.is_official());
        for rejected in [
            "",
            "../api",
            "not-a-valid-account-id",
            "0123456789abcdef0123456789abcdeg",
        ] {
            let error = Endpoint::cloudflare(rejected).unwrap_err();
            assert!(!error.to_string().contains("not-a-valid-account-id"));
        }
    }

    #[test]
    fn the_default_is_the_official_endpoint() {
        let endpoint = Endpoint::official();
        assert!(endpoint.is_official());
        assert!(endpoint.is_secure());
        assert_eq!(endpoint.base(), "https://api.typesafe.ai");
        assert_eq!(
            endpoint.url_for(SYSTEM_ONE_PATH),
            "https://api.typesafe.ai/v1/systemone"
        );
    }

    /// An octet with a leading zero is octal to a resolver and decimal to Rust.
    ///
    /// `http://0000127.00000000000012.077.2` looked like loopback to the old check —
    /// `"0000127".parse::<u8>()` is `Ok(127)` — while `getaddrinfo` reads the leading
    /// zeros as octal and resolves it to **87.10.63.2**, a public address. Accepting it
    /// meant sending a credential across the internet in cleartext. Found by the
    /// `endpoint_url` fuzz target; the input is kept in `fuzz/seeds/endpoint_url/`.
    #[test]
    fn an_octal_ambiguous_host_is_not_loopback() {
        for host in [
            "0000127.00000000000012.077.2",
            "0177.0.0.1",
            "127.0.0.01",
            "00.0.0.0",
            "0127.0.0.1",
            "127.000.000.001",
        ] {
            assert!(
                !is_loopback(host),
                "{host} was accepted as loopback, but a resolver may read it as octal"
            );
            assert_eq!(
                Endpoint::parse(&format!("http://{host}")),
                Err(EndpointError::InsecureScheme {
                    host: host.to_owned()
                }),
                "cleartext was permitted to {host}"
            );
        }
    }

    /// And the canonical spellings still work, so the fix is not a blanket refusal.
    #[test]
    fn canonical_loopback_addresses_are_still_accepted() {
        for host in ["127.0.0.1", "127.1.2.3", "127.255.255.255", "localhost"] {
            assert!(is_loopback(host), "{host} should be loopback");
            assert!(
                Endpoint::parse(&format!("http://{host}:8080")).is_ok(),
                "{host}"
            );
        }
        assert!(Endpoint::parse("http://[::1]:9443").is_ok());
        // Adjacent but not loopback.
        assert!(!is_loopback("128.0.0.1"));
        assert!(!is_loopback("126.255.255.255"));
        assert!(!is_loopback("127.0.0"));
        assert!(!is_loopback("127.0.0.1.1"));
        assert!(!is_loopback("127.0.0.256"));
    }

    #[test]
    fn plain_http_to_a_public_host_is_refused() {
        // The whole point of the module: a credential must not cross a plaintext link.
        assert_eq!(
            Endpoint::parse("http://api.typesafe.ai"),
            Err(EndpointError::InsecureScheme {
                host: "api.typesafe.ai".to_owned()
            })
        );
    }

    #[test]
    fn loopback_over_plain_http_is_allowed_for_local_development() {
        for url in [
            "http://127.0.0.1:8080",
            "http://127.9.9.9",
            "http://localhost:3000",
            "http://[::1]:9000",
        ] {
            assert!(Endpoint::parse(url).is_ok(), "rejected {url}");
        }
    }

    #[test]
    fn hosts_that_only_look_local_are_not_loopback() {
        // `localhost.evil.example` and `127.0.0.1.evil.example` resolve to an attacker.
        for url in [
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "http://notlocalhost",
        ] {
            assert!(
                matches!(
                    Endpoint::parse(url),
                    Err(EndpointError::InsecureScheme { .. })
                ),
                "accepted {url}"
            );
        }
    }

    #[test]
    fn userinfo_is_refused() {
        assert_eq!(
            Endpoint::parse("https://user:password@example.com"),
            Err(EndpointError::ContainsUserinfo)
        );
    }

    #[test]
    fn query_and_fragment_are_refused() {
        // `jev` appends a path; silently dropping a query the user wrote would send the
        // request somewhere other than where they asked.
        assert_eq!(
            Endpoint::parse("https://example.com/base?token=abc"),
            Err(EndpointError::ContainsQueryOrFragment)
        );
        assert_eq!(
            Endpoint::parse("https://example.com/base#frag"),
            Err(EndpointError::ContainsQueryOrFragment)
        );
    }

    #[test]
    fn other_schemes_are_refused() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com",
            "gopher://example.com",
            "javascript:alert(1)",
            "api.typesafe.ai",
            "//api.typesafe.ai",
        ] {
            assert!(Endpoint::parse(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn the_official_endpoint_survives_case_and_an_explicit_default_port() {
        // Otherwise the user's JEV_API_KEY is silently ignored and they are told to set
        // JEV_CUSTOM_API_KEY for a host that *is* the official one.
        for raw in [
            "https://API.TYPESAFE.AI",
            "https://Api.TypeSafe.ai",
            "https://api.typesafe.ai:443",
            "https://API.TypeSafe.ai:443/",
        ] {
            let endpoint = Endpoint::parse(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(endpoint.is_official(), "{raw} was treated as non-official");
            assert_eq!(endpoint.base(), DEFAULT_BASE_URL);
        }
        // A non-default port is a different endpoint and must stay one.
        assert!(
            !Endpoint::parse("https://api.typesafe.ai:8443")
                .unwrap()
                .is_official()
        );
        // And a lookalike host is still not official.
        assert!(
            !Endpoint::parse("https://api.typesafe.ai.evil.com")
                .unwrap()
                .is_official()
        );
    }

    #[test]
    fn a_bracketed_ipv6_host_keeps_its_brackets() {
        // Found by the idempotence property: without the brackets the normalized form
        // re-parses with `:1` as a port and `::` as the host, so what `jev doctor`
        // reports would not be what the transport uses.
        for raw in ["http://[::1]", "http://[::1]:9000", "http://[::1]:80"] {
            let endpoint = Endpoint::parse(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(
                endpoint.base().contains("[::1]"),
                "{raw} normalized to {}",
                endpoint.base()
            );
            assert_eq!(
                Endpoint::parse(endpoint.base()).as_ref(),
                Ok(&endpoint),
                "{raw} is not idempotent"
            );
        }
    }

    #[test]
    fn the_default_http_port_is_also_normalized_for_loopback() {
        let endpoint = Endpoint::parse("http://127.0.0.1:80").unwrap();
        assert_eq!(endpoint.base(), "http://127.0.0.1");
    }

    #[test]
    fn a_trailing_slash_is_normalized_away() {
        assert_eq!(
            Endpoint::parse("https://api.typesafe.ai/").unwrap().base(),
            "https://api.typesafe.ai"
        );
        // …and the normalized form is still recognized as the official endpoint, so a
        // user who types the trailing slash is not warned at spuriously.
        assert!(
            Endpoint::parse("https://api.typesafe.ai/")
                .unwrap()
                .is_official()
        );
    }

    #[test]
    fn a_base_path_is_preserved_for_proxies() {
        let endpoint = Endpoint::parse("https://proxy.example.com/typesafe").unwrap();
        assert_eq!(
            endpoint.url_for(SYSTEM_ONE_PATH),
            "https://proxy.example.com/typesafe/v1/systemone"
        );
        assert!(!endpoint.is_official());
    }

    #[test]
    fn control_characters_and_spaces_are_refused() {
        // A newline in a URL is a request-splitting primitive if it reaches a client
        // that does not check.
        for url in [
            "https://example.com/\r\nHost: evil",
            "https://exa mple.com",
            "https://example.com/\u{0}",
        ] {
            assert!(Endpoint::parse(url).is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn invalid_ports_are_refused() {
        assert_eq!(
            Endpoint::parse("https://example.com:0"),
            Err(EndpointError::InvalidPort)
        );
        assert_eq!(
            Endpoint::parse("https://example.com:99999"),
            Err(EndpointError::InvalidPort)
        );
        assert_eq!(
            Endpoint::parse("https://example.com:http"),
            Err(EndpointError::InvalidPort)
        );
    }

    #[test]
    fn an_over_long_url_is_refused() {
        let url = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(Endpoint::parse(&url), Err(EndpointError::TooLong));
    }

    /// URL-shaped input, so the properties below actually reach the parser's success
    /// path.
    ///
    /// Random text never starts with `https://`, so a generator of arbitrary strings
    /// exercises only the rejection branch — the properties look thorough and assert
    /// nothing. This composes a scheme, a host, an optional port and a path from sets
    /// that mix the valid with the hostile.
    fn url() -> impl Strategy<Value = String> {
        let scheme = prop::sample::select(vec!["https", "http", "HTTPS", "ftp", "file", ""]);
        let host = prop::sample::select(vec![
            "api.typesafe.ai",
            "API.TypeSafe.ai",
            "api.typesafe.ai.evil.com",
            "localhost",
            "localhost.evil.example",
            "127.0.0.1",
            "127.9.9.9",
            "127.0.0.1.evil.example",
            "[::1]",
            "example.com",
            "",
            "exa mple.com",
            "user@example.com",
        ]);
        let port = prop::sample::select(vec!["", ":443", ":80", ":8443", ":0", ":99999", ":http"]);
        let path =
            prop::sample::select(vec!["", "/", "/v1", "/base/", "/a?b=c", "/a#b", "/\u{1b}"]);
        (scheme, host, port, path)
            .prop_map(|(scheme, host, port, path)| format!("{scheme}://{host}{port}{path}"))
    }

    proptest! {
        /// Parsing must be total: no input shape may panic.
        #[test]
        fn parsing_never_panics(raw in ".{0,300}") {
            let _ = Endpoint::parse(&raw);
        }

        /// Totality again, over input that is actually URL-shaped.
        #[test]
        fn parsing_url_shaped_input_never_panics(raw in url()) {
            let _ = Endpoint::parse(&raw);
        }

        /// Anything that parses must round-trip to itself, so the URL `jev doctor`
        /// reports is the URL the transport will use.
        #[test]
        fn parsed_endpoints_are_idempotent(raw in url()) {
            if let Ok(endpoint) = Endpoint::parse(&raw) {
                let reparsed = Endpoint::parse(endpoint.base());
                prop_assert_eq!(Ok(endpoint), reparsed);
            }
        }

        /// The security invariant, stated as a property rather than as examples: an
        /// endpoint that parses is either TLS-protected or unambiguously loopback.
        #[test]
        fn every_accepted_endpoint_is_tls_or_loopback(raw in url()) {
            if let Ok(endpoint) = Endpoint::parse(&raw) {
                prop_assert!(
                    endpoint.is_secure() || is_loopback(endpoint.host()),
                    "accepted a cleartext non-loopback endpoint: {}",
                    endpoint.base()
                );
            }
        }

        /// Only the official host is ever official, however it is spelled. A lookalike
        /// must never be, and the official one must survive case and `:443`.
        #[test]
        fn only_the_official_host_is_official(raw in url()) {
            if let Ok(endpoint) = Endpoint::parse(&raw)
                && endpoint.is_official()
            {
                prop_assert_eq!(endpoint.host(), "api.typesafe.ai");
                prop_assert!(endpoint.is_secure());
                prop_assert_eq!(endpoint.base(), DEFAULT_BASE_URL);
            }
        }
    }
}
