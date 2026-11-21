//! Authenticated connection test of a project's database, with the project's own access
//! data. A TCP port that answers, or a running container, proves nothing about the login or
//! the database: only this test says "Banco de dados conectado".
//!
//! Agreed in `.internal/conversas/2026-11-20-t019-03-conexao-e-cartao.md`:
//! - only loopback hosts are tested (`localhost` as 127.0.0.1, `127.0.0.1`, `::1`); remote
//!   hosts and Compose service names are not;
//! - the access data is read on demand and never kept: [`DatabaseCredentials`] prints its
//!   password as a mask, and no result carries it;
//! - MySQL/MariaDB use TLS accepting the server's own certificate, only toward loopback (MySQL
//!   9's `caching_sha2_password` needs a secure channel on a user's first login). This does
//!   not verify who listens on the port; it only keeps the password off a plain channel.

use crate::databases::{DatabaseEngine, DatabaseRequirement};
use std::{
    fmt, fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::{Duration, SystemTime},
};

/// The access data a project declares, read from the same file as its database settings.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DatabaseCredentials {
    pub database: Option<String>,
    pub user: Option<String>,
    password: Option<String>,
    /// Settings present but written with a variable (`${...}`), which the app cannot resolve.
    pub unresolved: Vec<&'static str>,
    /// Settings the file does not have at all.
    pub missing: Vec<&'static str>,
}

impl fmt::Debug for DatabaseCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseCredentials")
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "••••"))
            .field("unresolved", &self.unresolved)
            .field("missing", &self.missing)
            .finish()
    }
}

impl DatabaseCredentials {
    /// Database and user are needed to test; an empty password is valid.
    fn complete(&self) -> bool {
        self.database.is_some() && self.user.is_some() && !self.unresolved.contains(&"senha")
    }

    /// The project's password, to create its user on a server the app manages (never shown
    /// nor stored by the app). Empty when the project has none.
    pub fn password_for_setup(&self) -> String {
        self.password.clone().unwrap_or_default()
    }

    /// The setting names that keep the test from running, for "Detalhes técnicos".
    pub fn insufficient(&self) -> Vec<String> {
        let missing = self.missing.iter().map(|name| format!("{name} ausente"));
        let unresolved = self
            .unresolved
            .iter()
            .map(|name| format!("{name} usa uma variável"));
        missing
            .chain(unresolved)
            .filter(|text| !text.starts_with("senha ausente"))
            .collect()
    }
}

fn unresolved_value(value: &str) -> bool {
    value.contains(['$', '{', '}'])
}

/// Reads database, user and password from the requirement's own file (`.env`,
/// `.env.local` or `wp-config.php`).
pub fn read_credentials(requirement: &DatabaseRequirement) -> DatabaseCredentials {
    let contents = fs::read_to_string(&requirement.source).unwrap_or_default();
    let is_wordpress = requirement
        .source
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("wp-config.php"));
    let lookup = |key: &str| -> Option<String> {
        if is_wordpress {
            wordpress_define(&contents, key)
        } else {
            dotenv_value(&contents, key)
        }
    };
    let names: [(&str, &'static str); 3] = if is_wordpress {
        [
            ("DB_NAME", "banco"),
            ("DB_USER", "usuário"),
            ("DB_PASSWORD", "senha"),
        ]
    } else {
        [
            ("DB_DATABASE", "banco"),
            ("DB_USERNAME", "usuário"),
            ("DB_PASSWORD", "senha"),
        ]
    };
    let mut credentials = DatabaseCredentials::default();
    for (key, label) in names {
        let value = lookup(key);
        let usable = match &value {
            None => {
                credentials.missing.push(label);
                None
            }
            Some(text) if unresolved_value(text) => {
                credentials.unresolved.push(label);
                None
            }
            Some(text) if text.is_empty() && label != "senha" => {
                credentials.missing.push(label);
                None
            }
            Some(text) => Some(text.clone()),
        };
        match label {
            "banco" => credentials.database = usable,
            "usuário" => credentials.user = usable,
            _ => credentials.password = usable,
        }
    }
    credentials
}

fn dotenv_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, raw) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let raw = raw.trim();
        let value = if let Some(inner) = raw
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        {
            inner
        } else if let Some(inner) = raw
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
        {
            inner
        } else {
            // Unquoted: a ` #` starts a comment.
            raw.split(" #").next().unwrap_or(raw).trim()
        };
        Some(value.to_owned())
    })
}

