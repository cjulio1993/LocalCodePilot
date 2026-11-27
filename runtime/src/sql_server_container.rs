//! Database servers in a container the app manages (Docker or Podman):
//! - SQL Server, for projects that use it on this computer. There is no portable SQL Server:
//!   the official Linux image runs in a container, and only after the person accepts
//!   Microsoft's license terms (T019.06);
//! - PostgreSQL on macOS and Linux, where no official package can be verified: the Docker
//!   Official Image, pinned by digest (T030). Windows keeps the native package.
//!
//! Both kinds share everything below; only the image, the port inside the container, the
//! data folder, the initial settings and the administrator login differ (`ContainerKind`).
//!
//! Agreed with product in `.internal/conversas/2026-11-20-t019-06-sql-server.md`:
//! - pinned image (`2022-CU28-ubuntu-22.04` by digest), Developer edition, terms link of that
//!   edition and version; the three change together;
//! - the container gets an initial `sa` password through a temporary `--env-file` (never the
//!   command line); the engine keeps it in the container's settings, so it is replaced right
//!   away by `ALTER LOGIN` with the final one, kept in the Windows Credential Manager; a
//!   failure stops the container and keeps its volume;
//! - recognized by name, label and image; stopping is confirmed by `inspect`;
//! - the project's login gets `db_owner` on its own database only, never `sysadmin`.

use crate::database_servers::Vault;
use localcodepilot_platform::installations::{configure_background_command, find_executable};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// The pinned image: version, digest and license terms go together.
pub const IMAGE: &str = "mcr.microsoft.com/mssql/server:2022-CU28-ubuntu-22.04@sha256:4525e3efac9f3e241cb003bb2ff2a82818b502f37e12049d788b1a9fa7e13b7f";
pub const VERSION_LABEL: &str = "SQL Server 2022 (CU28), edição Developer";
pub const TERMS_URL: &str = "https://www.microsoft.com/en-us/Useterms/Retail/SQLServer2022/SQLServer2022DeveloperExpressEvaluation/Useterms_Retail_SQLServer2022_SQLServer2022DeveloperExpressEvaluation_English.htm";
/// The image download, for the confirmation.
pub const APPROXIMATE_DOWNLOAD: &str = "cerca de 500 MB";
/// The image once extracted by the engine (checked by `examples/sql_server_check.rs`).
pub const APPROXIMATE_SPACE: &str = "cerca de 1,6 GB";

/// PostgreSQL's Docker Official Image, pinned by the digest of its multi-platform index
/// (linux/amd64 and linux/arm64), read from registry-1.docker.io on 2026-11-27 (T030).
pub const POSTGRES_IMAGE: &str = "docker.io/library/postgres:19.0-trixie@sha256:4ea94de574c17f0372722806c949757050fbb1cd7648410fe45d6bba275cc00e";
pub const POSTGRES_VERSION_LABEL: &str = "PostgreSQL 19.0";
/// The compressed layers of the image for amd64 (162 MB) and arm64 (161 MB).
pub const POSTGRES_APPROXIMATE_DOWNLOAD: &str = "cerca de 160 MB";
/// The administrator role created in the container; the project gets its own role.
pub const POSTGRES_ADMIN: &str = "localcodepilot_admin";
pub const POSTGRES_PREPARATION_FAILED: &str =
    "Não foi possível concluir a preparação do PostgreSQL. Seus dados foram mantidos.";
pub const POSTGRES_STOP_NOT_CONFIRMED: &str = "Não foi possível confirmar que o PostgreSQL parou. Verifique o container na página Containers.";

/// What runs in the container. Registries written before T030 have no kind: SQL Server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContainerKind {
    #[default]
    #[serde(rename = "sqlserver")]
    SqlServer,
    #[serde(rename = "postgresql")]
    PostgreSql,
}

