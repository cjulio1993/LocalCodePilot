//! E2E of the project's database on a managed server:
//! `project_database_check <postgresql|mysql|mariadb> <folder> <port>`. Prepares a server,
//! gives a throwaway project its database and user twice (the second time creates nothing),
//! logs in with the project's data, refuses an unsafe name, stops the server and removes
//! the test credential.

use localcodepilot_platform::credentials;
use localcodepilot_runtime::{
    database_connection::{ConnectionOutcome, read_credentials, test_connection},
    database_packages,
    database_servers::{self, ServerError, SystemVault},
    databases::{DatabaseEngine, DatabaseRequirement},
};
use std::{env, fs, path::PathBuf, process::ExitCode, time::Duration};

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let engine = match arguments.first().map(String::as_str) {
        Some("postgresql") => DatabaseEngine::PostgreSql,
        Some("mysql") => DatabaseEngine::MySql,
        Some("mariadb") => DatabaseEngine::MariaDb,
        _ => return ExitCode::from(2),
    };
    let root = PathBuf::from(&arguments[1]);
    let port: u16 = arguments[2].parse().unwrap();
    let vault = SystemVault;
    let package = database_packages::catalog_package(engine).unwrap();
    let archive = package.download(&root, |_, _| {}).unwrap();
    package.prepare(&archive, &root).unwrap();
    let mut server = match database_servers::prepare(package, &root, port, &vault) {
        Ok(server) => server,
        Err(error) => {
            println!("FALHOU: preparar: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut ok = true;
    let mut check = |name: &str, passed: bool, detail: String| {
        println!("{} {name}{detail}", if passed { "ok:" } else { "FALHOU:" });
        ok &= passed;
    };
    let first = database_servers::ensure_project_database(
        &server,
        &vault,
        "loja",
        "app_loja",
        "senha'do\\projeto",
    );
    check(
        "criar banco e usuário",
        first.as_ref().is_ok_and(|done| done.len() == 2),
        format!(" {first:?}"),
    );
    let second = database_servers::ensure_project_database(
        &server,
        &vault,
        "loja",
        "app_loja",
        "senha'do\\projeto",
    );
    check(
        "de novo não cria nada",
        second.as_ref().is_ok_and(Vec::is_empty),
        format!(" {second:?}"),
    );
    let project = root.join("projeto");
    fs::create_dir_all(&project).unwrap();
    let connection = if engine == DatabaseEngine::PostgreSql {
        "pgsql"
    } else {
        arguments[0].as_str()
    };
    fs::write(
        project.join(".env"),
        format!(
            "DB_CONNECTION={connection}\nDB_HOST=127.0.0.1\nDB_PORT={port}\nDB_DATABASE=loja\nDB_USERNAME=app_loja\nDB_PASSWORD=\"senha'do\\projeto\"\n"
        ),
    )
    .unwrap();
    let requirement = DatabaseRequirement {
        source: project.join(".env"),
        engine,
        host: Some("127.0.0.1".into()),
        port: Some(port),
        compose_service: None,
    };
    let test = test_connection(
        &requirement,
        &read_credentials(&requirement),
        Duration::from_secs(5),
    );
    check(
        "o projeto entra no próprio banco",
        test.as_ref()
            .is_some_and(|test| test.outcome == ConnectionOutcome::Connected),
        format!(" {:?}", test.map(|test| test.outcome)),
    );
    let unsafe_name =
        database_servers::ensure_project_database(&server, &vault, "loja;DROP", "app", "x");
    check(
        "nome inseguro é recusado",
        matches!(unsafe_name, Err(ServerError::InvalidName)),
        String::new(),
    );
    let stopped = database_servers::stop(&mut server, &root, &vault);
    check("parar", stopped.is_ok(), format!(" {stopped:?}"));
    let removed = credentials::delete(&database_servers::credential_target(&server.id));
    check(
        "apagar a credencial de teste",
        removed.is_ok(),
        String::new(),
    );
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
