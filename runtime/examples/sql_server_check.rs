//! E2E of the database containers the app manages (CI on Linux with Docker, or any computer
//! with a container engine ready):
//! `sql_server_check <docker|podman> <folder> <port> [sqlserver|postgresql]`.
//! PostgreSQL (T030) also checks that the port is published only on 127.0.0.1 and that the
//! project's role has no server-wide rights.
//! Accepts the license on behalf of the test only (the app never does: the person checks the
//! box), pulls the pinned image, creates the container, checks that the initial `sa` password
//! no longer works, gives a project its database and login, tests the project's login,
//! stops and starts the container, and removes everything at the end.

use localcodepilot_runtime::{
    database_connection::{ConnectionOutcome, read_credentials, test_connection},
    database_servers::Vault,
    databases::{DatabaseEngine, DatabaseRequirement},
    sql_server_container::{self, ContainerKind, ContainerState},
};
use std::{
    cell::RefCell, collections::HashMap, env, fs, path::Path, path::PathBuf, process::ExitCode,
    time::Duration,
};

/// The Credential Manager exists only on Windows: the test keeps the passwords in memory.
#[derive(Default)]
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

fn project(
    kind: ContainerKind,
    root: &Path,
    name: &str,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
) -> DatabaseRequirement {
    let folder = root.join(name);
    fs::create_dir_all(&folder).unwrap();
    let env_file = folder.join(".env");
    fs::write(
        &env_file,
        format!(
            "DB_CONNECTION={}\nDB_HOST=127.0.0.1\nDB_PORT={port}\nDB_DATABASE={database}\nDB_USERNAME={user}\nDB_PASSWORD=\"{password}\"\n",
            match kind {
                ContainerKind::SqlServer => "sqlsrv",
                ContainerKind::PostgreSql => "pgsql",
            }
        ),
    )
    .unwrap();
    DatabaseRequirement {
        source: env_file,
        engine: match kind {
            ContainerKind::SqlServer => DatabaseEngine::SqlServer,
            ContainerKind::PostgreSql => DatabaseEngine::PostgreSql,
        },
        host: Some("127.0.0.1".into()),
        port: Some(port),
        compose_service: None,
    }
}