impl ContainerKind {
    pub fn image(self) -> &'static str {
        match self {
            Self::SqlServer => IMAGE,
            Self::PostgreSql => POSTGRES_IMAGE,
        }
    }

    pub fn version_label(self) -> &'static str {
        match self {
            Self::SqlServer => VERSION_LABEL,
            Self::PostgreSql => POSTGRES_VERSION_LABEL,
        }
    }

    pub fn preparation_failed(self) -> &'static str {
        match self {
            Self::SqlServer => PREPARATION_FAILED,
            Self::PostgreSql => POSTGRES_PREPARATION_FAILED,
        }
    }

    pub fn stop_not_confirmed(self) -> &'static str {
        match self {
            Self::SqlServer => STOP_NOT_CONFIRMED,
            Self::PostgreSql => POSTGRES_STOP_NOT_CONFIRMED,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::SqlServer => "SQL Server",
            Self::PostgreSql => "PostgreSQL",
        }
    }

    /// The port the server listens on inside the container.
    fn inner_port(self) -> u16 {
        match self {
            Self::SqlServer => 1433,
            Self::PostgreSql => 5432,
        }
    }

    /// Where the volume is mounted: the image's data folder (`/var/lib/postgresql` since the
    /// PostgreSQL 18 images).
    fn data_path(self) -> &'static str {
        match self {
            Self::SqlServer => "/var/opt/mssql",
            Self::PostgreSql => "/var/lib/postgresql",
        }
    }

    /// Part of the container, volume and credential names.
    fn prefix(self) -> &'static str {
        match self {
            Self::SqlServer => "mssql",
            Self::PostgreSql => "pg",
        }
    }

    fn credential_folder(self) -> &'static str {
        match self {
            Self::SqlServer => "sqlserver",
            Self::PostgreSql => "postgresql",
        }
    }

    fn admin_user(self) -> &'static str {
        match self {
            Self::SqlServer => "sa",
            Self::PostgreSql => POSTGRES_ADMIN,
        }
    }

    /// The settings the container starts with, through a temporary `--env-file`.
    fn initial_settings(self, password: &str) -> String {
        match self {
            Self::SqlServer => {
                format!("ACCEPT_EULA=Y\nMSSQL_PID=Developer\nMSSQL_SA_PASSWORD={password}\n")
            }
            Self::PostgreSql => format!(
                "POSTGRES_USER={POSTGRES_ADMIN}\nPOSTGRES_PASSWORD={password}\nPOSTGRES_DB=postgres\n"
            ),
        }
    }

    /// The statement that replaces the initial administrator password with the final one.
    fn password_change(self, password: &str) -> String {
        match self {
            Self::SqlServer => format!(
                "ALTER LOGIN sa WITH PASSWORD = '{}'",
                password.replace('\'', "''")
            ),
            Self::PostgreSql => format!(
                "ALTER ROLE {POSTGRES_ADMIN} PASSWORD '{}'",
                password.replace('\'', "''")
            ),
        }
    }
}
const LABEL: &str = "localcodepilot.servidor";
const REGISTRY_FILE: &str = "sqlserver.json";
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const ADMIN_PASSWORD_MISSING: &str = "A senha de administração deste banco não está disponível. O LocalCodePilot não fará alterações nele.";
const NO_WORKING_PASSWORD: &str =
    "Nenhuma senha de administração guardada pelo LocalCodePilot entra neste SQL Server.";
const CONTAINER_MISSING: &str = "O container do SQL Server deste projeto não existe mais.";
const POSTGRES_NO_WORKING_PASSWORD: &str =
    "Nenhuma senha de administração guardada pelo LocalCodePilot entra neste PostgreSQL.";
const POSTGRES_CONTAINER_MISSING: &str = "O container do PostgreSQL deste projeto não existe mais.";
/// The first sentence of a preparation that failed after the container was created.
pub const PREPARATION_FAILED: &str =
    "Não foi possível concluir a preparação do SQL Server. Seus dados foram mantidos.";
/// The stop could not be confirmed by `inspect`.
pub const STOP_NOT_CONFIRMED: &str = "Não foi possível confirmar que o SQL Server parou. Verifique o container na página Containers.";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60 * 30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlServerContainer {
    pub id: String,
    #[serde(default)]
    pub kind: ContainerKind,
    /// `docker` or `podman`.
    pub engine: String,
    pub port: u16,
    #[serde(default)]
    pub stop_failed: bool,
}

impl SqlServerContainer {
    pub fn container(&self) -> String {
        format!("localcodepilot-{}-{}", self.kind.prefix(), self.id)
    }

    pub fn volume(&self) -> String {
        format!("localcodepilot-{}-{}", self.kind.prefix(), self.id)
    }

