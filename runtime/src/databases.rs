use rusqlite::{Connection, OpenFlags};
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
    /// Compose service of the same project that `host` names, such as `db`. The database
    /// then lives on the Compose network and cannot be probed from the host.
    pub compose_service: Option<String>,
}

/// The destination state for a detected database. This does not imply that
/// authentication or the requested schema has been verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseRoute {
    LocalServerReachable(DatabaseEngine),
    LocalServerUnavailable(DatabaseEngine),
    RemoteServer(DatabaseEngine),
    ComposeService(DatabaseEngine),
    FileDatabase,
    NeedsConfiguration(DatabaseEngine),
}

pub fn route_database(requirement: &DatabaseRequirement) -> DatabaseRoute {
    if requirement.engine == DatabaseEngine::Sqlite {
        return DatabaseRoute::FileDatabase;
    }
    let Some(host) = requirement.host.as_deref() else {
        return DatabaseRoute::NeedsConfiguration(requirement.engine);
    };
    if requirement.compose_service.is_some() {
        return DatabaseRoute::ComposeService(requirement.engine);
    }
    if !matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return DatabaseRoute::RemoteServer(requirement.engine);
    }
    match probe_local_tcp(requirement) {
        Some(true) => DatabaseRoute::LocalServerReachable(requirement.engine),
        Some(false) => DatabaseRoute::LocalServerUnavailable(requirement.engine),
        None => DatabaseRoute::NeedsConfiguration(requirement.engine),
    }
}

/// Resolves only a project-owned SQLite file. Paths outside the project and
/// in-memory databases are intentionally left to the application itself.
fn sqlite_path(requirement: &DatabaseRequirement, project_root: &Path) -> Result<PathBuf, String> {
    if requirement.engine != DatabaseEngine::Sqlite {
        return Err("O banco detectado não é SQLite".into());
    }
    let root = fs::canonicalize(project_root)
        .map_err(|error| format!("Não foi possível localizar o projeto: {error}"))?;
    if !fs::canonicalize(&requirement.source).is_ok_and(|source| source.starts_with(&root)) {
        return Err("A configuração SQLite está fora do projeto".into());
    }
    let contents = fs::read_to_string(&requirement.source)
        .map_err(|error| format!("Não foi possível ler a configuração do banco: {error}"))?;
    let name = contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            (key == "DB_DATABASE").then(|| value.trim().trim_matches(['\'', '"']))
        })
        .next_back()
        .ok_or("DB_DATABASE não foi definido para SQLite")?;
    if name.is_empty() || name == ":memory:" || name.contains(['$', '{', '}']) {
        return Err("DB_DATABASE não aponta para um arquivo SQLite local".into());
    }
    let configured = Path::new(name);
    let target = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    };
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("Atalhos simbólicos para arquivos SQLite não são preparados".into());
    }
    let parent = target.parent().ok_or("Caminho SQLite inválido")?;
    let mut existing = parent;
    while !existing.exists() {
        existing = existing.parent().ok_or("Caminho SQLite inválido")?;
    }
    let resolved = fs::canonicalize(existing)
        .map_err(|error| format!("Não foi possível validar a pasta SQLite: {error}"))?;
    if !resolved.starts_with(&root) {
        return Err("O arquivo SQLite está fora do projeto; preparação automática ignorada".into());
    }
    Ok(target)
}

pub fn prepare_sqlite_database(
    requirement: &DatabaseRequirement,
    project_root: &Path,
) -> Result<(), String> {
    let target = sqlite_path(requirement, project_root)?;
    let parent = target.parent().ok_or("Caminho SQLite inválido")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Não foi possível criar a pasta SQLite: {error}"))?;
    let root = fs::canonicalize(project_root)
        .map_err(|error| format!("Não foi possível localizar o projeto: {error}"))?;
    if !fs::canonicalize(parent).is_ok_and(|resolved| resolved.starts_with(root)) {
        return Err("A pasta SQLite está fora do projeto".into());
    }
    let connection = Connection::open_with_flags(
        target,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|error| format!("Não foi possível abrir SQLite: {error}"))?;
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| format!("Não foi possível validar SQLite: {error}"))?;
    if result != "ok" {
        return Err("O arquivo SQLite não passou na verificação de integridade".into());
    }
    Ok(())
}