fn login(requirement: &DatabaseRequirement) -> Option<ConnectionOutcome> {
    test_connection(
        requirement,
        &read_credentials(requirement),
        Duration::from_secs(20),
    )
    .map(|test| test.outcome)
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let engine = arguments
        .first()
        .cloned()
        .unwrap_or_else(|| "docker".into());
    let root = PathBuf::from(
        arguments
            .get(1)
            .cloned()
            .unwrap_or_else(|| "lcp-sqlserver".into()),
    );
    let port: u16 = arguments
        .get(2)
        .and_then(|port| port.parse().ok())
        .unwrap_or(14333);
    let kind = match arguments.get(3).map(String::as_str) {
        Some("postgresql") => ContainerKind::PostgreSql,
        _ => ContainerKind::SqlServer,
    };
    fs::create_dir_all(&root).unwrap();
    let vault = MemoryVault::default();
    let mut ok = true;
    let mut check = |name: &str, passed: bool, detail: String| {
        println!("{} {name}{detail}", if passed { "ok:" } else { "FALHOU:" });
        ok &= passed;
    };
    let server =
        match sql_server_container::prepare(&root, kind, &engine, port, &vault, None, |step| {
            println!("etapa: {step:?}")
        }) {
            Ok(server) => server,
            Err(error) => {
                println!("FALHOU: preparar: {error}");
                return ExitCode::FAILURE;
            }
        };
    if let Ok(output) = std::process::Command::new(&engine)
        .args(["image", "inspect", "--format", "{{.Size}}", kind.image()])
        .output()
    {
        let bytes: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        println!("imagem extraída: {} MB", bytes / 1_000_000);
    }
    // The space on disk once extracted, as the files take it inside the image.
    if let Ok(output) = std::process::Command::new(&engine)
        .args([
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            kind.image(),
            "-c",
            "du -sxm / 2>/dev/null | cut -f1",
        ])
        .output()
    {
        println!(
            "espaço extraído: {} MB",
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    check(
        "preparado e rodando",
        sql_server_container::state(&server, None) == Ok(ContainerState::Running),
        String::new(),
    );
    let leftovers = fs::read_dir(root.join(".temp"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    check(
        "nenhum arquivo de senha sobrou",
        leftovers == 0,
        format!(" ({leftovers})"),
    );
    // Only 127.0.0.1 on the computer: never 0.0.0.0 or another interface.
    let published = std::process::Command::new(&engine)
        .args(["port", &server.container()])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    check(
        "porta só em 127.0.0.1",
        !published.trim().is_empty()
            && published
                .lines()
                .all(|line| line.trim().ends_with(&format!("127.0.0.1:{port}"))),
        format!(" ({})", published.trim().replace('\n', "; ")),
    );
    check(
        "a senha inicial do administrador foi trocada",
        vault
            .0
            .borrow()
            .keys()
            .all(|key| !key.ends_with("-inicial")),
        String::new(),
    );
    let created = sql_server_container::ensure_project_database(
        &server,
        &vault,
        "loja",
        "app_loja",
        "senha'do-projeto",
    );
    check(
        "banco e login do projeto",
        created.is_ok(),
        format!(" {created:?}"),
    );
    let again = sql_server_container::ensure_project_database(
        &server,
        &vault,
        "loja",
        "app_loja",
        "senha'do-projeto",
    );
    check("de novo não falha", again.is_ok(), format!(" {again:?}"));
    if kind == ContainerKind::PostgreSql {
        // The project's role: no superuser, no new databases or roles.
        let rights = std::process::Command::new(&engine)
            .args([
                "exec",
                &server.container(),
                "psql",
                "-U",
                sql_server_container::POSTGRES_ADMIN,
                "-d",
                "postgres",
                "-tAc",
                "SELECT rolsuper::text || rolcreatedb::text || rolcreaterole::text FROM pg_roles WHERE rolname = 'app_loja'",
            ])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
        check(
            "o papel do projeto não tem poderes no servidor",
            rights == "falsefalsefalse",
            format!(" ({rights})"),
        );
    }
    let good = project(
        kind,
        &root,
        "certo",
        port,
        "loja",
        "app_loja",
        "senha'do-projeto",
    );
    check(
        "o projeto entra no próprio banco",
        login(&good) == Some(ConnectionOutcome::Connected),
        format!(" {:?}", login(&good)),
    );
    let wrong = project(
        kind,
        &root,
        "senha-errada",
        port,
        "loja",
        "app_loja",
        "errada",
    );
    check(
        "senha errada é recusada",
        login(&wrong) == Some(ConnectionOutcome::AccessDenied),
        format!(" {:?}", login(&wrong)),
    );
    let missing = project(
        kind,
        &root,
        "sem-banco",
        port,
        "outro",
        "app_loja",
        "senha'do-projeto",
    );
    check(
        "banco ausente",
        login(&missing) == Some(ConnectionOutcome::DatabaseMissing),
        format!(" {:?}", login(&missing)),
    );
    let stopped = sql_server_container::stop(&server, None);
    check(
        "parar (confirmado pelo inspect)",
        stopped.is_ok()
            && sql_server_container::state(&server, None) == Ok(ContainerState::Stopped),
        format!(" {stopped:?}"),
    );
    let started = sql_server_container::start(&server, &vault, None);
    check(
        "iniciar de novo com a senha guardada",
        started.is_ok(),
        format!(" {started:?}"),
    );
    check(
        "o projeto entra depois de reiniciar",
        login(&good) == Some(ConnectionOutcome::Connected),
        String::new(),
    );
    let _ = sql_server_container::stop(&server, None);
    // Cleanup of the test: container, volume and credentials.
    let container = server.container();
    let volume = server.volume();
    for args in [
        ["rm", "-f", container.as_str()],
        ["volume", "rm", volume.as_str()],
    ] {
        let _ = std::process::Command::new(&engine).args(args).status();
    }
    sql_server_container::forget_credentials(&server, &vault);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