    fn final_target(&self) -> String {
        format!(
            "LocalCodePilot/{}/{}",
            self.kind.credential_folder(),
            self.id
        )
    }

    fn initial_target(&self) -> String {
        format!(
            "LocalCodePilot/{}/{}-inicial",
            self.kind.credential_folder(),
            self.id
        )
    }
}

pub fn load_registry(root: &Path) -> Vec<SqlServerContainer> {
    fs::read_to_string(root.join(REGISTRY_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn find(root: &Path, port: u16) -> Option<SqlServerContainer> {
    load_registry(root)
        .into_iter()
        .find(|server| server.port == port)
}

pub fn save(root: &Path, server: &SqlServerContainer) -> Result<(), String> {
    let mut servers = load_registry(root);
    servers.retain(|known| known.id != server.id);
    servers.push(server.clone());
    write_registry(root, &servers)
}

fn write_registry(root: &Path, servers: &[SqlServerContainer]) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(&servers).map_err(|error| error.to_string())?;
    let partial = root.join(format!("{REGISTRY_FILE}.part"));
    fs::write(&partial, text).map_err(|error| error.to_string())?;
    fs::rename(&partial, root.join(REGISTRY_FILE)).map_err(|error| error.to_string())
}

/// A password SQL Server's policy accepts (upper, lower, digit, symbol), with no quote.
fn new_password() -> Result<String, String> {
    let mut bytes = [0_u8; 24];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "não foi possível gerar uma senha aleatória".to_owned())?;
    let body: String = bytes
        .iter()
        .map(|byte| {
            const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
            ALPHABET[*byte as usize % ALPHABET.len()] as char
        })
        .collect();
    Ok(format!("{body}Aa9!"))
}

fn new_id() -> Result<String, String> {
    let mut bytes = [0_u8; 6];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "não foi possível gerar um identificador".to_owned())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Runs the engine's command line, hidden, and returns its output.
fn engine_command(
    engine: &str,
    args: &[&str],
    path: Option<&OsStr>,
    timeout: Duration,
) -> Result<String, String> {
    let program = find_executable(engine, path)
        .ok_or_else(|| format!("{engine} não foi encontrado neste computador"))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("não foi possível executar {engine}: {error}"))?;
    let read = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stream) = stream {
                let mut bytes = vec![];
                let _ = stream.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let stdout = read(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = read(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(200))
            }
            _ => {
                let _ = child.kill();
                return Err(format!("{engine} não respondeu a tempo"));
            }
        }
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{engine} {}: {}",
            args.first().unwrap_or(&""),
            err.trim()
        ))
    }
}

/// What `inspect` says about the app's container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    Stopped,
    /// No container with that name.
    Missing,
    /// A container with that name that is not the app's (label or image differ).
    NotOurs(String),
}

pub fn state(server: &SqlServerContainer, path: Option<&OsStr>) -> Result<ContainerState, String> {
    let format = format!(
        "{{{{.State.Running}}}}|{{{{index .Config.Labels \"{LABEL}\"}}}}|{{{{.Config.Image}}}}"
    );
    match engine_command(
        &server.engine,
        &[
            "inspect",
            "--type",
            "container",
            "--format",
            &format,
            &server.container(),
        ],
        path,
        Duration::from_secs(30),
    ) {
        Ok(text) => Ok(parse_state(text.trim(), &server.id, server.kind)),
        Err(error) if error.to_ascii_lowercase().contains("no such") => Ok(ContainerState::Missing),
        Err(error) => Err(error),
    }
}

fn parse_state(text: &str, id: &str, kind: ContainerKind) -> ContainerState {
    let mut parts = text.split('|');
    let running = parts.next().unwrap_or_default();
    let label = parts.next().unwrap_or_default();
    let image = parts.next().unwrap_or_default();
    if label != id {
        return ContainerState::NotOurs("o rótulo do container não é o do LocalCodePilot".into());
    }
    let image = image.split('@').next().unwrap_or_default();
    if image.is_empty() || !kind.image().starts_with(image) {
        return ContainerState::NotOurs("a imagem do container não é a fixada".into());
    }
    if running == "true" {
        ContainerState::Running
    } else {
        ContainerState::Stopped
    }
}

