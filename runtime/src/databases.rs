use std::{
    collections::VecDeque,
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseEngine {
    PostgreSql,
    MySql,
    MariaDb,
    SqlServer,
    Sqlite,
}

impl DatabaseEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::PostgreSql => "PostgreSQL",
            Self::MySql => "MySQL",
            Self::MariaDb => "MariaDB",
            Self::SqlServer => "SQL Server",
            Self::Sqlite => "SQLite",
        }
    }

    pub fn default_port(self) -> Option<u16> {
        match self {
            Self::PostgreSql => Some(5432),
            Self::MySql | Self::MariaDb => Some(3306),
            Self::SqlServer => Some(1433),
            Self::Sqlite => None,
        }
    }

    fn from_connection(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "pgsql" | "postgres" | "postgresql" => Some(Self::PostgreSql),
            "mysql" => Some(Self::MySql),
            "mariadb" => Some(Self::MariaDb),
            "sqlsrv" | "mssql" => Some(Self::SqlServer),
            "sqlite" => Some(Self::Sqlite),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRequirement {
    pub source: PathBuf,
    pub engine: DatabaseEngine,
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Checks only loopback TCP endpoints. `None` means there is no safe local probe to run.
/// A successful TCP connection does not validate credentials or database existence.
pub fn probe_local_tcp(requirement: &DatabaseRequirement) -> Option<bool> {
    let (Some(host), Some(port)) = (requirement.host.as_deref(), requirement.port) else {
        return None;
    };
    let addresses = match host {
        "127.0.0.1" => vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)],
        "::1" => vec![SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)],
        "localhost" => vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
        ],
        _ => return None,
    };
    Some(
        addresses
            .iter()
            .any(|address| TcpStream::connect_timeout(address, Duration::from_millis(800)).is_ok()),
    )
}

/// Detects active Laravel-style DB_* settings. Never returns a database name, user or password.
pub fn detect_database_requirements(project_root: &Path) -> Vec<DatabaseRequirement> {
    let mut found = Vec::new();
    let mut queue = VecDeque::from([(project_root.to_path_buf(), 0_usize)]);
    let scan_children = project_root.join(".git").exists();
    while let Some((directory, depth)) = queue.pop_front() {
        for name in [".env", ".env.local"] {
            let path = directory.join(name);
            if let Ok(contents) = fs::read_to_string(&path)
                && let Some(requirement) = parse_dotenv(&path, &contents)
            {
                found.push(requirement);
            }
        }
        if !scan_children || depth >= 8 {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || [
                    "node_modules",
                    "vendor",
                    "target",
                    "dist",
                    "build",
                    ".venv",
                    "venv",
                ]
                .iter()
                .any(|ignored| name.eq_ignore_ascii_case(ignored))
                || entry.path().join(".localcodepilot-ignore").is_file()
            {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    found
}

fn parse_dotenv(path: &Path, contents: &str) -> Option<DatabaseRequirement> {
    let mut engine = None;
    let mut host = None;
    let mut port = None;
    let mut port_was_set = false;
    for line in contents.lines() {
        let Some((key, raw)) = line.trim().split_once('=') else {
            continue;
        };
        let value = raw.trim().trim_matches(['\'', '"']);
        match key {
            "DB_CONNECTION" => engine = DatabaseEngine::from_connection(value),
            "DB_HOST"
                if !value.is_empty() && !value.chars().any(|ch| matches!(ch, '$' | '{' | '}')) =>
            {
                host = Some(value.to_owned());
            }
            "DB_PORT" => {
                port_was_set = true;
                port = value.parse::<u16>().ok().filter(|port| *port > 0);
            }
            _ => {}
        }
    }
    let engine = engine?;
    Some(DatabaseRequirement {
        source: path.to_path_buf(),
        engine,
        host: if engine == DatabaseEngine::Sqlite {
            None
        } else {
            host
        },
        port: if engine == DatabaseEngine::Sqlite {
            None
        } else if port_was_set {
            port
        } else {
            engine.default_port()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_example_files_and_does_not_probe_invalid_or_remote_targets() {
        let root = std::env::temp_dir().join(format!(
            "lcp-db-safety-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".env.example"), "DB_CONNECTION=mysql\n").unwrap();
        assert!(detect_database_requirements(&root).is_empty());

        let path = root.join(".env");
        fs::write(
            &path,
            "DB_CONNECTION=sqlsrv\nDB_HOST=server.example\nDB_PORT=invalid\n",
        )
        .unwrap();
        let found = detect_database_requirements(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].engine, DatabaseEngine::SqlServer);
        assert_eq!(found[0].port, None);
        assert_eq!(probe_local_tcp(&found[0]), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_multiple_database_engines_without_exposing_credentials() {
        let root = std::env::temp_dir().join(format!("lcp-db-detect-{}", std::process::id()));
        for (name, connection, port) in [
            ("api", "pgsql", "5432"),
            ("web", "mysql", "3307"),
            ("legacy", "sqlsrv", "1433"),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(".env"), format!(
                "DB_CONNECTION={connection}\nDB_HOST=127.0.0.1\nDB_PORT={port}\nDB_PASSWORD=private-test-value\n"
            )).unwrap();
        }
        fs::create_dir_all(root.join(".git")).unwrap();
        let found = detect_database_requirements(&root);
        assert_eq!(found.len(), 3);
        assert!(
            found
                .iter()
                .any(|db| db.engine == DatabaseEngine::PostgreSql)
        );
        assert!(
            found
                .iter()
                .any(|db| db.engine == DatabaseEngine::MySql && db.port == Some(3307))
        );
        assert!(
            found
                .iter()
                .any(|db| db.engine == DatabaseEngine::SqlServer)
        );
        assert!(!format!("{found:?}").contains("private-test-value"));
        fs::remove_dir_all(root).unwrap();
    }
}