fn wordpress_define(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('#') {
            return None;
        }
        let body = line.strip_prefix("define(")?.trim_start();
        let quote = body.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let body = body
            .strip_prefix(quote)?
            .strip_prefix(key)?
            .strip_prefix(quote)?
            .trim_start()
            .strip_prefix(',')?
            .trim_start();
        let value_quote = body.chars().next()?;
        if !matches!(value_quote, '\'' | '"') {
            // A constant or an expression (getenv(...)): not something the app can read.
            return Some("${expressão}".to_owned());
        }
        Some(
            body.strip_prefix(value_quote)?
                .split_once(value_quote)?
                .0
                .to_owned(),
        )
    })
}

/// What one connection test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionOutcome {
    /// Logged in and selected the configured database.
    Connected,
    /// The server refused the user, the password, or the user's access to the database.
    AccessDenied,
    /// Logged in, but the configured database does not exist.
    DatabaseMissing,
    /// Nothing answers on the address.
    NotResponding,
    TimedOut,
    /// The project does not give access data the app can read.
    InsufficientData,
    /// Another failure, with the driver's message (never the password).
    Failed(String),
}

/// One connection test, for the card and "Detalhes técnicos".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionTest {
    pub outcome: ConnectionOutcome,
    /// The address actually used (`127.0.0.1:3306`), which may differ from the configured host.
    pub target: String,
    pub user: Option<String>,
    pub database: Option<String>,
    /// Missing or variable settings, when the test could not run.
    pub insufficient: Vec<String>,
    pub at: SystemTime,
}

/// The loopback address the app tests for a configured host, or `None` for any other host.
pub fn loopback_address(host: &str) -> Option<IpAddr> {
    match host.trim().to_ascii_lowercase().as_str() {
        "localhost" | "127.0.0.1" => Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        "::1" | "[::1]" => Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        _ => None,
    }
}

/// Tests the login of a project's database. Returns `None` when the requirement is not tested
/// by the app: SQLite, SQL Server, Compose service names and hosts outside this computer.
pub fn test_connection(
    requirement: &DatabaseRequirement,
    credentials: &DatabaseCredentials,
    timeout: Duration,
) -> Option<ConnectionTest> {
    if requirement.compose_service.is_some()
        || !matches!(
            requirement.engine,
            DatabaseEngine::PostgreSql
                | DatabaseEngine::MySql
                | DatabaseEngine::MariaDb
                | DatabaseEngine::SqlServer
        )
    {
        return None;
    }
    let address = loopback_address(requirement.host.as_deref()?)?;
    let port = requirement.port?;
    let mut test = ConnectionTest {
        outcome: ConnectionOutcome::InsufficientData,
        target: match address {
            IpAddr::V6(_) => format!("[::1]:{port}"),
            IpAddr::V4(_) => format!("127.0.0.1:{port}"),
        },
        user: credentials.user.clone(),
        database: credentials.database.clone(),
        insufficient: credentials.insufficient(),
        at: SystemTime::now(),
    };
    if !credentials.complete() {
        return Some(test);
    }
    let (Some(user), Some(database)) = (&credentials.user, &credentials.database) else {
        return Some(test);
    };
    let password = credentials.password.as_deref().unwrap_or_default();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            test.outcome = ConnectionOutcome::Failed(error.to_string());
            return Some(test);
        }
    };
    let attempt = async {
        match requirement.engine {
            DatabaseEngine::PostgreSql => {
                postgres_login(address, port, user, password, database).await
            }
            // The certificate SQL Server generates for itself is accepted only on loopback.
            DatabaseEngine::SqlServer => {
                mssql_login(&address.to_string(), port, user, password, database, true).await
            }
            _ => mysql_login(address, port, user, password, database).await,
        }
    };
    test.outcome = runtime
        .block_on(async { tokio::time::timeout(timeout, attempt).await })
        .unwrap_or(ConnectionOutcome::TimedOut);
    if let ConnectionOutcome::Failed(message) = &mut test.outcome
        && !password.is_empty()
    {
        *message = message.replace(password, "••••");
    }
    Some(test)
}