/// The administrator password that logs in now: the final one, or the initial one when the
/// change did not finish.
fn sa_password(server: &SqlServerContainer, vault: &dyn Vault) -> Result<Vec<String>, String> {
    let mut found = vec![];
    for target in [server.final_target(), server.initial_target()] {
        if let Some(password) = vault.read(&target)? {
            found.push(password);
        }
    }
    if found.is_empty() {
        return Err(ADMIN_PASSWORD_MISSING.into());
    }
    Ok(found)
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
}

/// Runs statements as the administrator of the app's container: `sa` through TDS, or the
/// app's PostgreSQL role. Loopback only.
fn as_admin(
    server: &SqlServerContainer,
    password: &str,
    statements: &[String],
) -> Result<(), String> {
    match server.kind {
        ContainerKind::SqlServer => as_sa(server, password, statements),
        ContainerKind::PostgreSql => as_postgres_admin(server, password, statements),
    }
}

fn as_postgres_admin(
    server: &SqlServerContainer,
    password: &str,
    statements: &[String],
) -> Result<(), String> {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(server.port)
        .user(POSTGRES_ADMIN)
        .password(password)
        .dbname("postgres")
        .connect_timeout(Duration::from_secs(10));
    runtime()?
        .block_on(async {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|error| error.to_string())?;
            let driver = tokio::spawn(connection);
            let mut result = Ok(());
            for statement in statements {
                if let Err(error) = client.batch_execute(statement).await {
                    result = Err(error.to_string());
                    break;
                }
            }
            drop(client);
            let _ = driver.await;
            result
        })
        .map_err(|error: String| error.replace(password, "••••"))
}

/// Runs T-SQL as `sa` on the app's container (loopback, its own certificate accepted).
fn as_sa(server: &SqlServerContainer, password: &str, statements: &[String]) -> Result<(), String> {
    use tokio_util::compat::TokioAsyncWriteCompatExt;
    runtime()?
        .block_on(async {
            let mut config = tiberius::Config::new();
            config.host("127.0.0.1");
            config.port(server.port);
            config.authentication(tiberius::AuthMethod::sql_server("sa", password));
            config.encryption(tiberius::EncryptionLevel::Required);
            config.trust_cert();
            let tcp = tokio::net::TcpStream::connect(config.get_addr())
                .await
                .map_err(|error| error.to_string())?;
            let mut client = tiberius::Client::connect(config, tcp.compat_write())
                .await
                .map_err(|error| error.to_string())?;
            for statement in statements {
                client
                    .execute(statement.as_str(), &[])
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        })
        .map_err(|error: String| error.replace(password, "••••"))
}

/// Waits until the administrator logs in with `password`.
fn wait_ready(server: &SqlServerContainer, password: &str) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match as_admin(server, password, &[]) {
            Ok(()) => return Ok(()),
            Err(_) if started.elapsed() < READY_TIMEOUT => {
                std::thread::sleep(Duration::from_secs(2))
            }
            Err(error) => {
                return Err(format!(
                    "o {} não ficou pronto: {error}",
                    server.kind.name()
                ));
            }
        }
    }
}

/// Stops the container and confirms it by `inspect`.
pub fn stop(server: &SqlServerContainer, path: Option<&OsStr>) -> Result<(), String> {
    match state(server, path)? {
        ContainerState::Stopped | ContainerState::Missing => return Ok(()),
        ContainerState::NotOurs(reason) => {
            return Err(format!(
                "o LocalCodePilot não para este container: {reason}"
            ));
        }
        ContainerState::Running => {}
    }
    let _ = engine_command(
        &server.engine,
        &["stop", &server.container()],
        path,
        Duration::from_secs(90),
    );
    match state(server, path)? {
        ContainerState::Stopped | ContainerState::Missing => Ok(()),
        _ => Err(server.kind.stop_not_confirmed().into()),
    }
}