pub fn probe_sqlite_database(
    requirement: &DatabaseRequirement,
    project_root: &Path,
) -> Option<bool> {
    let target = sqlite_path(requirement, project_root).ok()?;
    if !target.is_file() {
        return Some(false);
    }
    let Ok(connection) = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Some(false);
    };
    let result: Result<String, _> =
        connection.query_row("PRAGMA quick_check", [], |row| row.get(0));
    Some(result.is_ok_and(|value| value == "ok"))
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
        let wordpress_config = directory.join("wp-config.php");
        if let Ok(contents) = fs::read_to_string(&wordpress_config)
            && let Some(requirement) = parse_wordpress_config(&wordpress_config, &contents)
        {
            found.push(requirement);
        }
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
    mark_compose_services(project_root, &mut found);
    found
}

/// Links hosts such as `db` to a service declared in one of the project's Compose files.
/// The Compose scan only runs when some host is not a loopback address.
fn mark_compose_services(project_root: &Path, requirements: &mut [DatabaseRequirement]) {
    let named_host = |requirement: &DatabaseRequirement| {
        requirement
            .host
            .as_deref()
            .filter(|host| !matches!(*host, "localhost" | "127.0.0.1" | "::1"))
            .map(str::to_owned)
    };
    if !requirements
        .iter()
        .any(|requirement| named_host(requirement).is_some())
    {
        return;
    }
    let services = crate::containers::detect_container_projects(project_root)
        .into_iter()
        .flat_map(|project| project.services)
        .map(|service| service.name)
        .collect::<Vec<_>>();
    for requirement in requirements {
        if let Some(host) = named_host(requirement)
            && services.contains(&host)
        {
            requirement.compose_service = Some(host);
        }
    }
}

