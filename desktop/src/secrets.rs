//! Masks values that look like secrets in log lines saved to disk or copied.
//! The live output stays untouched; this only covers known patterns:
//! `NAME=value` / `NAME: value` where the name mentions a password, token,
//! secret or key; `Bearer value`; and `scheme://user:password@host`.

/// What replaces a masked value.
pub const MASK: &str = "••••";

const SENSITIVE_NAMES: [&str; 5] = ["password", "passwd", "secret", "token", "key"];

/// The line with every recognised secret value replaced by [`MASK`]. The
/// names, the separators and the rest of the line stay readable.
pub fn mask_secrets(line: &str) -> String {
    let mut ranges = Vec::new();
    url_credentials(line, &mut ranges);
    bearer_tokens(line, &mut ranges);
    assignments(line, &mut ranges);
    if ranges.is_empty() {
        return line.to_owned();
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    let mut masked = String::with_capacity(line.len());
    let mut cursor = 0;
    for (start, end) in merged {
        masked.push_str(&line[cursor..start]);
        masked.push_str(MASK);
        cursor = end;
    }
    masked.push_str(&line[cursor..]);
    masked
}

/// Masks every line.
pub fn mask_lines(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| mask_secrets(line)).collect()
}

/// Whether command arguments seem to carry a secret: one of the masked
/// patterns, or an option named like a secret followed by its value
/// (`--token abc`).
pub fn arguments_look_secret(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, argument)| {
        if mask_secrets(argument) != *argument {
            return true;
        }
        let option = argument.trim_start_matches('-');
        argument.starts_with('-')
            && !option.contains('=')
            && is_sensitive_name(option)
            && args
                .get(index + 1)
                .is_some_and(|value| !value.is_empty() && !value.starts_with('-'))
    })
}

fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SENSITIVE_NAMES
        .iter()
        .any(|sensitive| name.contains(sensitive))
}

fn is_name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}

/// End (exclusive) of an unquoted value starting at `start`.
fn value_end(line: &str, start: usize) -> usize {
    line[start..]
        .char_indices()
        .find(|(_, character)| {
            character.is_whitespace()
                || matches!(
                    character,
                    ',' | ';' | '&' | '"' | '\'' | '<' | '>' | ')' | ']' | '}'
                )
        })
        .map_or(line.len(), |(offset, _)| start + offset)
}

fn skip_blanks(line: &str, start: usize) -> usize {
    let rest = &line[start..];
    start + rest.len() - rest.trim_start_matches([' ', '\t']).len()
}

/// `scheme://user:password@host`: masks the password.
fn url_credentials(line: &str, ranges: &mut Vec<(usize, usize)>) {
    let mut from = 0;
    while let Some(found) = line[from..].find("://") {
        let authority_start = from + found + 3;
        let authority_end = line[authority_start..]
            .find(|character: char| {
                character == '/' || character.is_whitespace() || matches!(character, '"' | '\'')
            })
            .map_or(line.len(), |offset| authority_start + offset);
        let authority = &line[authority_start..authority_end];
        if let Some(at) = authority.rfind('@')
            && let Some(colon) = authority[..at].find(':')
            && colon + 1 < at
        {
            ranges.push((authority_start + colon + 1, authority_start + at));
        }
        from = authority_end;
    }
}

/// `Bearer value`, with any capitalisation.
fn bearer_tokens(line: &str, ranges: &mut Vec<(usize, usize)>) {
    // ASCII lowercasing keeps every byte offset.
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("bearer") {
        let start = from + found;
        let after = start + "bearer".len();
        from = after;
        let at_word_start = line[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric());
        let value_start = skip_blanks(line, after);
        if !at_word_start || value_start == after {
            continue;
        }
        let end = value_end(line, value_start);
        if end > value_start {
            ranges.push((value_start, end));
        }
    }
}

