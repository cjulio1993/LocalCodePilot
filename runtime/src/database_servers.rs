//! Database servers the app prepares and runs for the user: PostgreSQL, MySQL and MariaDB
//! from the pinned packages (`database_packages`) on Windows, and MySQL and MariaDB on macOS
//! and Linux where a package exists (T029). Each server has its own data folder under
//! `<root>/servidores/<id>`, listens only on 127.0.0.1, and keeps its administrator
//! password in the system's vault (`platform::credentials`).
//!
//! Decisions agreed with the product side are in
//! `.internal/conversas/2026-11-20-t019-02-servidor-gerenciado.md`:
//! - the password is saved and read back from the vault before anything is initialized;
//!   a temporary file passes it to `initdb`/`mysqld` and is removed as soon as the server
//!   has consumed it; it never goes on a command line;
//! - a server is "managed" only when PID, start time, executable, data folder and the
//!   server's own PID file all match; any doubt makes it an existing server, never stopped;
//! - stopping is the server's own clean shutdown; one that does not stop in time is
//!   reported and left running, never killed (the only exception is a server created in
//!   this very operation that never accepted a connection).

use crate::database_packages::DatabasePackage;
use localcodepilot_core::ports::Port;
use localcodepilot_platform::{credentials, installations::configure_background_command, ports};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// The administrator account of a managed server. Its name is not a secret.
pub const POSTGRES_ADMIN: &str = "localcodepilot_admin";
pub const MYSQL_ADMIN: &str = "root";

const START_TIMEOUT: Duration = Duration::from_secs(90);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const REGISTRY_FILE: &str = "servidores.json";
const IDENTITY_FILE: &str = ".localcodepilot-servidor";
const TEMP_FOLDER: &str = ".temp";

/// What the app remembers about a server it prepared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedServer {
    /// Random, chosen at initialization; names the credential and the folder.
    pub id: String,
    /// `postgresql`, `mysql` or `mariadb`.
    pub engine: String,
    pub version: String,
    pub port: u16,
    /// The prepared package (`<root>\<engine>-<version>`).
    pub program_folder: PathBuf,
    /// `<root>\servidores\<id>`: data, configuration and log.
    pub folder: PathBuf,
    /// The running process, as started by the app.
    pub process_id: Option<u32>,
    pub started_at: Option<u64>,
    /// The app closed while this server did not stop in time; cleared once it is seen
    /// stopped, or stopped by the app.
    #[serde(default)]
    pub stop_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// The port is used by another program: nothing was started.
    PortInUse(u16),
    /// The vault could not keep the password: nothing was prepared.
    Vault(String),
    /// The administrator password is not in the vault: nothing is changed on the server.
    CredentialMissing,
    /// The Visual C++ runtime the server needs is missing or old.
    MissingRuntime(Vec<String>),
    /// The server did not start; the log tail explains why.
    StartFailed {
        detail: String,
        log: String,
    },
    /// The clean shutdown did not finish in time; the server may still be running.
    StopTimedOut,
    /// The server answered but did not accept the administrator password.
    Authentication(String),
    /// The project's database or user name has characters the app does not create.
    InvalidName,
    /// The server on the port is not recognized as the app's: nothing is changed on it.
    NotManaged(String),
    Io(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse(port) => write!(
                formatter,
                "a porta {port} já está em uso; nenhum banco foi iniciado"
            ),
            Self::Vault(detail) => write!(
                formatter,
                "Não foi possível guardar a senha do banco no Gerenciador de Credenciais do Windows. Nenhum servidor foi preparado. ({detail})"
            ),
            Self::CredentialMissing => write!(
                formatter,
                "A senha de administração deste banco não está disponível. O LocalCodePilot não fará alterações nele."
            ),
            Self::MissingRuntime(missing) => write!(
                formatter,
                "falta o Microsoft Visual C++ Redistributable: {}",
                missing.join("; ")
            ),
            Self::StartFailed { detail, .. } => write!(formatter, "o banco não iniciou: {detail}"),
            Self::StopTimedOut => write!(
                formatter,
                "Não foi possível parar o banco de dados. Ele pode continuar rodando."
            ),
            Self::Authentication(detail) => write!(
                formatter,
                "o banco não aceitou a senha de administração: {detail}"
            ),
            Self::InvalidName => write!(
                formatter,
                "O nome do banco ou do usuário na configuração deste projeto não pode ser criado automaticamente. Confira os nomes em Detalhes."
            ),
            Self::NotManaged(reason) => write!(
                formatter,
                "o servidor na porta não é o que o LocalCodePilot preparou ({reason}); nada foi alterado nele"
            ),
            Self::Io(detail) => write!(formatter, "{detail}"),
        }
    }
}

fn io(context: &str) -> impl Fn(std::io::Error) -> ServerError + '_ {
    move |error| ServerError::Io(format!("{context}: {error}"))
}

/// The vault, injected so tests never touch the user's real Credential Manager.
pub trait Vault {
    fn save_new(&self, target: &str, user: &str, secret: &str) -> Result<(), String>;
    fn read(&self, target: &str) -> Result<Option<String>, String>;
    fn delete(&self, target: &str) -> Result<(), String>;
}

/// The Windows Credential Manager.
pub struct SystemVault;

impl Vault for SystemVault {
    fn save_new(&self, target: &str, user: &str, secret: &str) -> Result<(), String> {
        credentials::save_new(target, user, secret)
    }

    fn read(&self, target: &str) -> Result<Option<String>, String> {
        credentials::read(target)
    }

    fn delete(&self, target: &str) -> Result<(), String> {
        credentials::delete(target)
    }
}

pub fn credential_target(id: &str) -> String {
    format!("LocalCodePilot/banco/{id}")
}

/// The Visual C++ DLLs each server imports and its package does not bring (checked on the
/// pinned executables: MariaDB ships them in `bin`).
fn runtime_dlls(engine: &str) -> &'static [&'static str] {
    match engine {
        "postgresql" => &["vcruntime140.dll"],
        "mysql" => &["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"],
        _ => &[],
    }
}

impl ManagedServer {
    pub fn data_folder(&self) -> PathBuf {
        self.folder.join("dados")
    }

    pub fn log_file(&self) -> PathBuf {
        self.folder.join("servidor.log")
    }

    fn config_file(&self) -> PathBuf {
        self.folder.join("my.ini")
    }

    fn pid_file(&self) -> PathBuf {
        match self.engine.as_str() {
            "postgresql" => self.data_folder().join("postmaster.pid"),
            _ => self.folder.join("servidor.pid"),
        }
    }

    /// A program of the package's `bin` folder, by its name without `.exe`.
    fn program(&self, name: &str) -> PathBuf {
        self.program_folder
            .join("bin")
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    fn server_program(&self) -> PathBuf {
        self.program(match self.engine.as_str() {
            "postgresql" => "postgres",
            "mysql" => "mysqld",
            _ => "mariadbd",
        })
    }

    /// MySQL and MariaDB also listen on a Unix socket outside Windows. It goes to the
    /// temporary folder, not the server folder: the path of a socket is limited to about
    /// 100 characters, and the app's folder on macOS is already long.
    fn socket_file(&self) -> PathBuf {
        let short = self.id.get(..12).unwrap_or(&self.id);
        std::env::temp_dir().join(format!("lcp-{short}.sock"))
    }

    pub fn admin_user(&self) -> &'static str {
        if self.engine == "postgresql" {
            POSTGRES_ADMIN
        } else {
            MYSQL_ADMIN
        }
    }