/// Starts the app's container (created before) and waits for `sa`. Finishes a password change
/// that a failure interrupted.
pub fn start(
    server: &SqlServerContainer,
    vault: &dyn Vault,
    path: Option<&OsStr>,
) -> Result<(), String> {
    match state(server, path)? {
        ContainerState::Running => {}
        ContainerState::Stopped => {
            engine_command(
                &server.engine,
                &["start", &server.container()],
                path,
                Duration::from_secs(120),
            )?;
        }
        ContainerState::Missing => {
            return Err(match server.kind {
                ContainerKind::SqlServer => CONTAINER_MISSING,
                ContainerKind::PostgreSql => POSTGRES_CONTAINER_MISSING,
            }
            .into());
        }
        ContainerState::NotOurs(reason) => return Err(reason),
    }
    finish_password_change(server, vault)
}

/// After the container answers: the final `sa` password is in force and the initial one is
/// gone. A failure stops the container (confirmed) and keeps the volume.
fn finish_password_change(server: &SqlServerContainer, vault: &dyn Vault) -> Result<(), String> {
    let candidates = sa_password(server, vault)?;
    let mut working = None;
    let started = Instant::now();
    while working.is_none() && started.elapsed() < READY_TIMEOUT {
        working = candidates
            .iter()
            .find(|password| as_admin(server, password, &[]).is_ok())
            .cloned();
        if working.is_none() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    let Some(working) = working else {
        return Err(match server.kind {
            ContainerKind::SqlServer => NO_WORKING_PASSWORD,
            ContainerKind::PostgreSql => POSTGRES_NO_WORKING_PASSWORD,
        }
        .into());
    };
    if let Some(initial) = vault.read(&server.initial_target())?
        && initial == working
    {
        let final_password = match vault.read(&server.final_target())? {
            Some(password) => password,
            None => {
                let password = new_password()?;
                vault.save_new(&server.final_target(), server.kind.admin_user(), &password)?;
                password
            }
        };
        as_admin(
            server,
            &working,
            &[server.kind.password_change(&final_password)],
        )?;
        as_admin(server, &final_password, &[])?;
        vault.delete(&server.initial_target())?;
    }
    Ok(())
}

/// The steps of a preparation after the image download, for the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerStep {
    /// The downloaded image is checked against the pinned digest.
    Verifying,
    /// The container is created and the administrator password replaced.
    Creating,
}

/// Whether the engine's copy of `image` (`image inspect` → RepoDigests) is the pinned one.
fn image_matches(repo_digests: &str, image: &str) -> bool {
    let Some(digest) = image.split('@').nth(1) else {
        return false;
    };
    repo_digests.contains(digest)
}

/// Creates the app's container of `kind` on `port` and leaves it running with the final
/// administrator password. For SQL Server, the person accepted the license terms before this
/// is called.
pub fn prepare(
    root: &Path,
    kind: ContainerKind,
    engine: &str,
    port: u16,
    vault: &dyn Vault,
    path: Option<&OsStr>,
    step: impl Fn(ContainerStep),
) -> Result<SqlServerContainer, String> {
    let server = SqlServerContainer {
        id: new_id()?,
        kind,
        engine: engine.to_owned(),
        port,
        stop_failed: false,
    };
    engine_command(engine, &["pull", kind.image()], path, COMMAND_TIMEOUT)?;
    // The engine pulls by digest; the app still confirms the copy it will run is that one.
    step(ContainerStep::Verifying);
    let digests = engine_command(
        engine,
        &[
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            kind.image(),
        ],
        path,
        Duration::from_secs(60),
    )?;
    if !image_matches(&digests, kind.image()) {
        return Err(format!(
            "a imagem baixada do {} não confere com o digest fixado; nada foi criado",
            kind.name()
        ));
    }
    step(ContainerStep::Creating);
    let initial = new_password()?;
    vault.save_new(&server.initial_target(), kind.admin_user(), &initial)?;
    save(root, &server)?;
    // The password reaches the container through a file that exists only for `run`, which
    // only this user can read (0600 in a 0700 folder on macOS and Linux).
    let env_file = write_private_temp(
        root,
        &format!("{}-{}.env", kind.prefix(), server.id),
        &kind.initial_settings(&initial),
    )?;
    let published = format!("127.0.0.1:{port}:{}", kind.inner_port());
    let mount = format!("{}:{}", server.volume(), kind.data_path());
    let label = format!("{LABEL}={}", server.id);
    let env_arg = env_file.display().to_string();
    let created = engine_command(
        engine,
        &[
            "run",
            "-d",
            "--name",
            &server.container(),
            "--label",
            &label,
            "-p",
            &published,
            "-v",
            &mount,
            "--env-file",
            &env_arg,
            kind.image(),
        ],
        path,
        Duration::from_secs(120),
    );
    let removed = fs::remove_file(&env_file);
    if let Err(error) = created {
        // No container was created: nothing of it is kept.
        if state(&server, path) == Ok(ContainerState::Missing) {
            let _ = release(root, &server.id);
            let _ = vault.delete(&server.initial_target());
        }
        return Err(error);
    }
    if removed.is_err() {
        let _ = stop(&server, path);
        return Err(
            "o arquivo temporário com a senha não pôde ser apagado; o container foi parado".into(),
        );
    }
    if let Err(error) =
        wait_ready(&server, &initial).and_then(|()| finish_password_change(&server, vault))
    {
        let stopped = stop(&server, path);
        return Err(match stopped {
            Ok(()) => format!("{} ({error})", kind.preparation_failed()),
            Err(stop_error) => format!("{stop_error} ({error})"),
        });
    }
    Ok(server)
}