/// Tests a database outside this computer, only when the person asks ("Testar conexão" in
/// "Detalhes técnicos"). No certificate exception: PostgreSQL without TLS, MySQL/MariaDB
/// without TLS (a server that requires TLS reports it as the failure).
pub fn test_remote_connection(
    requirement: &DatabaseRequirement,
    credentials: &DatabaseCredentials,
    timeout: Duration,
) -> Option<ConnectionTest> {
    let host = requirement.host.as_deref()?.trim().to_owned();
    if requirement.compose_service.is_some()
        || loopback_address(&host).is_some()
        || host.is_empty()
        || !matches!(
            requirement.engine,
            DatabaseEngine::PostgreSql
                | DatabaseEngine::MySql
                | DatabaseEngine::MariaDb
                | DatabaseEngine::SqlServer
        )
    {
        return None;
    }
    let port = requirement.port?;
    let mut test = ConnectionTest {
        outcome: ConnectionOutcome::InsufficientData,
        target: format!("{host}:{port}"),
        user: credentials.user.clone(),
        database: credentials.database.clone(),
        insufficient: credentials.insufficient(),
        at: SystemTime::now(),
    };
    let (Some(user), Some(database)) = (&credentials.user, &credentials.database) else {
        return Some(test);
    };
    if !credentials.complete() {
        return Some(test);
    }
    let password = credentials.password.as_deref().unwrap_or_default();
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        test.outcome = ConnectionOutcome::Failed("não foi possível preparar o teste".into());
        return Some(test);
    };
    let engine = requirement.engine;
    let attempt = async {
        if engine == DatabaseEngine::SqlServer {
            // A remote SQL Server must present a certificate this computer trusts.
            return mssql_login(&host, port, user, password, database, false).await;
        }
        if engine == DatabaseEngine::PostgreSql {
            let mut config = tokio_postgres::Config::new();
            config
                .host(&host)
                .port(port)
                .user(user)
                .password(password)
                .dbname(database);
            match config.connect(tokio_postgres::NoTls).await {
                Ok((client, connection)) => {
                    let driver = tokio::spawn(connection);
                    drop(client);
                    let _ = driver.await;
                    ConnectionOutcome::Connected
                }
                Err(error) => match error.code() {
                    Some(code) => classify_postgres(code.code()),
                    None => classify_error_chain(&error),
                },
            }
        } else {
            let options = mysql_async::OptsBuilder::default()
                .ip_or_hostname(host.clone())
                .tcp_port(port)
                .user(Some(user))
                .pass(Some(password))
                .db_name(Some(database));
            crate::tls_ready();
            match mysql_async::Conn::new(options).await {
                Ok(connection) => {
                    let _ = connection.disconnect().await;
                    ConnectionOutcome::Connected
                }
                Err(mysql_async::Error::Server(error)) => {
                    classify_mysql(error.code, &error.message)
                }
                Err(mysql_async::Error::Io(error)) => classify_error_chain(&error),
                Err(error) => ConnectionOutcome::Failed(error.to_string()),
            }
        }
    };
    test.outcome = runtime
        .block_on(async { tokio::time::timeout(timeout, attempt).await })
        .unwrap_or(ConnectionOutcome::TimedOut);
    if let ConnectionOutcome::Failed(message) = &mut test.outcome
        && !password.is_empty()
    {
        *message = message.replace(password, "••••");
    }
    Some(test)
}
/// SQL Server: logs in with SQL authentication and selects the configured database. With
/// `trust_own_certificate` (loopback only), the server's self-generated certificate is accepted.
async fn mssql_login(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
    trust_own_certificate: bool,
) -> ConnectionOutcome {
    use tokio_util::compat::TokioAsyncWriteCompatExt;
    let mut config = tiberius::Config::new();
    config.host(host);
    config.port(port);
    config.authentication(tiberius::AuthMethod::sql_server(user, password));
    config.database(database);
    config.encryption(tiberius::EncryptionLevel::Required);
    if trust_own_certificate {
        config.trust_cert();
    }
    let tcp = match tokio::net::TcpStream::connect(config.get_addr()).await {
        Ok(tcp) => tcp,
        Err(error) => return classify_error_chain(&error),
    };
    let _ = tcp.set_nodelay(true);
    match tiberius::Client::connect(config, tcp.compat_write()).await {
        Ok(client) => {
            drop(client);
            ConnectionOutcome::Connected
        }
        Err(tiberius::error::Error::Server(token)) => classify_mssql(token.code(), token.message()),
        Err(error) => classify_io(&error.to_string()),
    }
}