    pub fn admin_password(&self, vault: &dyn Vault) -> Result<String, ServerError> {
        vault
            .read(&credential_target(&self.id))
            .map_err(ServerError::Vault)?
            .ok_or(ServerError::CredentialMissing)
    }
}

/// `my.ini` for MySQL and MariaDB: paths, port and loopback only, no secret. The error
/// log is chosen per start: the file in a normal start, the console (read and masked by
/// the app) when the server may see the password.
fn mysql_config(server: &ManagedServer) -> String {
    let slash = |path: &Path| path.display().to_string().replace('\\', "/");
    let mut config = format!(
        "[mysqld]\nbasedir={}\ndatadir={}\nport={}\nbind-address=127.0.0.1\npid-file={}\n",
        slash(&server.program_folder),
        slash(&server.data_folder()),
        server.port,
        slash(&server.pid_file()),
    );
    if !cfg!(windows) {
        config.push_str(&format!("socket={}\n", slash(&server.socket_file())));
    }
    if server.engine == "mysql" {
        // No X Protocol: no second port (33060) next to the one the project uses.
        config.push_str("mysqlx=OFF\n");
    }
    config
}

/// SQL run by the server at its first start, before it accepts connections: protects every
/// root account and removes the anonymous ones. Only quote-free passwords are generated.
fn mysql_init_sql(password: &str) -> String {
    debug_assert!(!password.contains(['\'', '\\']));
    let mut sql = String::new();
    for host in ["localhost", "127.0.0.1", "::1"] {
        sql.push_str(&format!(
            "ALTER USER IF EXISTS 'root'@'{host}' IDENTIFIED BY '{password}';\n"
        ));
    }
    sql.push_str("DROP USER IF EXISTS ''@'localhost';\nFLUSH PRIVILEGES;\n");
    sql
}

/// A random administrator password: 32 bytes in URL-safe base64, without quotes or
/// backslashes, so it is safe inside the init SQL.
fn new_password() -> Result<String, ServerError> {
    let mut bytes = [0_u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| ServerError::Io("não foi possível gerar uma senha aleatória".into()))?;
    Ok(base64_url(&bytes))
}

fn new_id() -> Result<String, ServerError> {
    let mut bytes = [0_u8; 9];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| ServerError::Io("não foi possível gerar um identificador".into()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |value, (index, byte)| {
                value | u32::from(*byte) << (16 - 8 * index)
            });
        for index in 0..=chunk.len() {
            text.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    text
}

// --- Temporary password files ---------------------------------------------------------

/// A file with the password for `initdb --pwfile` or `mysqld --init-file`. Its name carries
/// the owner (PID and process start time), so a sweep only removes files whose owner is
/// gone; it is removed when dropped, even on error.
struct SecretFile {
    path: PathBuf,
}

impl SecretFile {
    fn create(root: &Path, contents: &str) -> Result<Self, ServerError> {
        use std::io::Write;
        let folder = root.join(TEMP_FOLDER);
        fs::create_dir_all(&folder).map_err(io("não foi possível criar a pasta temporária"))?;
        let pid = std::process::id();
        let started = ports::process_started_at(pid).unwrap_or_default();
        let path = folder.join(format!("{pid}-{started}-{}.tmp", new_id()?));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io("não foi possível criar o arquivo temporário"))?;
        let written = file
            .write_all(contents.as_bytes())
            .and_then(|()| file.sync_all());
        let secret = Self { path };
        written.map_err(io("não foi possível gravar o arquivo temporário"))?;
        Ok(secret)
    }

    /// Removes the file now, reporting a failure (the caller must not go on).
    fn remove(&self) -> Result<(), ServerError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ServerError::Io(format!(
                "o arquivo temporário com a senha não pôde ser apagado: {error}"
            ))),
        }
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Removes temporary password files left by an app that is no longer running. A file
/// whose owner (same PID and same start time) still runs belongs to an operation in
/// progress and stays.
pub fn sweep_abandoned_secrets(root: &Path) {
    sweep_abandoned_secrets_with(root, ports::process_started_at);
}

fn sweep_abandoned_secrets_with(root: &Path, started_at: impl Fn(u32) -> Option<u64>) {
    let Ok(entries) = fs::read_dir(root.join(TEMP_FOLDER)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let mut parts = name.splitn(3, '-');
        let owner = parts.next().and_then(|pid| pid.parse::<u32>().ok());
        let start = parts.next().and_then(|start| start.parse::<u64>().ok());
        let alive = match (owner, start) {
            (Some(pid), Some(start)) => started_at(pid) == Some(start),
            _ => false,
        };
        if !alive {
            let _ = fs::remove_file(entry.path());
        }
    }
}

// --- Registry ---------------------------------------------------------------------------

pub fn load_registry(root: &Path) -> Vec<ManagedServer> {
    fs::read_to_string(root.join(REGISTRY_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The server the app prepared for `engine` (`postgresql`, `mysql`, `mariadb`) on `port`.
pub fn find_server(root: &Path, engine: &str, port: u16) -> Option<ManagedServer> {
    load_registry(root)
        .into_iter()
        .find(|server| server.engine == engine && server.port == port)
}

/// Remembers (or forgets) that a server did not stop when the app closed.
pub fn mark_stop_failed(
    root: &Path,
    server: &ManagedServer,
    failed: bool,
) -> Result<(), ServerError> {
    let mut server = server.clone();
    server.stop_failed = failed;
    save_server(root, &server)
}

fn save_server(root: &Path, server: &ManagedServer) -> Result<(), ServerError> {
    let mut servers = load_registry(root);
    servers.retain(|known| known.id != server.id);
    servers.push(server.clone());
    let text = serde_json::to_string_pretty(&servers)
        .map_err(|error| ServerError::Io(error.to_string()))?;
    let path = root.join(REGISTRY_FILE);
    let partial = path.with_extension("json.part");
    fs::write(&partial, text).map_err(io("não foi possível gravar o registro dos bancos"))?;
    fs::rename(&partial, &path).map_err(io("não foi possível gravar o registro dos bancos"))
}

// --- Recognition ------------------------------------------------------------------------

/// Who is listening on a managed server's port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recognition {
    /// The server the app started, with every identity check passing.
    Managed,
    /// Nothing listens on the port.
    NotRunning,
    /// Something else, or the app cannot be sure: never stopped or changed by the app.
    Existing { reason: String },
}

/// What the recognition needs from the system, injected for tests.
pub struct ProcessView<'a> {
    pub listener: &'a dyn Fn(u16) -> Option<u32>,
    pub details: &'a dyn Fn(u32) -> Option<ports::ProcessDetails>,
}

pub fn recognize(server: &ManagedServer) -> Recognition {
    let listener = |port: u16| Port::new(port).and_then(ports::listener_process_id);
    recognize_with(
        server,
        &ProcessView {
            listener: &listener,
            details: &ports::process_details,
        },
        fs::read_to_string(server.pid_file()).ok().as_deref(),
    )
}

fn recognize_with(
    server: &ManagedServer,
    view: &ProcessView<'_>,
    pid_file: Option<&str>,
) -> Recognition {
    let existing = |reason: &str| Recognition::Existing {
        reason: reason.to_owned(),
    };
    let Some(owner) = (view.listener)(server.port) else {
        return if port_is_free(server.port) {
            Recognition::NotRunning
        } else {
            existing("a porta está ocupada e o processo não pôde ser identificado")
        };
    };
    if server.process_id != Some(owner) {
        return existing("o processo na porta não é o que o LocalCodePilot iniciou");
    }
    let Some(details) = (view.details)(owner) else {
        return existing("o processo na porta não pôde ser consultado");
    };
    if server.started_at != Some(details.started_at) {
        return existing("o número do processo foi reaproveitado por outro programa");
    }
    let program = server.server_program();
    let same_program = details
        .executable
        .as_deref()
        .is_some_and(|executable| same_path(executable, &program));
    if !same_program {
        return existing("o programa na porta não é o servidor preparado pelo LocalCodePilot");
    }
    let data = server
        .data_folder()
        .display()
        .to_string()
        .replace('\\', "/");
    let config = server
        .config_file()
        .display()
        .to_string()
        .replace('\\', "/");
    let mentions_ours = details.command_line.iter().any(|part| {
        let part = part.replace('\\', "/").to_ascii_lowercase();
        part.contains(&data.to_ascii_lowercase()) || part.contains(&config.to_ascii_lowercase())
    });
    if !mentions_ours {
        return existing("o servidor na porta usa outra pasta de dados");
    }
    let pid_matches = pid_file
        .and_then(|text| text.lines().next())
        .and_then(|line| line.trim().parse::<u32>().ok())
        == Some(owner);
    if !pid_matches {
        return existing("o arquivo de PID do servidor não confere");
    }
    Recognition::Managed
}

fn same_path(a: &Path, b: &Path) -> bool {
    let normalize = |path: &Path| {
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
    };
    normalize(a) == normalize(b)
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).is_ok()
        && Port::new(port).is_none_or(|port| ports::listener_process_id(port).is_none())
}