/// Writes `contents` to a new file of `<root>/.temp` that only this user can read.
fn write_private_temp(root: &Path, name: &str, contents: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    let temp = root.join(".temp");
    fs::create_dir_all(&temp).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let path = temp.join(name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|error| error.to_string())?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// Names the app creates: letters, digits and `_`, up to 63, not starting with a digit.
fn creatable(name: &str) -> bool {
    crate::database_servers::creatable_name(name)
}

/// The project's database, login and user, only what is missing; `db_owner` of its own
/// database only. The project's login is tested afterwards by the caller.
fn project_sql(database: &str, user: &str, password: &str) -> Vec<String> {
    let password = password.replace('\'', "''");
    vec![
        format!("IF DB_ID(N'{database}') IS NULL CREATE DATABASE [{database}]"),
        format!(
            "IF SUSER_ID(N'{user}') IS NULL CREATE LOGIN [{user}] WITH PASSWORD = N'{password}', CHECK_POLICY = OFF, DEFAULT_DATABASE = [{database}]"
        ),
        format!(
            "USE [{database}]; IF USER_ID(N'{user}') IS NULL CREATE USER [{user}] FOR LOGIN [{user}]; ALTER ROLE db_owner ADD MEMBER [{user}]"
        ),
    ]
}

pub fn ensure_project_database(
    server: &SqlServerContainer,
    vault: &dyn Vault,
    database: &str,
    user: &str,
    password: &str,
) -> Result<(), String> {
    if !creatable(database) || !creatable(user) {
        return Err("O nome do banco ou do usuário na configuração deste projeto não pode ser criado automaticamente. Confira os nomes em Detalhes.".into());
    }
    let admin = vault
        .read(&server.final_target())?
        .ok_or(ADMIN_PASSWORD_MISSING)?;
    match server.kind {
        ContainerKind::SqlServer => as_sa(server, &admin, &project_sql(database, user, password)),
        ContainerKind::PostgreSql => postgres_project(server, &admin, database, user, password),
    }
    .map_err(|error| error.replace(password, "••••"))
}

/// PostgreSQL: a login role without SUPERUSER, CREATEDB or CREATEROLE, owner of the project's
/// database, as on the native server; only what is missing is created.
fn postgres_project(
    server: &SqlServerContainer,
    admin: &str,
    database: &str,
    user: &str,
    password: &str,
) -> Result<(), String> {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(server.port)
        .user(POSTGRES_ADMIN)
        .password(admin)
        .dbname("postgres")
        .connect_timeout(Duration::from_secs(10));
    runtime()?.block_on(async {
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(|error| error.to_string())?;
        let driver = tokio::spawn(connection);
        let result = async {
            let role = client
                .query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&user])
                .await
                .map_err(|error| error.to_string())?;
            if role.is_none() {
                client
                    .batch_execute(&postgres_role_sql(user, password))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            let exists = client
                .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&database])
                .await
                .map_err(|error| error.to_string())?;
            if exists.is_none() {
                client
                    .batch_execute(&format!("CREATE DATABASE \"{database}\" OWNER \"{user}\""))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        .await;
        drop(client);
        let _ = driver.await;
        result
    })
}

