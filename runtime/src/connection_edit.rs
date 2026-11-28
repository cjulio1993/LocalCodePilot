//! Editing the database access data a project declares (T040): the database name, the user
//! and the password in the project's own `.env`, `.env.local` or `wp-config.php`, the same
//! file the app reads them from. Only plain literal values are changed; a value that comes
//! from a variable or an expression, or that is defined in a way that cannot be changed
//! safely, is left alone. Everything else in the file stays byte for byte.

use crate::database_connection::{dotenv_value, wordpress_define};
use std::path::Path;

/// Which access setting a field is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKey {
    Database,
    User,
    Password,
}

/// Why a field cannot be changed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldRefusal {
    /// The value comes from a variable (`${…}`) or a PHP expression.
    Variable,
    /// Defined more than once, inside a condition, or otherwise not safe to rewrite.
    Unsafe,
}

/// One access setting as the form shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionField {
    pub key: ConnectionKey,
    /// The name in the file (`DB_USERNAME`, `DB_USER`…).
    pub name: &'static str,
    /// The current literal value; never filled for the password.
    pub value: Option<String>,
    pub refusal: Option<FieldRefusal>,
}

impl std::fmt::Debug for ConnectionField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionField")
            .field("key", &self.key)
            .field("name", &self.name)
            .field(
                "value",
                &match self.key {
                    ConnectionKey::Password => None,
                    _ => self.value.as_deref(),
                },
            )
            .field("refusal", &self.refusal)
            .finish()
    }
}

/// The kind of file, by name: the formats the app reads the access data from.
fn is_wordpress(path: &Path) -> Option<bool> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    match name.as_str() {
        "wp-config.php" => Some(true),
        ".env" | ".env.local" => Some(false),
        _ => None,
    }
}

fn names(wordpress: bool) -> [(ConnectionKey, &'static str); 3] {
    if wordpress {
        [
            (ConnectionKey::Database, "DB_NAME"),
            (ConnectionKey::User, "DB_USER"),
            (ConnectionKey::Password, "DB_PASSWORD"),
        ]
    } else {
        [
            (ConnectionKey::Database, "DB_DATABASE"),
            (ConnectionKey::User, "DB_USERNAME"),
            (ConnectionKey::Password, "DB_PASSWORD"),
        ]
    }
}

/// The lines of the file that set `name`, by index.
fn defining_lines(contents: &str, wordpress: bool, name: &str) -> Vec<usize> {
    contents
        .split('\n')
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim_end_matches('\r');
            if wordpress {
                wordpress_define(line.trim_start(), name).is_some()
            } else {
                dotenv_value(line, name).is_some()
            }
        })
        .map(|(index, _)| index)
        .collect()
}

/// The fields of `source` the form can show, or `None` when the file is not one of the
/// supported formats or cannot be read as text.
pub fn connection_fields(source: &Path) -> Option<Vec<ConnectionField>> {
    let wordpress = is_wordpress(source)?;
    let contents = String::from_utf8(std::fs::read(source).ok()?).ok()?;
    Some(fields_of(&contents, wordpress))
}

fn fields_of(contents: &str, wordpress: bool) -> Vec<ConnectionField> {
    let lines = contents.split('\n').collect::<Vec<_>>();
    names(wordpress)
        .into_iter()
        .map(|(key, name)| {
            let found = defining_lines(contents, wordpress, name);
            let value = found.first().and_then(|&index| {
                let line = lines[index].trim_end_matches('\r');
                if wordpress {
                    wordpress_define(line.trim_start(), name)
                } else {
                    dotenv_value(line, name)
                }
            });
            let refusal = if found.len() > 1 {
                Some(FieldRefusal::Unsafe)
            } else if wordpress
                && found
                    .first()
                    .is_some_and(|&index| lines[index].starts_with([' ', '\t']))
            {
                // An indented define may sit inside an `if`: rewriting it could change only
                // one branch.
                Some(FieldRefusal::Unsafe)
            } else if value.as_deref().is_some_and(|value| value.contains("${")) {
                Some(FieldRefusal::Variable)
            } else {
                None
            };
            ConnectionField {
                key,
                name,
                value: match key {
                    ConnectionKey::Password => None,
                    _ => value.filter(|_| refusal.is_none()),
                },
                refusal,
            }
        })
        .collect()
}

/// How a value is written in a `.env`, so that both the app and the project's own loader read
/// it back unchanged: bare when simple, in single quotes (literal) otherwise, in double quotes
/// with escapes when it has an apostrophe. `${` would be read as a variable: refused.
fn dotenv_literal(value: &str) -> Result<String, &'static str> {
    if value.contains("${") || value.contains(['\n', '\r']) {
        return Err("valor não suportado");
    }
    let simple = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-.:/@+,%".contains(c));
    if simple && !value.is_empty() {
        Ok(value.to_owned())
    } else if !value.contains('\'') {
        Ok(format!("'{value}'"))
    } else {
        Ok(format!(
            "\"{}\"",
            value.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    }
}

/// A PHP single-quoted string: only `'` and `\` are escaped.
fn php_literal(value: &str) -> Result<String, &'static str> {
    if value.contains(['\n', '\r']) {
        return Err("valor não suportado");
    }
    Ok(format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "\\'")
    ))
}

