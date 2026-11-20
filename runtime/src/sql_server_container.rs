//! SQL Server in a container the app manages (Docker or Podman), for projects that use SQL
//! Server on this computer. There is no portable SQL Server: the official Linux image runs in
//! a container, and only after the person accepts Microsoft's license terms.
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
pub const IMAGE: &str = "mcr.microsoft.com/mssql/server:2022-CU28-ubuntu-22.04@sha256:e10cd49443c94dbdcb7a050987db22ff4d8c87052ac53768abe1ed848d3fcef6";
pub const VERSION_LABEL: &str = "SQL Server 2022 (CU28), edição Developer";
pub const TERMS_URL: &str = "https://www.microsoft.com/en-us/Useterms/Retail/SQLServer2022/SQLServer2022DeveloperExpressEvaluation/Useterms_Retail_SQLServer2022_SQLServer2022DeveloperExpressEvaluation_English.htm";
/// The image download, for the confirmation.
pub const APPROXIMATE_DOWNLOAD: &str = "cerca de 1,5 GB";
const LABEL: &str = "localcodepilot.servidor";
const REGISTRY_FILE: &str = "sqlserver.json";
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const ADMIN_PASSWORD_MISSING: &str = "A senha de administração deste banco não está disponível. O LocalCodePilot não fará alterações nele.";
const NO_WORKING_PASSWORD: &str =
    "Nenhuma senha de administração guardada pelo LocalCodePilot entra neste SQL Server.";
const CONTAINER_MISSING: &str = "O container do SQL Server deste projeto não existe mais.";
/// The first sentence of a preparation that failed after the container was created.
pub const PREPARATION_FAILED: &str =
    "Não foi possível concluir a preparação do SQL Server. Seus dados foram mantidos.";
/// The stop could not be confirmed by `inspect`.
pub const STOP_NOT_CONFIRMED: &str = "Não foi possível confirmar que o SQL Server parou. Verifique o container na página Containers.";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60 * 30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlServerContainer {
    pub id: String,
    /// `docker` or `podman`.
    pub engine: String,
    pub port: u16,
    #[serde(default)]
    pub stop_failed: bool,
}

impl SqlServerContainer {
    pub fn container(&self) -> String {
        format!("localcodepilot-mssql-{}", self.id)
    }

    pub fn volume(&self) -> String {
        format!("localcodepilot-mssql-{}", self.id)
    }

    fn final_target(&self) -> String {
        format!("LocalCodePilot/sqlserver/{}", self.id)
    }

    fn initial_target(&self) -> String {
        format!("LocalCodePilot/sqlserver/{}-inicial", self.id)
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
        Ok(text) => Ok(parse_state(text.trim(), &server.id)),
        Err(error) if error.to_ascii_lowercase().contains("no such") => Ok(ContainerState::Missing),
        Err(error) => Err(error),
    }
}

fn parse_state(text: &str, id: &str) -> ContainerState {
    let mut parts = text.split('|');
    let running = parts.next().unwrap_or_default();
    let label = parts.next().unwrap_or_default();
    let image = parts.next().unwrap_or_default();
    if label != id {
        return ContainerState::NotOurs("o rótulo do container não é o do LocalCodePilot".into());
    }
    if !IMAGE.starts_with(image.split('@').next().unwrap_or_default()) {
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

/// Waits until `sa` logs in with `password`.
fn wait_ready(server: &SqlServerContainer, password: &str) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match as_sa(server, password, &[]) {
            Ok(()) => return Ok(()),
            Err(_) if started.elapsed() < READY_TIMEOUT => {
                std::thread::sleep(Duration::from_secs(2))
            }
            Err(error) => return Err(format!("o SQL Server não ficou pronto: {error}")),
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
        _ => Err(STOP_NOT_CONFIRMED.into()),
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
        ContainerState::Missing => return Err(CONTAINER_MISSING.into()),
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
            .find(|password| as_sa(server, password, &[]).is_ok())
            .cloned();
        if working.is_none() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    let Some(working) = working else {
        return Err(NO_WORKING_PASSWORD.into());
    };
    if let Some(initial) = vault.read(&server.initial_target())?
        && initial == working
    {
        let final_password = match vault.read(&server.final_target())? {
            Some(password) => password,
            None => {
                let password = new_password()?;
                vault.save_new(&server.final_target(), "sa", &password)?;
                password
            }
        };
        as_sa(
            server,
            &working,
            &[format!(
                "ALTER LOGIN sa WITH PASSWORD = '{}'",
                final_password.replace('\'', "''")
            )],
        )?;
        as_sa(server, &final_password, &[])?;
        vault.delete(&server.initial_target())?;
    }
    Ok(())
}

/// Creates the app's SQL Server container on `port` and leaves it running with the final `sa`
/// password. The person accepted the license terms before this is called.
pub fn prepare(
    root: &Path,
    engine: &str,
    port: u16,
    vault: &dyn Vault,
    path: Option<&OsStr>,
    pulled: impl Fn(),
) -> Result<SqlServerContainer, String> {
    let server = SqlServerContainer {
        id: new_id()?,
        engine: engine.to_owned(),
        port,
        stop_failed: false,
    };
    engine_command(engine, &["pull", IMAGE], path, COMMAND_TIMEOUT)?;
    pulled();
    let initial = new_password()?;
    vault.save_new(&server.initial_target(), "sa", &initial)?;
    save(root, &server)?;
    // The password reaches the container through a file that exists only for `run`.
    let temp = root.join(".temp");
    fs::create_dir_all(&temp).map_err(|error| error.to_string())?;
    let env_file = temp.join(format!("mssql-{}.env", server.id));
    fs::write(
        &env_file,
        format!("ACCEPT_EULA=Y\nMSSQL_PID=Developer\nMSSQL_SA_PASSWORD={initial}\n"),
    )
    .map_err(|error| error.to_string())?;
    let published = format!("127.0.0.1:{port}:1433");
    let mount = format!("{}:/var/opt/mssql", server.volume());
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
            IMAGE,
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
            Ok(()) => format!("{PREPARATION_FAILED} ({error})"),
            Err(stop_error) => format!("{stop_error} ({error})"),
        });
    }
    Ok(server)
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
    as_sa(server, &admin, &project_sql(database, user, password))
        .map_err(|error| error.replace(password, "••••"))
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
    as_sa(
        server,
        &admin,
        &[format!(
            "ALTER LOGIN [{user}] WITH PASSWORD = N'{}'",
            password.replace('\'', "''")
        )],
    )
    .map_err(|error| error.replace(password, "••••"))
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
        let image = IMAGE.split('@').next().unwrap();
        assert_eq!(
            parse_state(&format!("true|abc|{image}"), "abc"),
            ContainerState::Running
        );
        assert_eq!(
            parse_state(&format!("false|abc|{image}"), "abc"),
            ContainerState::Stopped
        );
        assert!(matches!(
            parse_state(&format!("true|outro|{image}"), "abc"),
            ContainerState::NotOurs(_)
        ));
        assert!(matches!(
            parse_state("true|abc|mcr.microsoft.com/mssql/server:2019-latest", "abc"),
            ContainerState::NotOurs(_)
        ));
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
