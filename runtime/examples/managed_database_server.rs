//! Full life cycle of one managed database server, for manual and E2E checks:
//! `managed_database_server <postgresql|mysql|mariadb> <folder> <port>`.
//! Prepares the pinned package, initializes and starts the server (password in the
//! Windows Credential Manager), checks the administrator login, recognizes it, stops it,
//! starts it again, stops it and removes the test credential. Use a disposable folder
//! and a free port.

use localcodepilot_platform::credentials;
use localcodepilot_runtime::{
    database_packages,
    database_servers::{self, Recognition, SystemVault},
    databases::DatabaseEngine,
};
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let (Some(engine), Some(root), Some(port)) = (
        arguments.first().map(String::as_str),
        arguments.get(1).map(PathBuf::from),
        arguments.get(2).and_then(|port| port.parse::<u16>().ok()),
    ) else {
        eprintln!("uso: managed_database_server <postgresql|mysql|mariadb> <pasta> <porta>");
        return ExitCode::from(2);
    };
    let engine = match engine {
        "postgresql" => DatabaseEngine::PostgreSql,
        "mysql" => DatabaseEngine::MySql,
        "mariadb" => DatabaseEngine::MariaDb,
        _ => return ExitCode::from(2),
    };
    let package = database_packages::catalog_package(engine).expect("pinned package");
    let step = |name: &str, result: Result<(), String>| -> bool {
        match result {
            Ok(()) => {
                println!("ok: {name}");
                true
            }
            Err(error) => {
                println!("FALHOU: {name}: {error}");
                false
            }
        }
    };
    let archive = match package.download(&root, |_, _| {}) {
        Ok(path) => path,
        Err(error) => {
            println!("FALHOU: download: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !step(
        "pacote",
        package
            .prepare(&archive, &root)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    ) {
        return ExitCode::FAILURE;
    }
    let vault = SystemVault;
    let mut server = match database_servers::prepare(package, &root, port, &vault) {
        Ok(server) => {
            println!(
                "ok: preparar e iniciar ({}, PID {:?})",
                server.id, server.process_id
            );
            server
        }
        Err(error) => {
            println!("FALHOU: preparar: {error}");
            return ExitCode::FAILURE;
        }
    };
    let target = database_servers::credential_target(&server.id);
    let mut ok = true;
    let leftovers = std::fs::read_dir(root.join(".temp"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    ok &= step(
        "nenhum arquivo de senha sobrou",
        if leftovers == 0 {
            Ok(())
        } else {
            Err(format!("{leftovers} arquivo(s)"))
        },
    );
    ok &= step(
        "reconhecido como gerenciado",
        match database_servers::recognize(&server) {
            Recognition::Managed => Ok(()),
            other => Err(format!("{other:?}")),
        },
    );
    ok &= step(
        "senha certa entra; errada não",
        server
            .admin_password(&vault)
            .map_err(|e| e.to_string())
            .and_then(|password| {
                database_servers::admin_check(&server, &password).map_err(|e| e.to_string())
            })
            .and_then(
                |()| match database_servers::admin_check(&server, "senha-errada") {
                    Err(_) => Ok(()),
                    Ok(()) => Err("a senha errada entrou".into()),
                },
            ),
    );
    ok &= step(
        "parar",
        database_servers::stop(&mut server, &root, &vault).map_err(|e| e.to_string()),
    );
    ok &= step(
        "depois de parar, não está rodando",
        match database_servers::recognize(&server) {
            Recognition::NotRunning => Ok(()),
            other => Err(format!("{other:?}")),
        },
    );
    ok &= step(
        "iniciar de novo",
        database_servers::start(&mut server, &root).map_err(|e| e.to_string()),
    );
    ok &= step(
        "login depois de reiniciar",
        server
            .admin_password(&vault)
            .map_err(|e| e.to_string())
            .and_then(|password| {
                database_servers::admin_check(&server, &password).map_err(|e| e.to_string())
            }),
    );
    ok &= step(
        "parar de novo",
        database_servers::stop(&mut server, &root, &vault).map_err(|e| e.to_string()),
    );
    ok &= step("apagar a credencial de teste", credentials::delete(&target));
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