/// SQL Server error numbers: 18456 login failed, 4060 the database cannot be opened.
fn classify_mssql(code: u32, message: &str) -> ConnectionOutcome {
    match code {
        18456 => ConnectionOutcome::AccessDenied,
        4060 => ConnectionOutcome::DatabaseMissing,
        _ => ConnectionOutcome::Failed(format!("{code}: {message}")),
    }
}

async fn postgres_login(
    address: IpAddr,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
) -> ConnectionOutcome {
    let mut config = tokio_postgres::Config::new();
    config
        .hostaddr(address)
        .port(port)
        .user(user)
        .password(password)
        .dbname(database);
    match config.connect(tokio_postgres::NoTls).await {
        Ok((client, connection)) => {
            let driver = tokio::spawn(connection);
            drop(client);
            let _ = driver.await;
            ConnectionOutcome::Connected
        }
        Err(error) => match error.code() {
            Some(code) => classify_postgres(code.code()),
            None => classify_error_chain(&error),
        },
    }
}

/// PostgreSQL SQLSTATE codes: 28P01/28000 refused, 3D000 database missing.
fn classify_postgres(code: &str) -> ConnectionOutcome {
    match code {
        "28P01" | "28000" | "42501" => ConnectionOutcome::AccessDenied,
        "3D000" => ConnectionOutcome::DatabaseMissing,
        other => ConnectionOutcome::Failed(format!("código {other}")),
    }
}

/// MySQL/MariaDB error numbers: 1045 refused, 1044 no access to the database, 1049 missing.
fn classify_mysql(code: u16, message: &str) -> ConnectionOutcome {
    match code {
        1045 | 1044 => ConnectionOutcome::AccessDenied,
        1049 => ConnectionOutcome::DatabaseMissing,
        _ => ConnectionOutcome::Failed(format!("{code}: {message}")),
    }
}

/// Walks the error and its sources: a refused connection anywhere in the chain means nothing
/// listens on the address.
fn classify_error_chain(error: &(dyn std::error::Error + 'static)) -> ConnectionOutcome {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => return ConnectionOutcome::NotResponding,
                std::io::ErrorKind::TimedOut => return ConnectionOutcome::TimedOut,
                _ => {}
            }
        }
        current = error.source();
    }
    classify_io(&error.to_string())
}

fn classify_io(message: &str) -> ConnectionOutcome {
    let lower = message.to_lowercase();
    if lower.contains("refused") || lower.contains("10061") || lower.contains("recusou") {
        ConnectionOutcome::NotResponding
    } else if lower.contains("11001")
        || lower.contains("not known")
        || lower.contains("no such host")
        || lower.contains("não é conhecido")
    {
        ConnectionOutcome::Failed("o nome do servidor não foi encontrado".into())
    } else {
        ConnectionOutcome::Failed(message.to_owned())
    }
}

/// MySQL options for a loopback address only: the certificate exception never reaches
/// another host, because there is no way to build these options for one.
fn mysql_options(
    address: IpAddr,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
    tls: bool,
) -> Option<mysql_async::Opts> {
    if !address.is_loopback() {
        return None;
    }
    let mut options = mysql_async::OptsBuilder::default()
        .ip_or_hostname(address.to_string())
        .tcp_port(port)
        .user(Some(user))
        .pass(Some(password))
        .db_name(Some(database));
    if tls {
        options = options
            .ssl_opts(mysql_async::SslOpts::default().with_danger_accept_invalid_certs(true));
    }
    Some(options.into())
}