// --- Life cycle -------------------------------------------------------------------------

/// Prepares a new server of `package` on `port`: saves the password in the vault, creates
/// the data folder, starts the server and confirms the administrator login. The server is
/// left running.
pub fn prepare(
    package: &DatabasePackage,
    root: &Path,
    port: u16,
    vault: &dyn Vault,
) -> Result<ManagedServer, ServerError> {
    sweep_abandoned_secrets(root);
    let program_folder = package.folder(root);
    let missing = localcodepilot_platform::vc_runtime::missing_runtime(
        &program_folder.join(&package.server_program),
        runtime_dlls(&package.engine),
    );
    if !missing.is_empty() {
        return Err(ServerError::MissingRuntime(missing));
    }
    if !port_is_free(port) {
        return Err(ServerError::PortInUse(port));
    }
    let id = new_id()?;
    let mut server = ManagedServer {
        engine: package.engine.clone(),
        version: package.version.clone(),
        port,
        program_folder,
        folder: root.join("servidores").join(&id),
        id,
        process_id: None,
        started_at: None,
        stop_failed: false,
    };
    let password = new_password()?;
    vault
        .save_new(
            &credential_target(&server.id),
            server.admin_user(),
            &password,
        )
        .map_err(ServerError::Vault)?;
    // Read back what was saved: the server is initialized with the vault's copy.
    let password = server.admin_password(vault)?;
    fs::create_dir_all(&server.folder).map_err(io("não foi possível criar a pasta do banco"))?;
    fs::write(server.folder.join(IDENTITY_FILE), &server.id)
        .map_err(io("não foi possível marcar a pasta do banco"))?;
    save_server(root, &server)?;
    let result = initialize_and_start(&mut server, root, &password, vault);
    // Every program that sees the password writes through the app, which masks it before
    // anything reaches the disk (a server may echo part of a failed statement: MySQL writes
    // "near '...'"). This last check proves the log is clean; a log that still had the
    // password and could not be cleaned fails the operation, before any other error.
    let cleaned = scrub_file(&server.log_file(), &password);
    cleaned?;
    result.map_err(|error| error.scrubbed(&password))?;
    Ok(server)
}

fn initialize_and_start(
    server: &mut ManagedServer,
    root: &Path,
    password: &str,
    vault: &dyn Vault,
) -> Result<(), ServerError> {
    match server.engine.as_str() {
        "postgresql" => {
            initialize_postgres(server, root, password)?;
            start(server, root)?;
        }
        _ => {
            initialize_mysql(server, password)?;
            let init = SecretFile::create(root, &mysql_init_sql(password))?;
            let started = start_with(server, root, Some((&init, password)));
            // The server writes "ready for connections" only after running its init file:
            // once the start returns, the file was consumed. It goes now, whatever happened.
            let removed = init.remove();
            started?;
            if let Err(error) = removed {
                // A server created in this operation that never accepted a connection: the
                // only case where the app ends a server itself. The folder is kept.
                abandon_fresh_server(server);
                return Err(error);
            }
            // The first start ran with its output read by the app. Once the password is
            // set, the server restarts in the normal way, writing its own log, and never
            // sees the password again.
            save_server(root, server)?;
            admin_check(server, password)?;
            stop(server, root, vault)?;
            start(server, root)?;
        }
    }
    save_server(root, server)?;
    admin_check(server, password)
}

const MASK: &str = "••••";

/// `text` with every copy of `secret` replaced by the mask.
fn scrub(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, MASK)
    }
}