/// `NAME=value`, `NAME: value`, `"name": "value"` and `--password=value`.
fn assignments(line: &str, ranges: &mut Vec<(usize, usize)>) {
    for (index, separator) in line.char_indices() {
        if !matches!(separator, '=' | ':') {
            continue;
        }
        let rest = &line[index + 1..];
        let before = &line[..index];
        // Comparisons (`==`, `!=`, `<=`) and URLs (`https://`) are not assignments.
        if rest.starts_with('=')
            || before.ends_with(['=', '!', '<', '>'])
            || (separator == ':' && rest.starts_with("//"))
        {
            continue;
        }
        let before = before.trim_end_matches([' ', '\t']);
        let before = before.strip_suffix(['"', '\'']).unwrap_or(before);
        let name_start = before
            .char_indices()
            .rev()
            .find(|(_, character)| !is_name_character(*character))
            .map_or(0, |(offset, character)| offset + character.len_utf8());
        let name = &before[name_start..];
        if name.is_empty() || !is_sensitive_name(name) {
            continue;
        }
        let value_start = skip_blanks(line, index + 1);
        let quote = line[value_start..]
            .chars()
            .next()
            .filter(|character| matches!(character, '"' | '\''));
        let (start, end) = match quote {
            Some(quote) => {
                let start = value_start + 1;
                let end = line[start..]
                    .find(quote)
                    .map_or(line.len(), |offset| start + offset);
                (start, end)
            }
            None => (value_start, value_end(line, value_start)),
        };
        if end > start {
            ranges.push((start, end));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_assignments_with_sensitive_names_in_any_case() {
        assert_eq!(
            mask_secrets("DB_PASSWORD=valor-ficticio pronto"),
            "DB_PASSWORD=•••• pronto"
        );
        assert_eq!(mask_secrets("api_token: abc123"), "api_token: ••••");
        assert_eq!(mask_secrets("Client-Secret = s3"), "Client-Secret = ••••");
        assert_eq!(mask_secrets("--password=ficticio"), "--password=••••");
        assert_eq!(
            mask_secrets(r#"{"apiKey": "valor ficticio", "port": 3000}"#),
            r#"{"apiKey": "••••", "port": 3000}"#
        );
    }

    #[test]
    fn masks_bearer_tokens_and_url_credentials() {
        assert_eq!(
            mask_secrets("Authorization: Bearer eyJ.ficticio"),
            "Authorization: Bearer ••••"
        );
        assert_eq!(
            mask_secrets("conectando a postgres://usuario:senha-ficticia@localhost:5432/app"),
            "conectando a postgres://usuario:••••@localhost:5432/app"
        );
    }

    #[test]
    fn masks_several_patterns_on_the_same_line() {
        assert_eq!(
            mask_secrets("TOKEN=t1 SECRET_KEY=t2 url=https://u:p@h/x bearer t3"),
            "TOKEN=•••• SECRET_KEY=•••• url=https://u:••••@h/x bearer ••••"
        );
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            "Servidor ouvindo em http://localhost:3000",
            "GET /api/users 200 12ms",
            "if token == expected",
            "12:30:01 compilado",
            "",
            "ação concluída: ok",
        ] {
            assert_eq!(mask_secrets(line), line);
        }
    }

    #[test]
    fn masking_is_stable_when_applied_twice() {
        let once = mask_secrets("PASSWORD=abc Authorization: Bearer xyz");
        assert_eq!(mask_secrets(&once), once);
    }

    #[test]
    fn detects_secret_looking_arguments() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        };
        assert!(arguments_look_secret(&args(&["--token", "abc"])));
        assert!(arguments_look_secret(&args(&["--api-key=abc"])));
        assert!(arguments_look_secret(&args(&[
            "serve",
            "postgres://u:p@h/db"
        ])));
        assert!(!arguments_look_secret(&args(&[
            "main.lua", "--porta", "8080"
        ])));
        assert!(!arguments_look_secret(&args(&["--token"])));
        assert!(!arguments_look_secret(&args(&["--keyboard", "--verbose"])));
    }
}