async fn mysql_login(
    address: IpAddr,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
) -> ConnectionOutcome {
    let attempt = |tls: bool| async move {
        let Some(options) = mysql_options(address, port, user, password, database, tls) else {
            return Err(ConnectionOutcome::Failed(
                "destino fora deste computador".into(),
            ));
        };
        crate::tls_ready();
        match mysql_async::Conn::new(options).await {
            Ok(connection) => {
                let _ = connection.disconnect().await;
                Ok(ConnectionOutcome::Connected)
            }
            Err(mysql_async::Error::Server(error)) => {
                Ok(classify_mysql(error.code, &error.message))
            }
            Err(mysql_async::Error::Io(error)) => Err(classify_error_chain(&error)),
            Err(mysql_async::Error::Driver(error)) => {
                Err(ConnectionOutcome::Failed(error.to_string()))
            }
            Err(error) => Err(ConnectionOutcome::Failed(error.to_string())),
        }
    };
    match attempt(true).await {
        Ok(outcome) => outcome,
        // A server without TLS (some local installs): try once more without it.
        Err(ConnectionOutcome::Failed(message)) if message.to_ascii_lowercase().contains("ssl") => {
            attempt(false).await.unwrap_or_else(|outcome| outcome)
        }
        Err(outcome) => outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// One folder per call: tests run in parallel and must not share files.
    fn temp_file(name: &str, contents: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let call = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("lcp-db-credentials-{}-{call}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    fn requirement(source: PathBuf, engine: DatabaseEngine, host: &str) -> DatabaseRequirement {
        DatabaseRequirement {
            source,
            engine,
            host: Some(host.into()),
            port: engine.default_port(),
            compose_service: None,
        }
    }

    #[test]
    fn reads_the_access_data_of_a_dotenv_and_never_prints_the_password() {
        let path = temp_file(
            ".env",
            "DB_CONNECTION=mysql\nDB_DATABASE=loja\nDB_USERNAME=\"app\"\nDB_PASSWORD='MARCA-SENHA-1' # comentário\n",
        );
        let credentials = read_credentials(&requirement(
            path.clone(),
            DatabaseEngine::MySql,
            "127.0.0.1",
        ));
        assert_eq!(
            credentials.database.as_deref(),
            Some("loja"),
            "{credentials:?} {:?}",
            fs::read_to_string(&path)
        );
        assert_eq!(credentials.user.as_deref(), Some("app"));
        assert!(credentials.complete());
        assert!(credentials.insufficient().is_empty());
        let printed = format!("{credentials:?}");
        assert!(!printed.contains("MARCA-SENHA-1"), "{printed}");
        assert!(printed.contains("••••"));
    }

    #[test]
    fn variables_and_missing_settings_keep_the_test_from_running() {
        let path = temp_file(
            ".env",
            "DB_CONNECTION=pgsql\nDB_DATABASE=${APP_DB}\nDB_PASSWORD=x\n",
        );
        let credentials =
            read_credentials(&requirement(path, DatabaseEngine::PostgreSql, "localhost"));
        assert!(!credentials.complete());
        assert_eq!(
            credentials.insufficient(),
            ["usuário ausente", "banco usa uma variável"]
        );
        // An empty password is a valid one; an empty user is not.
        let path = temp_file(".env", "DB_DATABASE=app\nDB_USERNAME=\nDB_PASSWORD=\n");
        let credentials = read_credentials(&requirement(path, DatabaseEngine::MySql, "localhost"));
        assert_eq!(credentials.insufficient(), ["usuário ausente"]);
        let test = test_connection(
            &requirement(PathBuf::new(), DatabaseEngine::MySql, "localhost"),
            &credentials,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(test.outcome, ConnectionOutcome::InsufficientData);
        assert_eq!(test.insufficient, ["usuário ausente"]);
    }

    #[test]
    fn reads_wordpress_constants_and_treats_expressions_as_variables() {
        let path = temp_file(
            "wp-config.php",
            "<?php\ndefine( 'DB_NAME', 'blog' );\ndefine('DB_USER', getenv('WP_USER'));\ndefine( \"DB_PASSWORD\", \"MARCA-SENHA-2\" );\ndefine('DB_HOST', 'localhost');\n",
        );
        let credentials = read_credentials(&requirement(path, DatabaseEngine::MySql, "localhost"));
        assert_eq!(credentials.database.as_deref(), Some("blog"));
        assert_eq!(credentials.user, None);
        assert_eq!(credentials.insufficient(), ["usuário usa uma variável"]);
        assert!(!format!("{credentials:?}").contains("MARCA-SENHA-2"));
    }

    #[test]
    fn only_loopback_hosts_are_tested() {
        let credentials = DatabaseCredentials {
            database: Some("app".into()),
            user: Some("app".into()),
            ..Default::default()
        };
        for host in [
            "db",
            "192.168.0.10",
            "0.0.0.0",
            "host.docker.internal",
            "exemplo.com",
        ] {
            assert_eq!(
                test_connection(
                    &requirement(PathBuf::new(), DatabaseEngine::PostgreSql, host),
                    &credentials,
                    Duration::from_secs(1)
                ),
                None,
                "{host}"
            );
        }
        let mut compose = requirement(PathBuf::new(), DatabaseEngine::MySql, "127.0.0.1");
        compose.compose_service = Some("db".into());
        assert_eq!(
            test_connection(&compose, &credentials, Duration::from_secs(1)),
            None
        );
        // SQLite is a file: nothing to log into.
        assert_eq!(
            test_connection(
                &requirement(PathBuf::new(), DatabaseEngine::Sqlite, "localhost"),
                &credentials,
                Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn the_remote_test_is_only_for_hosts_outside_this_computer() {
        let credentials = DatabaseCredentials {
            database: Some("app".into()),
            user: Some("app".into()),
            ..Default::default()
        };
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(
                test_remote_connection(
                    &requirement(PathBuf::new(), DatabaseEngine::MySql, host),
                    &credentials,
                    Duration::from_secs(1)
                )
                .is_none(),
                "{host}"
            );
        }
        let mut compose = requirement(PathBuf::new(), DatabaseEngine::MySql, "db");
        compose.compose_service = Some("db".into());
        assert!(test_remote_connection(&compose, &credentials, Duration::from_secs(1)).is_none());
        // A name that does not resolve fails fast and says so.
        let test = test_remote_connection(
            &requirement(
                PathBuf::new(),
                DatabaseEngine::PostgreSql,
                "nao-existe.invalid",
            ),
            &credentials,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(test.target, "nao-existe.invalid:5432");
        assert_ne!(test.outcome, ConnectionOutcome::Connected);
    }
    #[test]
    fn the_certificate_exception_cannot_be_built_for_another_host() {
        let remote = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10));
        assert!(mysql_options(remote, 3306, "u", "p", "d", true).is_none());
        assert!(mysql_options(remote, 3306, "u", "p", "d", false).is_none());
        assert!(
            mysql_options(IpAddr::V4(Ipv4Addr::LOCALHOST), 3306, "u", "p", "d", true).is_some()
        );
        assert!(
            mysql_options(IpAddr::V6(Ipv6Addr::LOCALHOST), 3306, "u", "p", "d", true).is_some()
        );
    }

    #[test]
    fn server_codes_tell_a_refused_login_from_a_missing_database() {
        assert_eq!(classify_postgres("28P01"), ConnectionOutcome::AccessDenied);
        assert_eq!(classify_postgres("28000"), ConnectionOutcome::AccessDenied);
        assert_eq!(
            classify_postgres("3D000"),
            ConnectionOutcome::DatabaseMissing
        );
        assert_eq!(classify_mysql(1045, ""), ConnectionOutcome::AccessDenied);
        assert_eq!(classify_mysql(1044, ""), ConnectionOutcome::AccessDenied);
        assert_eq!(classify_mysql(1049, ""), ConnectionOutcome::DatabaseMissing);
        assert_eq!(classify_mssql(18456, ""), ConnectionOutcome::AccessDenied);
        assert_eq!(classify_mssql(4060, ""), ConnectionOutcome::DatabaseMissing);
        assert_eq!(
            classify_io("connection refused (os error 10061)"),
            ConnectionOutcome::NotResponding
        );
        assert_eq!(
            classify_io("Input/output error: Este host não é conhecido. (os error 11001)"),
            ConnectionOutcome::Failed("o nome do servidor não foi encontrado".into())
        );
    }

    #[test]
    fn nothing_listening_is_not_responding() {
        // A port nothing listens on, on this computer.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let credentials = DatabaseCredentials {
            database: Some("app".into()),
            user: Some("app".into()),
            ..Default::default()
        };
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::MySql,
            DatabaseEngine::SqlServer,
        ] {
            let mut target = requirement(PathBuf::new(), engine, "127.0.0.1");
            target.port = Some(port);
            let test = test_connection(&target, &credentials, Duration::from_secs(5)).unwrap();
            assert_eq!(test.outcome, ConnectionOutcome::NotResponding, "{engine:?}");
            assert_eq!(test.target, format!("127.0.0.1:{port}"));
        }
    }
}
