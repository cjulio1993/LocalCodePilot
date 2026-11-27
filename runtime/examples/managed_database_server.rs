//! Full life cycle of one managed database server, for manual and E2E checks:
//! `managed_database_server <postgresql|mysql|mariadb> <folder> <port>`.
//! Prepares the pinned package, initializes and starts the server (password in the
//! system vault), checks the administrator login, recognizes it, stops it,
//! starts it again, stops it and removes the test credential. Use a disposable folder
//! and a free port.

use localcodepilot_platform::credentials;
use localcodepilot_runtime::{
    database_packages,
    database_servers::{self, Recognition, SystemVault},
    databases::DatabaseEngine,
};
use std::{env, path::PathBuf, process::ExitCode};

/// A wrong password with a mark, to look for in logs and errors.
const WRONG_MARKED: &str = "MARCA-ERRADA-7f3c9";

/// Searches every file of the server folder, the registry and the server's command line
/// for any of `secrets`.
fn secret_leaks(
    root: &std::path::Path,
    server: &database_servers::ManagedServer,
    secrets: &[&str],
) -> Result<(), String> {
    fn files(folder: &std::path::Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let mut paths = vec![root.join("servidores.json")];
    files(&server.folder, &mut paths);
    files(&root.join(".temp"), &mut paths);
    let mut leaks = vec![];
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
            if bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
            {
                leaks.push(path.display().to_string());
            }
        }
    }
    if let Some(details) = server
        .process_id
        .and_then(localcodepilot_platform::ports::process_details)
    {
        let line = details.command_line.join(" ");
        if secrets
            .iter()
            .any(|secret| !secret.is_empty() && line.contains(secret))
        {
            leaks.push("linha de comando do servidor".into());
        }
    }
    println!("  ({} arquivos verificados)", paths.len());
    if leaks.is_empty() {
        Ok(())
    } else {
        Err(leaks.join(", "))
    }
}

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
    // The same check the app makes before it offers to prepare the engine here.
    let package = match database_packages::availability(engine) {
        Ok(package) => {
            println!("ok: pacote para este sistema: {}", package.file_name);
            package
        }
        Err(reason) => {
            println!("FALHOU: pacote para este sistema: {reason}");
            return ExitCode::FAILURE;
        }
    };
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
                |()| match database_servers::admin_check(&server, WRONG_MARKED) {
                    Err(error) if error.to_string().contains(WRONG_MARKED) => {
                        Err("o erro mostra a senha errada".into())
                    }
                    Err(_) => Ok(()),
                    Ok(()) => Err("a senha errada entrou".into()),
                },
            ),
    );
    // CA-9: neither the real password nor a marked wrong one appears in any file of the
    // server (logs, configuration, data), in the registry, or on the server's command line.
    let real = server.admin_password(&vault).unwrap_or_default();
    ok &= step(
        "nenhuma senha nos arquivos, no registro nem na linha de comando",
        secret_leaks(&root, &server, &[real.as_str(), WRONG_MARKED]),
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