/// Whether `path` holds a copy of `secret`; `None` when it cannot be read.
fn file_has_secret(path: &Path, secret: &str) -> Option<bool> {
    match fs::read(path) {
        Ok(bytes) => Some(
            !secret.is_empty()
                && bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// Leaves `path` without any copy of `secret`, and proves it by reading it again. Tries,
/// in order: rewrite and replace, the same without the read-only attribute, rewrite in
/// place (a server may keep its log open), and removing the file. If the secret is still
/// there, or the file cannot be read to check, it fails: the caller must not go on as if
/// the log were clean. The error never carries the secret.
fn scrub_file(path: &Path, secret: &str) -> Result<(), ServerError> {
    let unreadable = || {
        ServerError::Io(format!(
            "o log do banco não pôde ser conferido para remover a senha: {}",
            path.display()
        ))
    };
    match file_has_secret(path, secret) {
        Some(false) => return Ok(()),
        None => return Err(unreadable()),
        Some(true) => {}
    }
    let clean = |path: &Path| file_has_secret(path, secret) == Some(false);
    let text = fs::read(path)
        .map(|bytes| scrub(&String::from_utf8_lossy(&bytes), secret))
        .map_err(|_| unreadable())?;
    let partial = path.with_extension("log.part");
    let replace =
        || fs::write(&partial, &text).is_ok() && fs::rename(&partial, path).is_ok() && clean(path);
    let mut done = replace();
    if !done && let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        done = fs::set_permissions(path, permissions).is_ok() && replace();
    }
    if !done {
        done = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
            .is_ok()
            && clean(path);
    }
    if !done {
        done = fs::remove_file(path).is_ok() && clean(path);
    }
    let _ = fs::remove_file(&partial);
    if done {
        Ok(())
    } else {
        Err(ServerError::Io(format!(
            "o log do banco ficou com a senha e não pôde ser limpo nem apagado; apague o arquivo {}",
            path.display()
        )))
    }
}

impl ServerError {
    fn scrubbed(self, secret: &str) -> Self {
        match self {
            Self::StartFailed { detail, log } => Self::StartFailed {
                detail: scrub(&detail, secret),
                log: scrub(&log, secret),
            },
            Self::Authentication(detail) => Self::Authentication(scrub(&detail, secret)),
            Self::Io(detail) => Self::Io(scrub(&detail, secret)),
            Self::Vault(detail) => Self::Vault(scrub(&detail, secret)),
            other => other,
        }
    }
}

fn initialize_postgres(
    server: &ManagedServer,
    root: &Path,
    password: &str,
) -> Result<(), ServerError> {
    let password_file = SecretFile::create(root, password)?;
    let mut command = Command::new(server.program("initdb"));
    command
        .arg("-D")
        .arg(server.data_folder())
        .args([
            "-U",
            POSTGRES_ADMIN,
            "-E",
            "UTF8",
            "--locale=C",
            "--auth=scram-sha-256",
        ])
        .arg(format!("--pwfile={}", password_file.path.display()));
    let result = run_to_end(command, &server.log_file(), "initdb", Some(password));
    // initdb has exited: nothing reads the file any more.
    let removed = password_file.remove();
    result?;
    removed
}

fn initialize_mysql(server: &ManagedServer, password: &str) -> Result<(), ServerError> {
    fs::write(server.config_file(), mysql_config(server))
        .map_err(io("não foi possível gravar a configuração do banco"))?;
    link_system_libraries(server)?;
    let mut command = if server.engine == "mysql" {
        let mut command = Command::new(server.program("mysqld"));
        command
            .arg(format!(
                "--defaults-file={}",
                server.config_file().display()
            ))
            .arg("--initialize-insecure");
        // Outside Windows the errors already go to the console (no log-error set), and
        // `--console` is a Windows option.
        if cfg!(windows) {
            command.arg("--console");
        }
        command
    } else if cfg!(windows) {
        let mut command = Command::new(server.program("mariadb-install-db"));
        command.arg(format!("--datadir={}", server.data_folder().display()));
        command
    } else {
        // The Linux package has the script in `scripts`. "normal" creates a root that logs
        // in with a password, as on Windows (the init file sets it), instead of the system
        // account through the socket.
        let mut command = Command::new(
            server
                .program_folder
                .join("scripts")
                .join("mariadb-install-db"),
        );
        command
            .arg(format!(
                "--defaults-file={}",
                server.config_file().display()
            ))
            .arg(format!("--basedir={}", server.program_folder.display()))
            .arg(format!("--datadir={}", server.data_folder().display()))
            .arg("--auth-root-authentication-method=normal");
        command
    };
    command.current_dir(&server.folder);
    run_to_end(command, &server.log_file(), "inicialização", Some(password))
}

/// MySQL on Linux loads `libaio.so.1`, which newer systems ship as `libaio.so.1t64`:
/// the package gets a link of its own (see `database_packages::libaio`). Nothing elsewhere.
fn link_system_libraries(server: &ManagedServer) -> Result<(), ServerError> {
    if cfg!(target_os = "linux") && server.engine == "mysql" {
        crate::database_packages::libaio::link_if_needed(&server.program_folder)
            .map_err(io("não foi possível preparar a biblioteca libaio do MySQL"))?;
    }
    Ok(())
}

/// Opens the log for a program. With a `secret`, the program writes to pipes the app reads,
/// and each line reaches the log only after the secret is masked: it is never on disk.
fn attach_output(
    command: &mut Command,
    log: &Path,
    secret: Option<&str>,
) -> Result<(), ServerError> {
    command.stdin(Stdio::null());
    if secret.is_some() {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        return Ok(());
    }
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(io("não foi possível abrir o log do banco"))?;
    let errors = output
        .try_clone()
        .map_err(io("não foi possível abrir o log do banco"))?;
    command
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(errors));
    Ok(())
}

/// Copies the program's piped output to the log, line by line, with `secret` masked.
/// The threads end when the program closes its output.
fn copy_masked(child: &mut Child, log: &Path, secret: &str) -> Vec<std::thread::JoinHandle<()>> {
    let streams: Vec<Box<dyn std::io::Read + Send>> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    streams
        .into_iter()
        .map(|stream| {
            let log = log.to_path_buf();
            let secret = secret.to_owned();
            std::thread::spawn(move || {
                use std::io::{BufRead, Write};
                let mut reader = std::io::BufReader::new(stream);
                let mut line = Vec::new();
                while reader
                    .read_until(b'\n', &mut line)
                    .is_ok_and(|read| read > 0)
                {
                    let masked = scrub(&String::from_utf8_lossy(&line), &secret);
                    if let Ok(mut file) =
                        fs::OpenOptions::new().create(true).append(true).open(&log)
                    {
                        let _ = file.write_all(masked.as_bytes());
                    }
                    line.clear();
                }
            })
        })
        .collect()
}

fn run_to_end(
    mut command: Command,
    log: &Path,
    name: &str,
    secret: Option<&str>,
) -> Result<(), ServerError> {
    attach_output(&mut command, log, secret)?;
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(io("não foi possível iniciar o programa do banco"))?;
    let copies = secret
        .map(|secret| copy_masked(&mut child, log, secret))
        .unwrap_or_default();
    let status = child
        .wait()
        .map_err(io("não foi possível acompanhar o programa do banco"))?;
    for copy in copies {
        let _ = copy.join();
    }
    if status.success() {
        Ok(())
    } else {
        Err(ServerError::StartFailed {
            detail: format!("{name} terminou com {status}"),
            log: log_tail(log),
        })
    }
}

/// Starts a prepared server and waits until its port answers.
pub fn start(server: &mut ManagedServer, root: &Path) -> Result<(), ServerError> {
    start_with(server, root, None)
}

/// `first_start` is MySQL's init file with the password it sets: that start writes to the
/// console, read and masked by the app, instead of its own log file.
fn start_with(
    server: &mut ManagedServer,
    root: &Path,
    first_start: Option<(&SecretFile, &str)>,
) -> Result<(), ServerError> {
    if !port_is_free(server.port) {
        return Err(ServerError::PortInUse(server.port));
    }
    link_system_libraries(server)?;
    let mut command = Command::new(server.server_program());
    if server.engine == "postgresql" {
        command
            .arg("-D")
            .arg(server.data_folder())
            .arg("-p")
            .arg(server.port.to_string())
            .args(["-c", "listen_addresses=127.0.0.1"]);
    } else {
        // `--defaults-file` must be the first option.
        command.arg(format!(
            "--defaults-file={}",
            server.config_file().display()
        ));
        if server.engine == "mysql" && cfg!(windows) {
            // On Windows MySQL otherwise starts a monitor process that runs the real server
            // as a child: the port would belong to a PID the app did not start, and the
            // server would never be recognized as the app's (nor stopped by it).
            command.arg("--no-monitor");
        }
        match first_start {
            Some((init, _)) => {
                // Without log-error the errors go to the console; `--console` says so on
                // Windows, where it is the only system that has the option.
                if cfg!(windows) {
                    command.arg("--console");
                }
                command.arg(format!("--init-file={}", init.path.display()));
            }
            None => {
                command.arg(format!("--log-error={}", server.log_file().display()));
            }
        }
    }
    // Only what this start writes counts as its readiness.
    let log_start = fs::metadata(server.log_file())
        .map(|meta| meta.len())
        .unwrap_or(0);
    let secret = first_start.map(|(_, secret)| secret);
    attach_output(&mut command, &server.log_file(), secret)?;
    command.current_dir(&server.folder);
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(io("não foi possível iniciar o banco"))?;
    // The copies live as long as the server's output: until this first start is stopped.
    if let Some(secret) = secret {
        drop(copy_masked(&mut child, &server.log_file(), secret));
    }
    server.process_id = Some(child.id());
    server.started_at = ports::process_started_at(child.id());
    // MySQL and MariaDB are ready, and have run their init file, only when they write
    // "ready for connections": MariaDB opens its port before it reads the init file.
    let ready_line = (server.engine != "postgresql").then(|| (server.log_file(), log_start));
    let waited = wait_until_ready(&mut child, server.port, ready_line, START_TIMEOUT);
    save_server(root, server)?;
    waited.map_err(|detail| ServerError::StartFailed {
        detail,
        log: log_tail(&server.log_file()),
    })
}

/// Whether the log, from byte `from`, has the server's "ready for connections" line.
fn log_says_ready(log: &Path, from: u64) -> bool {
    fs::read(log).is_ok_and(|bytes| {
        let start = usize::try_from(from).unwrap_or(usize::MAX).min(bytes.len());
        String::from_utf8_lossy(&bytes[start..]).contains("ready for connections")
    })
}

fn wait_until_ready(
    child: &mut Child,
    port: u16,
    ready_line: Option<(PathBuf, u64)>,
    timeout: Duration,
) -> Result<(), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let started = Instant::now();
    loop {
        let ready = ready_line
            .as_ref()
            .is_none_or(|(log, from)| log_says_ready(log, *from));
        if ready && TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("o servidor terminou ao iniciar ({status})"));
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "o servidor não abriu a porta {port} em {} segundos",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Stops a managed server with its own clean shutdown. Only a server recognized as the
/// app's is touched; one that does not stop in time is left running and reported.
pub fn stop(server: &mut ManagedServer, root: &Path, vault: &dyn Vault) -> Result<(), ServerError> {
    match recognize(server) {
        Recognition::NotRunning => return Ok(()),
        Recognition::Existing { reason } => {
            return Err(ServerError::Io(format!(
                "o LocalCodePilot não para este banco: {reason}"
            )));
        }
        Recognition::Managed => {}
    }
    match server.engine.as_str() {
        "postgresql" => {
            let mut command = Command::new(server.program("pg_ctl"));
            command
                .arg("stop")
                .arg("-D")
                .arg(server.data_folder())
                .args([
                    "-m",
                    "fast",
                    "-w",
                    "-t",
                    &STOP_TIMEOUT.as_secs().to_string(),
                ]);
            // pg_ctl reports the timeout as a failure; the port check below decides.
            let _ = run_to_end(command, &server.log_file(), "pg_ctl stop", None);
        }
        _ => {
            let password = server.admin_password(vault)?;
            mysql_admin(server, &password, "SHUTDOWN")
                .map_err(|error| error.scrubbed(&password))?;
        }
    }
    // Stopped means the process has ended, not only that the port closed: MySQL closes the
    // port first and keeps its files open while it finishes writing them.
    let process_gone = |server: &ManagedServer| {
        server.process_id.is_none_or(|pid| {
            server.started_at.is_none() || ports::process_started_at(pid) != server.started_at
        })
    };
    let started = Instant::now();
    while started.elapsed() < STOP_TIMEOUT {
        if port_is_free(server.port) && process_gone(server) {
            server.process_id = None;
            server.started_at = None;
            server.stop_failed = false;
            return save_server(root, server);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(ServerError::StopTimedOut)
}

/// The one exception to "never kill": a server created in this very operation that never
/// accepted a connection, whose password file could not be removed. Its folder stays.
fn abandon_fresh_server(server: &ManagedServer) {
    if let Some(pid) = server.process_id
        && ports::process_started_at(pid) == server.started_at
    {
        let _ = ports::terminate_process_tree(pid);
    }
}

fn log_tail(log: &Path) -> String {
    // Server tools may write in the Windows code page (pg_ctl in Portuguese is cp1252):
    // read the bytes, so a line that is not UTF-8 never hides the rest of the log.
    let text = fs::read(log)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

// --- The project's database and user ---------------------------------------------------

/// Names the app creates: letters, digits and `_`, up to 63 characters, not starting with a
/// digit. Anything else is left to the person (no quoting games with other servers' rules).
pub fn creatable_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

/// A MySQL string literal body: backslashes and quotes escaped.
fn mysql_literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "''")
}

/// A PostgreSQL string literal body (standard_conforming_strings): quotes doubled.
fn postgres_literal(text: &str) -> String {
    text.replace('\'', "''")
}

/// The statements that give the project its database and user on MySQL/MariaDB. Privileges
/// stay on the project's database (`db.*`), never `*.*` nor `GRANT OPTION`; an existing user
/// is not changed (`IF NOT EXISTS`).
fn mysql_project_sql(database: &str, user: &str, password: &str) -> Vec<String> {
    let mut sql = vec![format!("CREATE DATABASE IF NOT EXISTS `{database}`")];
    for host in ["localhost", "127.0.0.1"] {
        sql.push(format!(
            "CREATE USER IF NOT EXISTS '{user}'@'{host}' IDENTIFIED BY '{}'",
            mysql_literal(password)
        ));
        sql.push(format!(
            "GRANT ALL PRIVILEGES ON `{database}`.* TO '{user}'@'{host}'"
        ));
    }
    sql
}

/// PostgreSQL: a login role without SUPERUSER or CREATEDB, owner of the project's database.
fn postgres_role_sql(user: &str, password: &str) -> String {
    format!(
        "CREATE ROLE \"{user}\" LOGIN PASSWORD '{}'",
        postgres_literal(password)
    )
}

fn postgres_database_sql(database: &str, user: &str) -> String {
    format!("CREATE DATABASE \"{database}\" OWNER \"{user}\"")
}

/// Gives the project its database and user on a server the app manages, only what is
/// missing. Returns what was created, for "Detalhes técnicos". The project's connection test
/// decides afterwards whether it is ready: nothing here presumes it.
pub fn ensure_project_database(
    server: &ManagedServer,
    vault: &dyn Vault,
    database: &str,
    user: &str,
    password: &str,
) -> Result<Vec<String>, ServerError> {
    if !creatable_name(database) || !creatable_name(user) {
        return Err(ServerError::InvalidName);
    }
    match recognize(server) {
        Recognition::Managed => {}
        Recognition::NotRunning => {
            return Err(ServerError::NotManaged(
                "o servidor não está rodando".into(),
            ));
        }
        Recognition::Existing { reason } => return Err(ServerError::NotManaged(reason)),
    }
    let admin = server.admin_password(vault)?;
    let result = if server.engine == "postgresql" {
        postgres_ensure(server, &admin, database, user, password)
    } else {
        mysql_ensure(server, &admin, database, user, password)
    };
    result.map_err(|error| error.scrubbed(password).scrubbed(&admin))
}

fn postgres_ensure(
    server: &ManagedServer,
    admin: &str,
    database: &str,
    user: &str,
    password: &str,
) -> Result<Vec<String>, ServerError> {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(server.port)
        .user(POSTGRES_ADMIN)
        .password(admin)
        .dbname("postgres")
        .connect_timeout(Duration::from_secs(10));
    let failed = |error: tokio_postgres::Error| ServerError::Authentication(error.to_string());
    runtime()?.block_on(async {
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(failed)?;
        let driver = tokio::spawn(connection);
        let mut done = vec![];
        let role = client
            .query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&user])
            .await
            .map_err(failed)?;
        if role.is_none() {
            client
                .batch_execute(&postgres_role_sql(user, password))
                .await
                .map_err(failed)?;
            done.push(format!("criado o usuário {user}"));
        }
        let exists = client
            .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&database])
            .await
            .map_err(failed)?;
        if exists.is_none() {
            client
                .batch_execute(&postgres_database_sql(database, user))
                .await
                .map_err(failed)?;
            done.push(format!("criado o banco {database}"));
        }
        drop(client);
        let _ = driver.await;
        Ok(done)
    })
}

