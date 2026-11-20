//! Downloads, verifies and extracts one pinned database package, for manual and E2E checks:
//! `cargo run -p localcodepilot-runtime --example prepare_database_package -- <postgresql|mysql|mariadb> <folder>`.
//! Nothing is started.

use localcodepilot_runtime::{database_packages, databases::DatabaseEngine};
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let engine = match arguments.next().as_deref() {
        Some("postgresql") => DatabaseEngine::PostgreSql,
        Some("mysql") => DatabaseEngine::MySql,
        Some("mariadb") => DatabaseEngine::MariaDb,
        _ => {
            eprintln!("uso: prepare_database_package <postgresql|mysql|mariadb> <pasta>");
            return ExitCode::from(2);
        }
    };
    let Some(base) = arguments.next().map(PathBuf::from) else {
        eprintln!("informe a pasta de destino");
        return ExitCode::from(2);
    };
    let Some(package) = database_packages::catalog_package(engine) else {
        eprintln!("sem pacote para esse motor");
        return ExitCode::from(2);
    };
    let mut last = 0;
    let archive = match package.download(&base, |received, total| {
        let percent = received * 100 / total.max(1);
        if percent >= last + 10 {
            last = percent;
            println!("baixando {percent}%");
        }
    }) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    match package.prepare(&archive, &base) {
        Ok(folder) => {
            println!("pronto: {}", folder.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
