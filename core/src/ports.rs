#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Port(u16);

impl Port {
    pub fn new(value: u16) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }
    pub fn value(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalServerUrl {
    address: String,
    port: Port,
}

impl LocalServerUrl {
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn port(&self) -> Port {
        self.port
    }
}

pub fn extract_local_urls(text: &str) -> Vec<LocalServerUrl> {
    let clean_text = strip_terminal_sequences(text);
    let text = clean_text.as_str();
    let mut urls = Vec::new();
    let mut cursor = 0;

    while cursor < text.len() {
        let remaining = &text[cursor..];
        let http = remaining.find("http://");
        let https = remaining.find("https://");
        let Some(start) = (match (http, https) {
            (Some(http), Some(https)) => Some(http.min(https)),
            (Some(http), None) => Some(http),
            (None, Some(https)) => Some(https),
            (None, None) => None,
        }) else {
            break;
        };
        let absolute_start = cursor + start;
        let candidate = &text[absolute_start..];
        let end = candidate
            .find(|character: char| {
                character.is_whitespace()
                    || character.is_control()
                    || matches!(character, '"' | '\'' | '<' | '>')
            })
            .unwrap_or(candidate.len());
        let candidate = candidate[..end].trim_end_matches(['.', ',', ';', ')', ']', '}']);
        if let Some(url) = parse_local_url(candidate)
            && !urls.contains(&url)
        {
            urls.push(url);
        }
        cursor = absolute_start + end.max(1);
    }
    urls
}

pub fn strip_terminal_sequences(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            clean.push(character);
            continue;
        }

        match characters.next() {
            Some('[') => {
                for control in characters.by_ref() {
                    if ('@'..='~').contains(&control) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(control) = characters.next() {
                    if control == '\u{7}' {
                        break;
                    }
                    if control == '\u{1b}' && characters.next_if_eq(&'\\').is_some() {
                        break;
                    }
                }
            }
            Some(_) | None => {}
        }
    }
    clean
}

fn parse_local_url(candidate: &str) -> Option<LocalServerUrl> {
    let (scheme, remainder, default_port) =
        if let Some(remainder) = candidate.strip_prefix("http://") {
            ("http://", remainder, 80)
        } else {
            let remainder = candidate.strip_prefix("https://")?;
            ("https://", remainder, 443)
        };
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let closing_bracket = ipv6.find(']')?;
        let host = &ipv6[..closing_bracket];
        let suffix = &ipv6[closing_bracket + 1..];
        let port = if suffix.is_empty() {
            default_port
        } else {
            suffix.strip_prefix(':')?.parse::<u16>().ok()?
        };
        (host, port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, port.parse::<u16>().ok()?)
    } else {
        (authority, default_port)
    };
    if !matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "::") {
        return None;
    }
    let port = Port::new(port)?;
    let browser_authority = match host {
        "0.0.0.0" | "::" => format!("localhost:{}", port.value()),
        "::1" => format!("[::1]:{}", port.value()),
        _ => format!("{host}:{}", port.value()),
    };
    let suffix = &remainder[authority_end..];
    Some(LocalServerUrl {
        address: format!("{scheme}{browser_authority}{suffix}"),
        port,
    })
}

/// Whether a server's log says its port was already taken, and which port when the message
/// names it. Each runtime words it differently: Node `EADDRINUSE ... :::3000`, Spring `Port
/// 8080 was already in use`, Flask `Port 5000 is in use by another program`, PHP `Failed to
/// listen on localhost:8000`, Rust, Go and Python `address already in use` (Windows error
/// 10048, translated by the system).
pub fn port_in_use(lines: &[&str]) -> Option<Option<u16>> {
    let mut found = false;
    let mut port = None;
    for line in lines {
        let lower = strip_terminal_sequences(line).to_ascii_lowercase();
        let in_use = lower.contains("eaddrinuse")
            || lower.contains("address already in use")
            || lower.contains("already in use")
            || lower.contains("is in use by another program")
            || lower.contains("failed to listen on")
            || lower.contains("os error 10048")
            || lower.contains("os error 98)")
            || lower.contains("os error 48)");
        if !in_use {
            continue;
        }
        found = true;
        port = named_port(&lower).or(port);
    }
    found.then_some(port)
}

/// The port in `...:3000`, `port 8080` or `localhost:8000`, ignoring OS error codes.
fn named_port(line: &str) -> Option<u16> {
    let after_port = line.split("port ").skip(1).filter_map(leading_number);
    let after_colon = line.split(':').skip(1).filter_map(leading_number);
    after_port
        .chain(after_colon)
        .filter(|port| *port >= 80)
        .last()
}

fn leading_number(text: &str) -> Option<u16> {
    let digits = text
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok().filter(|port| *port > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_taken_port_in_each_runtime_log() {
        let cases: [(&[&str], Option<Option<u16>>); 8] = [
            (
                &["Error: listen EADDRINUSE: address already in use :::3000"],
                Some(Some(3000)),
            ),
            (
                &["Web server failed to start. Port 8080 was already in use."],
                Some(Some(8080)),
            ),
            (
                &[
                    "Address already in use",
                    "Port 5000 is in use by another program.",
                ],
                Some(Some(5000)),
            ),
            (
                &["[Mon] Failed to listen on localhost:8000 (reason: Address already in use)"],
                Some(Some(8000)),
            ),
            (
                &["Error: Os { code: 10048, kind: AddrInUse, message: \"...\" } (os error 10048)"],
                Some(None),
            ),
            (&["OSError: [Errno 98] Address already in use"], Some(None)),
            (&["listening on port 3000"], None),
            (&["Server running at http://localhost:3000"], None),
        ];
        for (lines, expected) in cases {
            assert_eq!(port_in_use(lines), expected, "{lines:?}");
        }
    }

    #[test]
    fn extracts_local_urls_from_server_output() {
        let urls = extract_local_urls(
            "Local: http://localhost:5173/\nServer running on [http://127.0.0.1:8000].",
        );

        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].address(), "http://localhost:5173/");
        assert_eq!(urls[0].port().value(), 5173);
        assert_eq!(urls[1].address(), "http://127.0.0.1:8000");
    }

    #[test]
    fn converts_wildcard_hosts_to_localhost() {
        let urls = extract_local_urls("Listening at http://0.0.0.0:3000");

        assert_eq!(urls[0].address(), "http://localhost:3000");
    }

    #[test]
    fn removes_terminal_colors_before_reading_the_port() {
        let urls =
            extract_local_urls("\u{1b}[36mhttp://localhost:\u{1b}[1m5173\u{1b}[22m/\u{1b}[39m");

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].address(), "http://localhost:5173/");
        assert_eq!(urls[0].port().value(), 5173);
    }

    #[test]
    fn ignores_external_urls() {
        assert!(extract_local_urls("Docs: https://example.com:443/help").is_empty());
    }
}