fn mysql_ensure(
    server: &ManagedServer,
    admin: &str,
    database: &str,
    user: &str,
    password: &str,
) -> Result<Vec<String>, ServerError> {
    use mysql_async::prelude::Queryable;
    let ssl = mysql_async::SslOpts::default().with_danger_accept_invalid_certs(true);
    let options = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(server.port)
        .user(Some(MYSQL_ADMIN))
        .pass(Some(admin))
        .ssl_opts(ssl);
    let failed = |error: mysql_async::Error| ServerError::Authentication(error.to_string());
    runtime()?.block_on(async {
        let mut connection = mysql_connect(options).await?;
        let had_database: Option<String> = connection
            .exec_first(
                "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?",
                (database,),
            )
            .await
            .map_err(failed)?;
        let had_user: Option<String> = connection
            .exec_first(
                "SELECT User FROM mysql.user WHERE User = ? LIMIT 1",
                (user,),
            )
            .await
            .map_err(failed)?;
        for statement in mysql_project_sql(database, user, password) {
            connection.query_drop(statement).await.map_err(failed)?;
        }
        let _ = connection.disconnect().await;
        let mut done = vec![];
        if had_user.is_none() {
            done.push(format!("criado o usuário {user}"));
        }
        if had_database.is_none() {
            done.push(format!("criado o banco {database}"));
        }
        Ok(done)
    })
}

