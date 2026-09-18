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

fn strip_terminal_sequences(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
