//! Disposable fixture for the connection test (E2E and screenshots only):
//! `database_connection_fixture prepare <root> <projects> <port>` prepares a managed MariaDB
//! server and four throwaway Laravel-like projects whose `.env` point to it (connected,
//! wrong password, missing database, nothing listening), and checks each with
//! `test_connection`; `database_connection_fixture clean <root>` stops the server and
//! removes the test credential. The `.env` files of this fixture hold a throwaway
//! password of a throwaway server, on purpose.

use localcodepilot_platform::credentials;
use localcodepilot_runtime::{
    database_connection::{ConnectionOutcome, read_credentials, test_connection},
    database_packages,
    database_servers::{self, SystemVault},
    databases::{DatabaseEngine, detect_database_requirements},
};
use std::{env, fs, path::Path, process::ExitCode, time::Duration};

fn project(projects: &Path, name: &str, port: u16, database: &str, password: &str) {
    let folder = projects.join(name);
    fs::create_dir_all(folder.join("public")).unwrap();
    fs::write(folder.join("artisan"), "<?php").unwrap();
    fs::write(
        folder.join("composer.json"),
        r#"{"name":"exemplo/app","require":{"php":"^8.2","laravel/framework":"^11.0"}}"#,
    )
    .unwrap();
    fs::write(
        folder.join(".env"),
        format!(
            "APP_NAME={name}\nDB_CONNECTION=mariadb\nDB_HOST=127.0.0.1\nDB_PORT={port}\nDB_DATABASE={database}\nDB_USERNAME=root\nDB_PASSWORD={password}\n"
        ),
    )
    .unwrap();
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let root = Path::new(arguments.get(1).map(String::as_str).unwrap_or(""));
    let vault = SystemVault;
    match arguments.first().map(String::as_str) {
        Some("prepare") => {
            let projects = Path::new(&arguments[2]);
            let port: u16 = arguments[3].parse().unwrap();
            let package = database_packages::catalog_package(DatabaseEngine::MariaDb).unwrap();
            let archive = package.download(root, |_, _| {}).unwrap();
            package.prepare(&archive, root).unwrap();
            let server = database_servers::prepare(package, root, port, &vault).unwrap();
            let password = server.admin_password(&vault).unwrap();
            project(projects, "loja-conectada", port, "mysql", &password);
            project(projects, "loja-senha-errada", port, "mysql", "senha-errada");
            project(
                projects,
                "loja-sem-banco",
                port,
                "loja_inexistente",
                &password,
            );
            project(projects, "loja-banco-parado", port + 1, "mysql", &password);
            let mut ok = true;
            for (name, expected) in [
                ("loja-conectada", ConnectionOutcome::Connected),
                ("loja-senha-errada", ConnectionOutcome::AccessDenied),
                ("loja-sem-banco", ConnectionOutcome::DatabaseMissing),
                ("loja-banco-parado", ConnectionOutcome::NotResponding),
            ] {
                let requirement = detect_database_requirements(&projects.join(name))
                    .into_iter()
                    .next()
                    .unwrap();
                let test = test_connection(
                    &requirement,
                    &read_credentials(&requirement),
                    Duration::from_secs(5),
                )
                .unwrap();
                let shown = format!("{test:?}");
                let leaked = shown.contains(&password);
                println!(
                    "{} {name}: {:?} (esperado {:?}){}",
                    if test.outcome == expected && !leaked {
                        "ok:"
                    } else {
                        "FALHOU:"
                    },
                    test.outcome,
                    expected,
                    if leaked {
                        " — o resultado mostra a senha"
                    } else {
                        ""
                    }
                );
                ok &= test.outcome == expected && !leaked;
            }
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("clean") => {
            for mut server in database_servers::load_registry(root) {
                let _ = database_servers::stop(&mut server, root, &vault);
                let _ = credentials::delete(&database_servers::credential_target(&server.id));
            }
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(2),
    }
}