/// Sets the project user's password to the project's own, on a server the app manages.
/// Used by "Corrigir conexão", after a confirmation, only when no other known project uses
/// the same user.
pub fn set_project_password(
    server: &ManagedServer,
    vault: &dyn Vault,
    user: &str,
    password: &str,
) -> Result<(), ServerError> {
    if !creatable_name(user) {
        return Err(ServerError::InvalidName);
    }
    match recognize(server) {
        Recognition::Managed => {}
        Recognition::NotRunning => {
            return Err(ServerError::NotManaged(
                "o servidor não está rodando".into(),
            ));
        }
        Recognition::Existing { reason } => return Err(ServerError::NotManaged(reason)),
    }
    let admin = server.admin_password(vault)?;
    let result = if server.engine == "postgresql" {
        postgres_admin(
            server,
            &admin,
            &format!(
                "ALTER ROLE \"{user}\" PASSWORD '{}'",
                postgres_literal(password)
            ),
        )
    } else {
        let mut result = Ok(());
        for host in ["localhost", "127.0.0.1"] {
            result = result.and_then(|()| {
                mysql_admin(
                    server,
                    &admin,
                    &format!(
                        "ALTER USER IF EXISTS '{user}'@'{host}' IDENTIFIED BY '{}'",
                        mysql_literal(password)
                    ),
                )
            });
        }
        result
    };
    result.map_err(|error| error.scrubbed(password).scrubbed(&admin))
}

/// Whether a server log shows damaged data files, the only case where "Criar banco novo" is
/// offered. A locked file, a permission or a version mismatch is not damage.
pub fn log_shows_damage(engine: &str, log: &str) -> bool {
    let log = log.to_ascii_lowercase();
    let signs: &[&str] = if engine == "postgresql" {
        &[
            "could not locate a valid checkpoint record",
            "invalid checkpoint record",
            "invalid primary checkpoint record",
            "incorrect checksum in control file",
        ]
    } else {
        &[
            "database page corruption",
            "data structure corruption",
            "is corrupted",
            "corrupt page",
        ]
    };
    signs.iter().any(|sign| log.contains(sign))
}

/// The tail of a server's log, for the damage check and "Detalhes técnicos".
pub fn server_log_tail(server: &ManagedServer) -> String {
    log_tail(&server.log_file())
}

/// "Criar banco novo": keeps the current folder as `<folder>-guardada-<moment>` (nothing is
/// deleted) and creates the server again with the same identity, port and administrator
/// password (still in the vault). Returns the kept folder.
pub fn recreate_with_new_data(
    server: &mut ManagedServer,
    root: &Path,
    vault: &dyn Vault,
) -> Result<PathBuf, ServerError> {
    if recognize(server) != Recognition::NotRunning {
        return Err(ServerError::NotManaged(
            "o servidor ainda está rodando; pare-o antes".into(),
        ));
    }
    let password = server.admin_password(vault)?;
    let moment = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let kept = server
        .folder
        .with_file_name(format!("{}-guardada-{moment}", server.id));
    fs::rename(&server.folder, &kept).map_err(io("não foi possível guardar a pasta atual"))?;
    fs::create_dir_all(&server.folder).map_err(io("não foi possível criar a pasta do banco"))?;
    fs::write(server.folder.join(IDENTITY_FILE), &server.id)
        .map_err(io("não foi possível marcar a pasta do banco"))?;
    server.process_id = None;
    server.started_at = None;
    server.stop_failed = false;
    let result = initialize_and_start(server, root, &password, vault);
    let cleaned = scrub_file(&server.log_file(), &password);
    cleaned?;
    result.map_err(|error| error.scrubbed(&password))?;
    Ok(kept)
}
// --- Administrator connection -----------------------------------------------------------

fn runtime() -> Result<tokio::runtime::Runtime, ServerError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| ServerError::Io(format!("não foi possível preparar a conexão: {error}")))
}

/// Logs in as the administrator, proving the password in the vault is the server's.
pub fn admin_check(server: &ManagedServer, password: &str) -> Result<(), ServerError> {
    match server.engine.as_str() {
        "postgresql" => postgres_admin(server, password, "SELECT 1"),
        _ => mysql_admin(server, password, "SELECT 1"),
    }
    .map_err(|error| error.scrubbed(password))
}

fn postgres_admin(server: &ManagedServer, password: &str, sql: &str) -> Result<(), ServerError> {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(server.port)
        .user(POSTGRES_ADMIN)
        .password(password)
        .dbname("postgres")
        .connect_timeout(Duration::from_secs(10));
    runtime()?.block_on(async {
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(|error| ServerError::Authentication(error.to_string()))?;
        let driver = tokio::spawn(connection);
        let result = client
            .batch_execute(sql)
            .await
            .map_err(|error| ServerError::Authentication(error.to_string()));
        drop(client);
        let _ = driver.await;
        result
    })
}

/// How long an administrator login to the app's MySQL/MariaDB may take before it fails.
const MYSQL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// An administrator connection to the app's MySQL/MariaDB, with one TLS backend chosen and a
/// time limit: a server that accepts the socket but never answers fails instead of hanging.
async fn mysql_connect(
    options: mysql_async::OptsBuilder,
) -> Result<mysql_async::Conn, ServerError> {
    crate::tls_ready();
    match tokio::time::timeout(MYSQL_CONNECT_TIMEOUT, mysql_async::Conn::new(options)).await {
        Ok(Ok(connection)) => Ok(connection),
        Ok(Err(error)) => Err(ServerError::Authentication(error.to_string())),
        Err(_) => Err(ServerError::StartFailed {
            detail: format!(
                "o servidor não respondeu ao login de administração em {} s",
                MYSQL_CONNECT_TIMEOUT.as_secs()
            ),
            log: String::new(),
        }),
    }
}