fn postgres_role_sql(user: &str, password: &str) -> String {
    format!(
        "CREATE ROLE \"{user}\" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '{}'",
        password.replace('\'', "''")
    )
}

/// "Corrigir conexão": the project's login takes the password configured in the project.
pub fn set_project_password(
    server: &SqlServerContainer,
    vault: &dyn Vault,
    user: &str,
    password: &str,
) -> Result<(), String> {
    if !creatable(user) {
        return Err("O nome do usuário na configuração deste projeto não pode ser alterado automaticamente.".into());
    }
    let admin = vault
        .read(&server.final_target())?
        .ok_or(ADMIN_PASSWORD_MISSING)?;
    let statement = match server.kind {
        ContainerKind::SqlServer => format!(
            "ALTER LOGIN [{user}] WITH PASSWORD = N'{}'",
            password.replace('\'', "''")
        ),
        ContainerKind::PostgreSql => format!(
            "ALTER ROLE \"{user}\" PASSWORD '{}'",
            password.replace('\'', "''")
        ),
    };
    as_admin(server, &admin, &[statement]).map_err(|error| error.replace(password, "••••"))
}

/// "Criar SQL Server novo": the app lets go of this container. The container and its volume
/// stay on the engine, with their names, and the data in them is kept.
pub fn release(root: &Path, id: &str) -> Result<(), String> {
    let mut servers = load_registry(root);
    servers.retain(|known| known.id != id);
    write_registry(root, &servers)
}

/// A close that could not confirm the stop is remembered until the next start of the app.
pub fn mark_stop_failed(
    root: &Path,
    server: &SqlServerContainer,
    failed: bool,
) -> Result<(), String> {
    let mut server = server.clone();
    server.stop_failed = failed;
    save(root, &server)
}

/// The start of a container failed because no administrator password the app keeps logs in:
/// trying again does not help; a new container (with a new volume) does.
pub fn needs_new_container(error: &str) -> bool {
    [
        NO_WORKING_PASSWORD,
        ADMIN_PASSWORD_MISSING,
        CONTAINER_MISSING,
        POSTGRES_NO_WORKING_PASSWORD,
        POSTGRES_CONTAINER_MISSING,
    ]
    .iter()
    .any(|known| error.contains(known))
}

/// Removes the test credentials of a container (tests and E2E only).
pub fn forget_credentials(server: &SqlServerContainer, vault: &dyn Vault) {
    let _ = vault.delete(&server.final_target());
    let _ = vault.delete(&server.initial_target());
}

