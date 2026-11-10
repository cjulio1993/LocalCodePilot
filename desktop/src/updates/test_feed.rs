//! Local update feed for end-to-end tests between two builds of the same commit.
//!
//! Compiled only with the `update-test-feed` feature, which is rejected in
//! release builds (see `updates.rs`) and checked again by the release
//! workflow. With it, `LOCALCODEPILOT_TEST_FEED` may point the update check
//! and the trusted asset prefix to a server on the loopback address, such as
//! `desktop/examples/update_test_feed.rs`.

use std::{env, thread, time::Duration};

/// Runtime variable with the local feed address, e.g. `http://127.0.0.1:8765`.
/// The release workflow searches the published executables for this name.
pub(super) const FEED_VARIABLE: &str = "LOCALCODEPILOT_TEST_FEED";

/// The local feed address, when the variable holds an accepted loopback URL.
pub(super) fn base() -> Option<String> {
    env::var(FEED_VARIABLE)
        .ok()
        .and_then(|value| loopback_base(&value))
}

/// Accepts only `http://127.0.0.1:<port>` or `http://localhost:<port>`, with
/// at most one trailing slash: no path, user, query or fragment. Anything else
/// is ignored, so the app keeps using the official releases.
pub(super) fn loopback_base(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_suffix('/').unwrap_or(value);
    let (host, port) = value
        .strip_prefix("http://127.0.0.1:")
        .map(|port| ("127.0.0.1", port))
        .or_else(|| {
            value
                .strip_prefix("http://localhost:")
                .map(|port| ("localhost", port))
        })?;
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok().filter(|port| *port != 0)?;
    Some(format!("http://{host}:{port}"))
}

/// The version this test build reports, set when it is compiled. The E2E
/// script builds the same commit as an "old" and a "new" version.
pub(super) fn version() -> Option<&'static str> {
    option_env!("LOCALCODEPILOT_TEST_VERSION")
}

/// The Ed25519 public key (64 hex digits) that signs the local feed's checksums,
/// set when the test build is compiled: the E2E script signs with a throwaway key,
/// never with the release key.
pub(super) fn signing_key() -> Option<[u8; 32]> {
    option_env!("LOCALCODEPILOT_TEST_SIGNING_KEY")
        .map(|hex| parse_key(hex).expect("LOCALCODEPILOT_TEST_SIGNING_KEY must be 64 hex digits"))
}

fn parse_key(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

/// Simulated startup problems of a "new" test version, set when it is
/// compiled and applied only when the updater starts it:
/// `fail` closes it at once, `slow` delays it past the updater's wait.
pub(super) fn simulate_updated_startup(confirmation_timeout: Duration) {
    match option_env!("LOCALCODEPILOT_TEST_STARTUP") {
        Some("fail") => std::process::exit(3),
        Some("slow") => thread::sleep(confirmation_timeout + Duration::from_secs(10)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_loopback_address_with_a_port_is_accepted() {
        assert_eq!(
            loopback_base("http://127.0.0.1:8765").as_deref(),
            Some("http://127.0.0.1:8765")
        );
        assert_eq!(
            loopback_base(" http://localhost:9000/ ").as_deref(),
            Some("http://localhost:9000")
        );
        for refused in [
            "",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:0",
            "http://127.0.0.1:70000",
            "http://127.0.0.1:8765/releases",
            "http://127.0.0.1:8765?x=1",
            "http://127.0.0.1:8765#x",
            "http://user@127.0.0.1:8765",
            "http://127.0.0.1.example.com:8765",
            "http://localhost.example.com:8765",
            "https://127.0.0.1:8765",
            "http://192.168.0.10:8765",
            "https://github.com/cjulio1993/LocalCodePilot",
        ] {
            assert_eq!(loopback_base(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_test_signing_key_is_exactly_64_hex_digits() {
        let key = parse_key(&"0a".repeat(32)).unwrap();
        assert_eq!(key, [0x0a; 32]);
        assert_eq!(parse_key(&"0a".repeat(31)), None);
        assert_eq!(parse_key(&"zz".repeat(32)), None);
    }
}