fn mysql_admin(server: &ManagedServer, password: &str, sql: &str) -> Result<(), ServerError> {
    use mysql_async::prelude::Queryable;
    // MySQL 9 authenticates with caching_sha2_password, which needs TLS on the first login.
    // The server generates its own certificate; on 127.0.0.1, to the app's own server,
    // accepting it is safe.
    let ssl = mysql_async::SslOpts::default().with_danger_accept_invalid_certs(true);
    let options = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(server.port)
        .user(Some(MYSQL_ADMIN))
        .pass(Some(password))
        .ssl_opts(ssl);
    runtime()?.block_on(async {
        let mut connection = mysql_connect(options).await?;
        let result = connection.query_drop(sql).await;
        // SHUTDOWN closes the connection from the server side.
        let _ = connection.disconnect().await;
        result.or_else(|error| {
            if sql == "SHUTDOWN" {
                Ok(())
            } else {
                Err(ServerError::Authentication(error.to_string()))
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn server() -> ManagedServer {
        ManagedServer {
            id: "abc".into(),
            engine: "mysql".into(),
            version: "9.7.4".into(),
            port: 1,
            program_folder: Path::new("lcp").join("bancos").join("mysql-9.7.4"),
            folder: Path::new("lcp")
                .join("bancos")
                .join("servidores")
                .join("abc"),
            process_id: Some(4242),
            started_at: Some(1000),
            stop_failed: false,
        }
    }

    fn details(started_at: u64, executable: &str, data: &str) -> ports::ProcessDetails {
        ports::ProcessDetails {
            started_at,
            executable: Some(PathBuf::from(executable)),
            command_line: vec![executable.to_owned(), format!("--defaults-file={data}")],
        }
    }

    fn check(
        listener: Option<u32>,
        found: Option<ports::ProcessDetails>,
        pid_file: Option<&str>,
    ) -> Recognition {
        let listener_fn = move |_: u16| listener;
        let details_fn = move |_: u32| found.clone();
        recognize_with(
            &server(),
            &ProcessView {
                listener: &listener_fn,
                details: &details_fn,
            },
            pid_file,
        )
    }

    // Paths built with `join`, so the tests mean the same on every system.
    fn exe() -> String {
        server().server_program().display().to_string()
    }
    fn config() -> String {
        server().config_file().display().to_string()
    }

    #[test]
    fn the_app_server_is_recognized_only_when_every_check_passes() {
        assert_eq!(
            check(
                Some(4242),
                Some(details(1000, &exe(), &config())),
                Some("4242\n")
            ),
            Recognition::Managed
        );
    }

    #[test]
    fn any_doubt_makes_it_an_existing_server() {
        let cases = [
            // Another process on the port.
            (Some(7), Some(details(1000, &exe(), &config())), Some("7")),
            // Same PID, later start: the PID was reused.
            (
                Some(4242),
                Some(details(2000, &exe(), &config())),
                Some("4242"),
            ),
            // Another program with our PID and start time.
            (
                Some(4242),
                Some(details(
                    1000,
                    &Path::new("outro").join("mysqld.exe").display().to_string(),
                    &config(),
                )),
                Some("4242"),
            ),
            // Our program, but another data folder.
            (
                Some(4242),
                Some(details(
                    1000,
                    &exe(),
                    &Path::new("outros").join("my.ini").display().to_string(),
                )),
                Some("4242"),
            ),
            // The server's own PID file disagrees, or is missing.
            (
                Some(4242),
                Some(details(1000, &exe(), &config())),
                Some("99"),
            ),
            (Some(4242), Some(details(1000, &exe(), &config())), None),
            // The process could not be read.
            (Some(4242), None, Some("4242")),
        ];
        for (listener, found, pid_file) in cases {
            assert!(
                matches!(
                    check(listener, found.clone(), pid_file),
                    Recognition::Existing { .. }
                ),
                "{listener:?} {found:?} {pid_file:?}"
            );
        }
    }

    #[test]
    fn mysql_config_listens_on_loopback_without_secrets_or_x_protocol() {
        let config = mysql_config(&server());
        assert!(config.contains("bind-address=127.0.0.1"));
        assert!(config.contains("datadir=lcp/bancos/servidores/abc/dados"));
        assert!(config.contains("mysqlx=OFF"));
        assert!(!config.to_ascii_lowercase().contains("password"));
        let mut mariadb = server();
        mariadb.engine = "mariadb".into();
        assert!(!mysql_config(&mariadb).contains("mysqlx"));
    }

    #[test]
    fn the_init_sql_protects_every_root_account() {
        let sql = mysql_init_sql("SenhaForte_123-abc");
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert!(sql.contains(&format!(
                "ALTER USER IF EXISTS 'root'@'{host}' IDENTIFIED BY 'SenhaForte_123-abc';"
            )));
        }
        assert!(sql.contains("DROP USER IF EXISTS ''@'localhost';"));
    }

    #[test]
    fn passwords_are_long_random_and_safe_in_sql() {
        let first = new_password().unwrap();
        let second = new_password().unwrap();
        assert_ne!(first, second);
        assert!(first.len() >= 43);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
        assert_eq!(base64_url(b"Man"), "TWFu");
        assert_eq!(base64_url(b"Ma"), "TWE");
    }

    #[test]
    fn only_secret_files_of_finished_owners_are_swept() {
        let root = std::env::temp_dir().join(format!("lcp-secrets-{}", std::process::id()));
        let temp = root.join(TEMP_FOLDER);
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("10-500-aaa.tmp"), "x").unwrap(); // owner alive
        fs::write(temp.join("11-600-bbb.tmp"), "x").unwrap(); // owner gone
        fs::write(temp.join("10-999-ccc.tmp"), "x").unwrap(); // PID reused
        fs::write(temp.join("lixo.tmp"), "x").unwrap(); // no owner
        sweep_abandoned_secrets_with(&root, |pid| (pid == 10).then_some(500));
        let left: Vec<_> = fs::read_dir(&temp)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["10-500-aaa.tmp"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_secret_file_is_removed_even_when_the_operation_fails() {
        let root = std::env::temp_dir().join(format!("lcp-secret-drop-{}", std::process::id()));
        let path = {
            let secret = SecretFile::create(&root, "segredo").unwrap();
            assert_eq!(fs::read_to_string(&secret.path).unwrap(), "segredo");
            secret.path.clone()
        };
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&root);
    }

    struct MemoryVault(RefCell<HashMap<String, String>>);

    impl Vault for MemoryVault {
        fn save_new(&self, target: &str, _: &str, secret: &str) -> Result<(), String> {
            let mut map = self.0.borrow_mut();
            if map.contains_key(target) {
                return Err("existe".into());
            }
            map.insert(target.into(), secret.into());
            Ok(())
        }

        fn read(&self, target: &str) -> Result<Option<String>, String> {
            Ok(self.0.borrow().get(target).cloned())
        }

        fn delete(&self, target: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(target);
            Ok(())
        }
    }

    #[test]
    fn only_a_ready_line_written_by_this_start_counts() {
        let root = std::env::temp_dir().join(format!("lcp-ready-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        // A previous start already wrote it: it does not count for the next one.
        let old = "mysqld.exe: ready for connections. Version: '9.7.4'\n";
        fs::write(&log, old).unwrap();
        assert!(!log_says_ready(&log, old.len() as u64));
        // MariaDB opens its socket before running the init file: not ready yet.
        let mut text = old.to_owned();
        text.push_str("Server socket created on IP: '127.0.0.1', port: '53377'.\n");
        fs::write(&log, &text).unwrap();
        assert!(!log_says_ready(&log, old.len() as u64));
        text.push_str("mariadbd.exe: ready for connections.\n");
        fs::write(&log, &text).unwrap();
        assert!(log_says_ready(&log, old.len() as u64));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_secret_echoed_by_the_server_never_reaches_the_log_or_the_error() {
        let root = std::env::temp_dir().join(format!("lcp-scrub-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        // What MySQL writes for a failed init statement: "near '...'" with the rest of it.
        fs::write(
            &log,
            "[ERROR] 1064 You have an error in your SQL syntax ... near 'IDENTIFIED BY 'MARCA-SECRETA-123'' at line 1\n",
        )
        .unwrap();
        scrub_file(&log, "MARCA-SECRETA-123").unwrap();
        let text = fs::read_to_string(&log).unwrap();
        assert!(!text.contains("MARCA-SECRETA-123"));
        assert!(text.contains("near 'IDENTIFIED BY '••••''"));
        let error = ServerError::StartFailed {
            detail: "falhou com MARCA-SECRETA-123".into(),
            log: text.clone(),
        }
        .scrubbed("MARCA-SECRETA-123");
        assert!(!format!("{error:?}").contains("MARCA-SECRETA-123"));
        let error =
            ServerError::Authentication("MARCA-SECRETA-123".into()).scrubbed("MARCA-SECRETA-123");
        assert_eq!(error, ServerError::Authentication("••••".into()));
        let _ = fs::remove_dir_all(&root);
    }

    /// A program that prints the password (as a server echoing a failed statement would):
    /// its output goes through the app, and the log never holds the password, not even for
    /// a moment, so nothing depends on cleaning the file afterwards.
    #[test]
    fn a_program_that_prints_the_secret_never_writes_it_to_disk() {
        let root = std::env::temp_dir().join(format!("lcp-masked-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args([
                "/C",
                "echo near 'MARCA-SECRETA-321' && echo erro MARCA-SECRETA-321 1>&2",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "echo \"near 'MARCA-SECRETA-321'\"; echo erro MARCA-SECRETA-321 1>&2",
            ]);
            command
        };
        command.current_dir(&root);
        run_to_end(command, &log, "teste", Some("MARCA-SECRETA-321")).unwrap();
        let text = fs::read_to_string(&log).unwrap();
        assert!(!text.contains("MARCA-SECRETA-321"), "{text}");
        assert!(text.contains("near '••••'"), "{text}");
        assert!(text.contains("erro ••••"), "{text}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_read_only_log_with_the_secret_is_still_cleaned() {
        let root = std::env::temp_dir().join(format!("lcp-scrub-ro-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        fs::write(&log, "near 'MARCA-SECRETA-456'\n").unwrap();
        let mut permissions = fs::metadata(&log).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&log, permissions).unwrap();
        scrub_file(&log, "MARCA-SECRETA-456").unwrap();
        assert_eq!(file_has_secret(&log, "MARCA-SECRETA-456"), Some(false));
        let _ = fs::remove_dir_all(&root);
    }

    /// Another process holds the log with no sharing: it can be neither read, changed nor
    /// removed. The failure is reported, without the secret in the message.
    #[cfg(windows)]
    #[test]
    fn a_log_that_cannot_be_cleaned_fails_without_showing_the_secret() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("lcp-scrub-lock-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        fs::write(&log, "near 'MARCA-SECRETA-789'\n").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&log)
            .unwrap();
        let error = scrub_file(&log, "MARCA-SECRETA-789").unwrap_err();
        assert!(matches!(error, ServerError::Io(_)), "{error:?}");
        assert!(!error.to_string().contains("MARCA-SECRETA-789"));
        drop(lock);
        // Once released, the next attempt cleans it.
        scrub_file(&log, "MARCA-SECRETA-789").unwrap();
        assert_eq!(file_has_secret(&log, "MARCA-SECRETA-789"), Some(false));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_plain_names_are_created() {
        for name in ["loja", "app_2", "_x", "Loja_DB"] {
            assert!(creatable_name(name), "{name}");
        }
        for name in [
            "",
            "2app",
            "loja-dev",
            "a b",
            "x`y",
            "x'y",
            "x\"y",
            "x;DROP",
            &"a".repeat(64),
        ] {
            assert!(!creatable_name(name), "{name}");
        }
    }

    #[test]
    fn project_privileges_stay_on_the_project_database() {
        let sql = mysql_project_sql("loja", "app", "s'e\\nha").join("\n");
        assert!(sql.contains("CREATE DATABASE IF NOT EXISTS `loja`"));
        assert!(
            sql.contains("CREATE USER IF NOT EXISTS 'app'@'localhost' IDENTIFIED BY 's''e\\\\nha'")
        );
        assert!(sql.contains("GRANT ALL PRIVILEGES ON `loja`.* TO 'app'@'127.0.0.1'"));
        assert!(!sql.contains("*.*"));
        assert!(!sql.to_ascii_uppercase().contains("GRANT OPTION"));
        let role = postgres_role_sql("app", "s'enha");
        assert_eq!(role, "CREATE ROLE \"app\" LOGIN PASSWORD 's''enha'");
        for forbidden in ["SUPERUSER", "CREATEDB", "CREATEROLE"] {
            assert!(!role.contains(forbidden));
        }
        assert_eq!(
            postgres_database_sql("loja", "app"),
            "CREATE DATABASE \"loja\" OWNER \"app\""
        );
    }
    #[test]
    fn only_recognized_damage_offers_a_new_data_folder() {
        assert!(log_shows_damage(
            "postgresql",
            "PANIC:  could not locate a valid checkpoint record"
        ));
        assert!(log_shows_damage(
            "postgresql",
            "FATAL:  incorrect checksum in control file"
        ));
        assert!(log_shows_damage(
            "mysql",
            "[ERROR] [MY-011906] [InnoDB] Database page corruption on disk or a failed file read"
        ));
        // A locked file, a permission or another version is not damage.
        for log in [
            "The innodb_system data file 'ibdata1' must be writable",
            "FATAL:  database files are incompatible with server",
            "could not open file \"postmaster.pid\": Permission denied",
            "Can't create/write to file",
        ] {
            assert!(!log_shows_damage("postgresql", log), "{log}");
            assert!(!log_shows_damage("mysql", log), "{log}");
        }
    }
    #[test]
    fn a_log_with_text_in_the_windows_code_page_is_still_read() {
        let root = std::env::temp_dir().join(format!("lcp-log-cp1252-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("servidor.log");
        // "servidor está parado" from pg_ctl in cp1252, then the server's own line.
        let mut bytes = b"servidor est\xe1 parado\n".to_vec();
        bytes.extend_from_slice(b"FATAL:  incorrect checksum in control file\n");
        fs::write(&log, bytes).unwrap();
        let tail = log_tail(&log);
        assert!(
            tail.contains("incorrect checksum in control file"),
            "{tail}"
        );
        assert!(log_shows_damage("postgresql", &tail));
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn a_missing_credential_changes_nothing() {
        let vault = MemoryVault(RefCell::default());
        assert_eq!(
            server().admin_password(&vault),
            Err(ServerError::CredentialMissing)
        );
        vault
            .save_new(&credential_target("abc"), "root", "x")
            .unwrap();
        assert_eq!(server().admin_password(&vault).as_deref(), Ok("x"));
    }
}