pub fn root_registry_path(root: &Path) -> PathBuf {
    root.join(REGISTRY_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_pinned_with_its_edition_and_terms() {
        assert!(IMAGE.contains("2022-CU28"));
        assert!(IMAGE.contains("@sha256:"));
        assert!(VERSION_LABEL.contains("2022") && VERSION_LABEL.contains("Developer"));
        assert!(TERMS_URL.contains("SQLServer2022") && TERMS_URL.contains("Developer"));
    }

    #[test]
    fn generated_passwords_meet_the_sql_server_policy() {
        let password = new_password().unwrap();
        assert!(password.len() >= 20);
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert!(!password.contains('\''));
    }

    #[test]
    fn only_the_apps_container_is_recognized() {
        let sql = ContainerKind::SqlServer;
        let image = IMAGE.split('@').next().unwrap();
        assert_eq!(
            parse_state(&format!("true|abc|{image}"), "abc", sql),
            ContainerState::Running
        );
        assert_eq!(
            parse_state(&format!("false|abc|{image}"), "abc", sql),
            ContainerState::Stopped
        );
        assert!(matches!(
            parse_state(&format!("true|outro|{image}"), "abc", sql),
            ContainerState::NotOurs(_)
        ));
        assert!(matches!(
            parse_state(
                "true|abc|mcr.microsoft.com/mssql/server:2019-latest",
                "abc",
                sql
            ),
            ContainerState::NotOurs(_)
        ));
        // PostgreSQL: its own image only, never SQL Server's or another tag.
        let pg = ContainerKind::PostgreSql;
        let pg_image = POSTGRES_IMAGE.split('@').next().unwrap();
        assert_eq!(
            parse_state(&format!("true|abc|{pg_image}"), "abc", pg),
            ContainerState::Running
        );
        assert_eq!(
            parse_state(&format!("true|abc|{POSTGRES_IMAGE}"), "abc", pg),
            ContainerState::Running
        );
        for other in [image, "docker.io/library/postgres:16", ""] {
            assert!(matches!(
                parse_state(&format!("true|abc|{other}"), "abc", pg),
                ContainerState::NotOurs(_)
            ));
        }
    }

    #[test]
    fn only_the_pinned_digest_is_accepted_after_the_download() {
        let digest = POSTGRES_IMAGE.split('@').nth(1).unwrap();
        assert!(image_matches(
            &format!("[\"postgres@{digest}\",\"docker.io/library/postgres@{digest}\"]"),
            POSTGRES_IMAGE
        ));
        assert!(!image_matches(
            "[\"postgres@sha256:0000000000000000000000000000000000000000000000000000000000000000\"]",
            POSTGRES_IMAGE
        ));
        assert!(!image_matches("[]", POSTGRES_IMAGE));
        assert!(!image_matches("[\"x\"]", "postgres:19"));
    }

    #[test]
    fn postgres_is_the_official_image_pinned_by_digest() {
        assert!(POSTGRES_IMAGE.starts_with("docker.io/library/postgres:19.0-"));
        let digest = POSTGRES_IMAGE.split("@sha256:").nth(1).unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
        let kind = ContainerKind::PostgreSql;
        assert_eq!(kind.image(), POSTGRES_IMAGE);
        assert_eq!(kind.inner_port(), 5432);
        let settings = kind.initial_settings("s3nha");
        assert!(settings.contains("POSTGRES_USER=localcodepilot_admin\n"));
        assert!(settings.contains("POSTGRES_PASSWORD=s3nha\n"));
        assert!(!settings.contains("EULA"));
        assert_eq!(
            kind.password_change("a'b"),
            "ALTER ROLE localcodepilot_admin PASSWORD 'a''b'"
        );
    }

    #[test]
    fn each_kind_has_its_own_names_and_old_registries_stay_sql_server() {
        let server = |kind| SqlServerContainer {
            id: "abc".into(),
            kind,
            engine: "docker".into(),
            port: 1,
            stop_failed: false,
        };
        let sql = server(ContainerKind::SqlServer);
        assert_eq!(sql.container(), "localcodepilot-mssql-abc");
        assert_eq!(sql.final_target(), "LocalCodePilot/sqlserver/abc");
        let pg = server(ContainerKind::PostgreSql);
        assert_eq!(pg.container(), "localcodepilot-pg-abc");
        assert_eq!(pg.volume(), "localcodepilot-pg-abc");
        assert_eq!(pg.final_target(), "LocalCodePilot/postgresql/abc");
        assert_eq!(pg.initial_target(), "LocalCodePilot/postgresql/abc-inicial");
        let old: Vec<SqlServerContainer> =
            serde_json::from_str(r#"[{"id":"x","engine":"podman","port":14333}]"#).unwrap();
        assert_eq!(old[0].kind, ContainerKind::SqlServer);
        let saved = serde_json::to_string(&pg).unwrap();
        assert!(saved.contains("\"kind\":\"postgresql\""));
    }

    #[test]
    fn the_postgres_project_role_has_no_server_wide_rights() {
        let sql = postgres_role_sql("app", "s'enha");
        assert_eq!(
            sql,
            "CREATE ROLE \"app\" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 's''enha'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_initial_settings_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("lcp-pg-env-{}", std::process::id()));
        let path = write_private_temp(&root, "pg-x.env", "POSTGRES_PASSWORD=x\n").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&root.join(".temp")), 0o700);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_project_login_owns_only_its_database() {
        let sql = project_sql("loja", "app", "s'enha").join("\n");
        assert!(sql.contains("CREATE DATABASE [loja]"));
        assert!(sql.contains("CREATE LOGIN [app] WITH PASSWORD = N's''enha'"));
        assert!(sql.contains("ALTER ROLE db_owner ADD MEMBER [app]"));
        assert!(!sql.to_ascii_lowercase().contains("sysadmin"));
        assert!(!sql.contains("sp_addsrvrolemember"));
    }
}