/// `contents` with each `(key, value)` set. Only the lines that define those keys change
/// (a missing `.env` key is added at the end); every other byte, the line endings included,
/// stays. Refuses a key whose field has a refusal.
pub fn write_connection_fields(
    contents: &str,
    source: &Path,
    changes: &[(ConnectionKey, String)],
) -> Result<String, String> {
    let wordpress = is_wordpress(source).ok_or("formato de arquivo não suportado")?;
    let newline = if contents.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let fields = fields_of(contents, wordpress);
    let mut lines = contents.split('\n').map(str::to_owned).collect::<Vec<_>>();
    for (key, value) in changes {
        let field = fields
            .iter()
            .find(|field| field.key == *key)
            .ok_or("campo desconhecido")?;
        if field.refusal.is_some() {
            return Err(format!("{} não pode ser alterado aqui", field.name));
        }
        let found = defining_lines(contents, wordpress, field.name);
        if wordpress {
            let index = *found
                .first()
                .ok_or_else(|| format!("{} não está definido no arquivo", field.name))?;
            let line = &lines[index];
            let carriage = line.ends_with('\r');
            let body = line.trim_end_matches('\r');
            let rewritten = rewrite_define(body, field.name, &php_literal(value)?)
                .ok_or_else(|| format!("{} não pode ser alterado aqui", field.name))?;
            lines[index] = if carriage {
                format!("{rewritten}\r")
            } else {
                rewritten
            };
        } else {
            let literal = dotenv_literal(value).map_err(str::to_owned)?;
            match found.first() {
                Some(&index) => {
                    let line = &lines[index];
                    let carriage = line.ends_with('\r');
                    let body = line.trim_end_matches('\r');
                    let indent = &body[..body.len() - body.trim_start().len()];
                    let export = if body.trim_start().starts_with("export ") {
                        "export "
                    } else {
                        ""
                    };
                    let rewritten = format!("{indent}{export}{}={literal}", field.name);
                    lines[index] = if carriage {
                        format!("{rewritten}\r")
                    } else {
                        rewritten
                    };
                }
                None => {
                    // Added at the end, after the last line that has text.
                    let last = lines
                        .iter()
                        .rposition(|line| !line.trim().is_empty())
                        .map_or(0, |position| position + 1);
                    let carriage = if newline == "\r\n" { "\r" } else { "" };
                    lines.insert(last, format!("{}={literal}{carriage}", field.name));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

/// The define line with its value replaced, keeping the rest of the line as it was.
fn rewrite_define(line: &str, name: &str, literal: &str) -> Option<String> {
    let start = line.find("define(")?;
    let after_name = {
        let rest = &line[start..];
        let name_at = rest.find(name)?;
        start + name_at + name.len()
    };
    // The value starts after the comma that follows the quoted name.
    let comma = after_name + line[after_name..].find(',')?;
    let value_start = comma + 1 + (line[comma + 1..].len() - line[comma + 1..].trim_start().len());
    let quote = line[value_start..].chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    // Find the closing quote, honoring backslash escapes.
    let mut escaped = false;
    let mut end = None;
    for (offset, c) in line[value_start + 1..].char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            end = Some(value_start + 1 + offset + c.len_utf8());
            break;
        }
    }
    let end = end?;
    Some(format!("{}{literal}{}", &line[..value_start], &line[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database_connection::read_credentials;
    use crate::databases::{DatabaseEngine, DatabaseRequirement};

    fn write_and_read(file: &str, contents: &str, changes: &[(ConnectionKey, &str)]) -> String {
        let folder = std::env::temp_dir().join(format!(
            "lcp-t040-{}-{}",
            std::process::id(),
            contents.len() + changes.len()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(file);
        let changes = changes
            .iter()
            .map(|(key, value)| (*key, (*value).to_owned()))
            .collect::<Vec<_>>();
        let written = write_connection_fields(contents, &path, &changes).unwrap();
        std::fs::write(&path, &written).unwrap();
        written
    }

    fn read_back(path: &Path) -> crate::database_connection::DatabaseCredentials {
        read_credentials(&DatabaseRequirement {
            source: path.to_path_buf(),
            engine: DatabaseEngine::MySql,
            host: Some("127.0.0.1".into()),
            port: Some(3306),
            compose_service: None,
        })
    }

    #[test]
    fn only_the_changed_keys_change_and_the_rest_stays_byte_for_byte() {
        let contents = "APP_NAME=Loja\r\n# banco\r\nDB_DATABASE=loja\r\nDB_USERNAME=app\r\nDB_PASSWORD=antiga\r\nMAIL=x\r\n";
        let written = write_and_read(
            ".env",
            contents,
            &[
                (ConnectionKey::User, "novo"),
                (ConnectionKey::Password, "s3nha"),
            ],
        );
        assert_eq!(
            written,
            "APP_NAME=Loja\r\n# banco\r\nDB_DATABASE=loja\r\nDB_USERNAME=novo\r\nDB_PASSWORD=s3nha\r\nMAIL=x\r\n"
        );
    }

    #[test]
    fn hard_values_are_quoted_so_they_read_back_unchanged() {
        for value in [
            "com espaço",
            "a#b",
            "aspas\"duplas",
            "barra\\invertida",
            "custo$10",
            "d'agua",
            "d'agua \"e\" \\",
        ] {
            let folder = std::env::temp_dir().join(format!(
                "lcp-t040-q-{}-{}",
                std::process::id(),
                value.len()
            ));
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join(".env");
            let written = write_connection_fields(
                "DB_DATABASE=loja\nDB_USERNAME=app\nDB_PASSWORD=x\n",
                &path,
                &[(ConnectionKey::Password, value.to_owned())],
            )
            .unwrap();
            std::fs::write(&path, &written).unwrap();
            assert_eq!(read_back(&path).password_for_setup(), value, "{written}");
            let _ = std::fs::remove_dir_all(&folder);
        }
        // A value the project's loader would read as a variable is refused.
        assert!(
            write_connection_fields(
                "DB_PASSWORD=x\n",
                Path::new(".env"),
                &[(ConnectionKey::Password, "a${B}".to_owned())],
            )
            .is_err()
        );
    }

    #[test]
    fn a_missing_key_is_added_at_the_end() {
        let written = write_connection_fields(
            "DB_DATABASE=loja\nDB_USERNAME=app\n\n",
            Path::new(".env"),
            &[(ConnectionKey::Password, "nova".to_owned())],
        )
        .unwrap();
        assert_eq!(
            written,
            "DB_DATABASE=loja\nDB_USERNAME=app\nDB_PASSWORD=nova\n\n"
        );
    }

    #[test]
    fn variables_duplicates_and_conditional_defines_are_left_alone() {
        let fields = fields_of(
            "DB_DATABASE=${APP_DB}\nDB_USERNAME=app\nDB_USERNAME=outro\nDB_PASSWORD=x\n",
            false,
        );
        assert_eq!(fields[0].refusal, Some(FieldRefusal::Variable));
        assert_eq!(fields[1].refusal, Some(FieldRefusal::Unsafe));
        assert_eq!(fields[2].refusal, None);
        // The password is never handed to the form.
        assert_eq!(fields[2].value, None);
        assert!(!format!("{:?}", fields[2]).contains('x'));

        let wordpress = fields_of(
            "<?php\ndefine( 'DB_NAME', getenv('WP_DB') );\nif ( $local ) {\n    define( 'DB_USER', 'dev' );\n}\ndefine( 'DB_PASSWORD', 'p' );\n",
            true,
        );
        assert_eq!(wordpress[0].refusal, Some(FieldRefusal::Variable));
        assert_eq!(wordpress[1].refusal, Some(FieldRefusal::Unsafe));
        assert_eq!(wordpress[2].refusal, None);
        assert!(
            write_connection_fields(
                "DB_USERNAME=a\nDB_USERNAME=b\n",
                Path::new(".env"),
                &[(ConnectionKey::User, "c".to_owned())],
            )
            .is_err()
        );
    }

    #[test]
    fn wp_config_values_keep_the_line_and_read_back_unchanged() {
        let contents = "<?php\r\ndefine( 'DB_NAME', 'loja' );\r\ndefine('DB_USER', \"app\");\r\ndefine( 'DB_PASSWORD', 'velha' ); // senha\r\n";
        let written = write_and_read(
            "wp-config.php",
            contents,
            &[
                (ConnectionKey::User, "novo"),
                (ConnectionKey::Password, "d'agua \\ $x"),
            ],
        );
        assert_eq!(
            written,
            "<?php\r\ndefine( 'DB_NAME', 'loja' );\r\ndefine('DB_USER', 'novo');\r\ndefine( 'DB_PASSWORD', 'd\\'agua \\\\ $x' ); // senha\r\n"
        );
        let folder = std::env::temp_dir().join(format!(
            "lcp-t040-{}-{}",
            std::process::id(),
            contents.len() + 2
        ));
        let credentials = read_back(&folder.join("wp-config.php"));
        assert_eq!(credentials.user.as_deref(), Some("novo"));
        assert_eq!(credentials.password_for_setup(), "d'agua \\ $x");
    }

    #[test]
    fn only_the_three_supported_files() {
        assert!(connection_fields(Path::new("settings.py")).is_none());
        assert!(connection_fields(Path::new("database.yml")).is_none());
    }
}