/// Reads only literal WordPress DB_HOST values. PHP expressions are left unresolved.
/// Database names and credentials are never retained or returned.
fn parse_wordpress_config(path: &Path, contents: &str) -> Option<DatabaseRequirement> {
    let host = contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('#') || !line.starts_with("define(") {
            return None;
        }
        let body = line.strip_prefix("define(")?.trim_start();
        let quote = body.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        let body = body
            .strip_prefix(quote)?
            .strip_prefix("DB_HOST")?
            .strip_prefix(quote)?;
        let body = body.trim_start().strip_prefix(',')?.trim_start();
        let value_quote = body.chars().next()?;
        if !matches!(value_quote, '\'' | '"') {
            return None;
        }
        let value = body.strip_prefix(value_quote)?.split_once(value_quote)?.0;
        (!value.is_empty() && !value.contains(['$', '{', '}'])).then(|| value.to_owned())
    })?;
    let (host, port) = match host.rsplit_once(':') {
        Some((name, port)) if port.parse::<u16>().is_ok_and(|port| port > 0) => {
            (name.to_owned(), port.parse::<u16>().ok())
        }
        _ => (host, DatabaseEngine::MySql.default_port()),
    };
    Some(DatabaseRequirement {
        source: path.to_path_buf(),
        engine: DatabaseEngine::MySql,
        host: Some(host),
        port,
        compose_service: None,
    })
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
        compose_service: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordpress_database_detection_keeps_credentials_private() {
        let path = Path::new("/tmp/wordpress/wp-config.php");
        let contents = "define( 'DB_NAME', 'private_db' );\n\
            define( 'DB_USER', 'private_user' );\n\
            define( 'DB_PASSWORD', 'private_password' );\n\
            define( 'DB_HOST', '127.0.0.1:3307' );";
        let requirement = parse_wordpress_config(path, contents).unwrap();
        assert_eq!(requirement.engine, DatabaseEngine::MySql);
        assert_eq!(requirement.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(requirement.port, Some(3307));
        assert!(!format!("{requirement:?}").contains("private"));
        assert!(parse_wordpress_config(path, "define( 'DB_HOST', getenv('DB_HOST') );").is_none());
    }

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

    #[test]
    fn compose_service_hosts_route_to_containers_instead_of_remote_servers() {
        let root = std::env::temp_dir().join(format!(
            "lcp-db-compose-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("docker")).unwrap();
        fs::write(
            root.join("wp-config.php"),
            "<?php\ndefine( 'DB_HOST', 'db:3306' );",
        )
        .unwrap();
        fs::write(
            root.join(".env"),
            "DB_CONNECTION=pgsql\nDB_HOST=analytics.example\n",
        )
        .unwrap();
        fs::write(
            root.join("docker").join("compose.yaml"),
            "services:\n  db:\n    image: mariadb:11\n  web:\n    image: wordpress:6\n",
        )
        .unwrap();

        let found = detect_database_requirements(&root);
        let wordpress = found
            .iter()
            .find(|db| db.engine == DatabaseEngine::MySql)
            .unwrap();
        assert_eq!(wordpress.compose_service.as_deref(), Some("db"));
        assert_eq!(
            route_database(wordpress),
            DatabaseRoute::ComposeService(DatabaseEngine::MySql)
        );
        let remote = found
            .iter()
            .find(|db| db.engine == DatabaseEngine::PostgreSql)
            .unwrap();
        assert_eq!(remote.compose_service, None);
        assert_eq!(
            route_database(remote),
            DatabaseRoute::RemoteServer(DatabaseEngine::PostgreSql)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn routes_each_engine_without_treating_remote_or_file_as_local_server() {
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::MySql,
            DatabaseEngine::MariaDb,
            DatabaseEngine::SqlServer,
        ] {
            let mut requirement = DatabaseRequirement {
                source: PathBuf::from(".env"),
                engine,
                host: Some("127.0.0.1".into()),
                port: None,
                compose_service: None,
            };
            assert_eq!(
                route_database(&requirement),
                DatabaseRoute::NeedsConfiguration(engine)
            );
            requirement.host = Some("db.example".into());
            assert_eq!(
                route_database(&requirement),
                DatabaseRoute::RemoteServer(engine)
            );
            requirement.host = None;
            assert_eq!(
                route_database(&requirement),
                DatabaseRoute::NeedsConfiguration(engine)
            );
        }
        let sqlite = DatabaseRequirement {
            source: PathBuf::from(".env"),
            engine: DatabaseEngine::Sqlite,
            host: None,
            port: None,
            compose_service: None,
        };
        assert_eq!(route_database(&sqlite), DatabaseRoute::FileDatabase);
    }

    #[test]
    fn prepares_project_owned_sqlite_and_rejects_paths_outside_project() {
        let root = std::env::temp_dir().join(format!(
            "lcp-sqlite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join(".env");
        fs::write(
            &source,
            "DB_CONNECTION=sqlite\nDB_DATABASE=database/app.sqlite\n",
        )
        .unwrap();
        let database = detect_database_requirements(&root).remove(0);
        assert_eq!(probe_sqlite_database(&database, &root), Some(false));
        prepare_sqlite_database(&database, &root).unwrap();
        prepare_sqlite_database(&database, &root).unwrap();
        assert!(root.join("database/app.sqlite").is_file());
        assert_eq!(probe_sqlite_database(&database, &root), Some(true));

        fs::write(root.join("database/app.sqlite"), "not a sqlite database").unwrap();
        assert_eq!(probe_sqlite_database(&database, &root), Some(false));
        assert!(prepare_sqlite_database(&database, &root).is_err());

        fs::write(
            &source,
            "DB_CONNECTION=sqlite\nDB_DATABASE=../outside.sqlite\n",
        )
        .unwrap();
        assert!(prepare_sqlite_database(&database, &root).is_err());
        assert!(!root.parent().unwrap().join("outside.sqlite").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
