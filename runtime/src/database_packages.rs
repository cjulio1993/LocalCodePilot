//! Official database server packages the app can download and prepare on Windows: the
//! catalog (one pinned version per engine, with its SHA-256), the verified download and
//! the extraction. Nothing here starts a server.
//!
//! Every version is pinned in `database-packages.json`, next to this file. A version only
//! goes in when the SHA-256 of our download matches an independent source:
//! - MariaDB: the `sha256sums.txt` the project signs and its download API;
//! - MySQL: the GPG signature of Oracle's release engineering on the zip;
//! - PostgreSQL: the official zip is hosted by EDB, which publishes no hash list, so the
//!   SHA-256 must match the one in Scoop's manifest, at a pinned revision.
//!
//! `scripts/verify-database-packages.ps1` checks the pins against those sources in CI.
//! A download whose SHA-256 differs is refused and deleted; there is no way to skip it.

use crate::databases::DatabaseEngine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const CATALOG_JSON: &str = include_str!("database-packages.json");

/// Written last in a prepared package folder: without it the folder is incomplete.
const READY_MARKER: &str = ".localcodepilot-pronto";

/// No data for this long fails the download as interrupted; a slow download that keeps
/// receiving data goes on.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DatabasePackage {
    /// `postgresql`, `mysql` or `mariadb`.
    pub engine: String,
    pub version: String,
    pub file_name: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    /// Disk space after preparing: the extracted package plus the data of a new server.
    pub space_bytes: u64,
    /// The single top folder inside the zip, removed on extraction.
    pub archive_root: String,
    /// The server program, relative to the prepared folder.
    pub server_program: String,
    /// Where the pinned SHA-256 was confirmed.
    pub provenance: String,
}

#[derive(Debug, Deserialize)]
struct Catalog {
    packages: Vec<DatabasePackage>,
}

fn catalog() -> &'static [DatabasePackage] {
    static CATALOG: OnceLock<Vec<DatabasePackage>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<Catalog>(CATALOG_JSON)
            .expect("database-packages.json is part of the build and must parse")
            .packages
    })
}

fn engine_key(engine: DatabaseEngine) -> Option<&'static str> {
    match engine {
        DatabaseEngine::PostgreSql => Some("postgresql"),
        DatabaseEngine::MySql => Some("mysql"),
        DatabaseEngine::MariaDb => Some("mariadb"),
        DatabaseEngine::SqlServer | DatabaseEngine::Sqlite => None,
    }
}

/// The pinned package of `engine`, in the catalog of every system (tests read it).
pub fn catalog_package(engine: DatabaseEngine) -> Option<&'static DatabasePackage> {
    let key = engine_key(engine)?;
    catalog().iter().find(|package| package.engine == key)
}

/// The package the app can prepare on this computer: Windows only in this version.
pub fn package_for_this_system(engine: DatabaseEngine) -> Option<&'static DatabasePackage> {
    if cfg!(windows) {
        catalog_package(engine)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// Connection problems, HTTP errors or a download that stopped.
    Network(String),
    /// The file is not the pinned one. It was deleted.
    Integrity {
        expected: String,
        actual: String,
    },
    /// The zip could not be read or has an unsafe or unexpected entry.
    Archive(String),
    Io(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(detail) => write!(formatter, "o download falhou: {detail}"),
            Self::Integrity { expected, actual } => write!(
                formatter,
                "o arquivo baixado não confere (SHA-256 esperado {expected}, recebido {actual}) e foi apagado"
            ),
            Self::Archive(detail) => write!(formatter, "o pacote não pôde ser extraído: {detail}"),
            Self::Io(detail) => write!(formatter, "{detail}"),
        }
    }
}

fn io_error(context: &str) -> impl Fn(io::Error) -> PackageError + '_ {
    move |error| PackageError::Io(format!("{context}: {error}"))
}

impl DatabasePackage {
    /// The prepared folder: `<base>\<engine>-<version>`.
    pub fn folder(&self, base: &Path) -> PathBuf {
        base.join(format!("{}-{}", self.engine, self.version))
    }

    /// The package is extracted, complete and from the pinned file.
    pub fn is_prepared(&self, base: &Path) -> bool {
        let folder = self.folder(base);
        fs::read_to_string(folder.join(READY_MARKER))
            .is_ok_and(|marker| marker.trim().eq_ignore_ascii_case(&self.sha256))
            && folder.join(&self.server_program).is_file()
    }

    fn download_path(&self, base: &Path) -> PathBuf {
        base.join("downloads").join(&self.file_name)
    }

    /// Downloads the pinned zip into `<base>\downloads`, resuming a partial download, and
    /// checks its SHA-256. `progress` gets the bytes received and the total. A file that
    /// does not match is deleted, so the next attempt downloads a new copy.
    pub fn download(
        &self,
        base: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<PathBuf, PackageError> {
        let target = self.download_path(base);
        if target.is_file() && sha256_of(&target)?.eq_ignore_ascii_case(&self.sha256) {
            progress(self.size_bytes, self.size_bytes);
            return Ok(target);
        }
        let _ = fs::remove_file(&target);
        let partial = target.with_extension("zip.part");
        if let Some(folder) = partial.parent() {
            fs::create_dir_all(folder)
                .map_err(io_error("não foi possível criar a pasta de downloads"))?;
        }
        let already = fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0);
        let already = if already >= self.size_bytes {
            0
        } else {
            already
        };
        // Same limits as the app's own update download: none in total, only to connect,
        // to get the response and per read (restarted whenever data arrives).
        let agent = ureq::Agent::config_builder()
            .timeout_global(None)
            .timeout_resolve(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(READ_TIMEOUT))
            .timeout_per_call(None)
            .timeout_recv_body(None)
            .https_only(true)
            .build()
            .new_agent();
        let mut request = agent.get(&self.url);
        if already > 0 {
            request = request.header("Range", &format!("bytes={already}-"));
        }
        let response = request
            .call()
            .map_err(|error| PackageError::Network(error.to_string()))?;
        // 206 continues the partial file; 200 means the server started over.
        let resumed = already > 0 && response.status().as_u16() == 206;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&partial)
            .map_err(io_error("não foi possível gravar o download"))?;
        let mut received = if resumed { already } else { 0 };
        let mut body = response.into_body().into_reader();
        let mut buffer = vec![0_u8; 256 * 1024];
        loop {
            let read = body
                .read(&mut buffer)
                .map_err(|error| PackageError::Network(error.to_string()))?;
            if read == 0 {
                break;
            }
            received += read as u64;
            if received > self.size_bytes {
                drop(file);
                let _ = fs::remove_file(&partial);
                return Err(PackageError::Network(format!(
                    "o servidor enviou mais que os {} bytes esperados",
                    self.size_bytes
                )));
            }
            file.write_all(&buffer[..read])
                .map_err(io_error("não foi possível gravar o download"))?;
            progress(received, self.size_bytes);
        }
        file.sync_all()
            .map_err(io_error("não foi possível gravar o download"))?;
        drop(file);
        if received < self.size_bytes {
            return Err(PackageError::Network(format!(
                "a conexão terminou com {received} de {} bytes",
                self.size_bytes
            )));
        }
        let actual = sha256_of(&partial)?;
        if !actual.eq_ignore_ascii_case(&self.sha256) {
            let _ = fs::remove_file(&partial);
            return Err(PackageError::Integrity {
                expected: self.sha256.clone(),
                actual,
            });
        }
        fs::rename(&partial, &target).map_err(io_error("não foi possível concluir o download"))?;
        Ok(target)
    }

    /// Extracts a verified zip into the prepared folder. It goes to a temporary folder
    /// first and is renamed at the end, so an interrupted extraction never looks prepared.
    /// The archive is hashed again here: only the pinned file is ever extracted.
    pub fn prepare(&self, archive: &Path, base: &Path) -> Result<PathBuf, PackageError> {
        let actual = sha256_of(archive)?;
        if !actual.eq_ignore_ascii_case(&self.sha256) {
            return Err(PackageError::Integrity {
                expected: self.sha256.clone(),
                actual,
            });
        }
        let folder = self.folder(base);
        if self.is_prepared(base) {
            return Ok(folder);
        }
        let staging = base.join(format!(
            ".{}-{}.extraindo-{}",
            self.engine,
            self.version,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&staging);
        let result = extract(archive, &self.archive_root, &staging).and_then(|()| {
            if !staging.join(&self.server_program).is_file() {
                return Err(PackageError::Archive(format!(
                    "o pacote não tem {}",
                    self.server_program
                )));
            }
            fs::write(staging.join(READY_MARKER), &self.sha256)
                .map_err(io_error("não foi possível concluir a extração"))?;
            // A leftover incomplete folder (no marker) is replaced; a prepared one was
            // returned above.
            if folder.exists() {
                fs::remove_dir_all(&folder).map_err(io_error(
                    "não foi possível substituir uma extração incompleta",
                ))?;
            }
            fs::rename(&staging, &folder).map_err(io_error("não foi possível concluir a extração"))
        });
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result.map(|()| folder)
    }
}

/// Extracts the entries under `root/` into `target`, refusing absolute paths, `..`,
/// and anything outside `root/`.
fn extract(archive: &Path, root: &str, target: &Path) -> Result<(), PackageError> {
    let file = File::open(archive).map_err(io_error("não foi possível abrir o pacote"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| PackageError::Archive(error.to_string()))?;
    fs::create_dir_all(target).map_err(io_error("não foi possível criar a pasta do pacote"))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| PackageError::Archive(error.to_string()))?;
        let name = entry
            .name()
            .map_err(|error| PackageError::Archive(error.to_string()))?
            .into_owned();
        let Some(relative) = entry_path(&name, root)? else {
            continue;
        };
        let destination = target.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(io_error("não foi possível criar uma pasta do pacote"))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(io_error("não foi possível criar uma pasta do pacote"))?;
        }
        let mut output =
            File::create(&destination).map_err(io_error("não foi possível extrair um arquivo"))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| PackageError::Archive(format!("{}: {error}", relative.display())))?;
    }
    Ok(())
}

/// The path of a zip entry below `root`, or `None` for `root` itself. An entry outside
/// `root`, absolute, or with `..` makes the whole package invalid.
fn entry_path(name: &str, root: &str) -> Result<Option<PathBuf>, PackageError> {
    let normalized = name.replace('\\', "/");
    let unsafe_entry = || PackageError::Archive(format!("entrada inválida no pacote: {name}"));
    let Some(rest) = normalized
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/').or(rest.is_empty().then_some("")))
    else {
        return Err(unsafe_entry());
    };
    let relative = Path::new(rest);
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(unsafe_entry());
    }
    Ok(Some(relative.to_path_buf()))
}

fn sha256_of(path: &Path) -> Result<String, PackageError> {
    let mut file = File::open(path).map_err(io_error("não foi possível ler o pacote"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(io_error("não foi possível ler o pacote"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lcp-db-packages-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, contents) in entries {
                if name.ends_with('/') {
                    writer.add_directory(*name, options).unwrap();
                } else {
                    writer.start_file(*name, options).unwrap();
                    writer.write_all(contents).unwrap();
                }
            }
            writer.finish().unwrap();
        }
        bytes.into_inner()
    }

    fn package_for(bytes: &[u8]) -> DatabasePackage {
        let hash: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        DatabasePackage {
            engine: "mariadb".into(),
            version: "0.0.1".into(),
            file_name: "teste.zip".into(),
            url: "https://example.invalid/teste.zip".into(),
            sha256: hash,
            size_bytes: bytes.len() as u64,
            space_bytes: 1,
            archive_root: "pacote".into(),
            server_program: "bin/servidor.exe".into(),
            provenance: "teste".into(),
        }
    }

    #[test]
    fn every_engine_the_app_prepares_has_one_pinned_package() {
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::MySql,
            DatabaseEngine::MariaDb,
        ] {
            let package = catalog_package(engine).expect("pinned package");
            assert_eq!(package.sha256.len(), 64, "{}", package.engine);
            assert!(package.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(package.url.starts_with("https://"), "{}", package.url);
            assert!(package.url.ends_with(&package.file_name), "{}", package.url);
            assert!(package.size_bytes > 10_000_000, "{}", package.engine);
            assert!(!package.provenance.is_empty());
        }
        for engine in [DatabaseEngine::SqlServer, DatabaseEngine::Sqlite] {
            assert!(catalog_package(engine).is_none());
        }
        assert_eq!(catalog().len(), 3);
    }

    #[test]
    fn only_entries_inside_the_root_are_extracted() {
        assert_eq!(entry_path("pacote/", "pacote").unwrap(), None);
        assert_eq!(
            entry_path("pacote/bin/x.exe", "pacote").unwrap(),
            Some(PathBuf::from("bin/x.exe"))
        );
        assert_eq!(
            entry_path("pacote\\bin\\x.exe", "pacote").unwrap(),
            Some(PathBuf::from("bin/x.exe"))
        );
        for unsafe_name in [
            "pacote/../fora.exe",
            "/pacote/bin/x.exe",
            "outro/bin/x.exe",
            "pacote-falso/bin/x.exe",
            "C:/pacote/x.exe",
        ] {
            assert!(entry_path(unsafe_name, "pacote").is_err(), "{unsafe_name}");
        }
    }

    #[test]
    fn a_verified_zip_is_prepared_once_and_marked_ready() {
        let base = temp_dir("prepare");
        let bytes = zip_with(&[
            ("pacote/", b""),
            ("pacote/bin/servidor.exe", b"exe"),
            ("pacote/share/dados.txt", b"x"),
        ]);
        let package = package_for(&bytes);
        let archive = base.join("teste.zip");
        fs::write(&archive, &bytes).unwrap();
        assert!(!package.is_prepared(&base));
        let folder = package.prepare(&archive, &base).unwrap();
        assert_eq!(folder, base.join("mariadb-0.0.1"));
        assert!(package.is_prepared(&base));
        assert_eq!(fs::read(folder.join("bin/servidor.exe")).unwrap(), b"exe");
        // No staging folder is left behind.
        let leftovers = fs::read_dir(&base)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains("extraindo"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_different_file_is_never_extracted() {
        let base = temp_dir("integrity");
        let bytes = zip_with(&[("pacote/bin/servidor.exe", b"exe")]);
        let package = package_for(&bytes);
        let archive = base.join("teste.zip");
        let tampered = zip_with(&[("pacote/bin/servidor.exe", b"outro")]);
        fs::write(&archive, &tampered).unwrap();
        assert!(matches!(
            package.prepare(&archive, &base),
            Err(PackageError::Integrity { .. })
        ));
        assert!(!package.folder(&base).exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_package_with_an_unsafe_entry_leaves_nothing_behind() {
        let base = temp_dir("unsafe");
        let bytes = zip_with(&[
            ("pacote/bin/servidor.exe", b"exe"),
            ("pacote/../fora.txt", b"x"),
        ]);
        let package = package_for(&bytes);
        let archive = base.join("teste.zip");
        fs::write(&archive, &bytes).unwrap();
        assert!(matches!(
            package.prepare(&archive, &base),
            Err(PackageError::Archive(_))
        ));
        assert!(!package.folder(&base).exists());
        assert!(!base.join("fora.txt").exists());
        assert!(!base.parent().unwrap().join("fora.txt").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_package_without_its_server_is_refused() {
        let base = temp_dir("no-server");
        let bytes = zip_with(&[("pacote/leia.txt", b"x")]);
        let package = package_for(&bytes);
        let archive = base.join("teste.zip");
        fs::write(&archive, &bytes).unwrap();
        assert!(matches!(
            package.prepare(&archive, &base),
            Err(PackageError::Archive(_))
        ));
        assert!(!package.is_prepared(&base));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn an_already_downloaded_pinned_file_is_reused_without_network() {
        let base = temp_dir("reuse");
        let bytes = zip_with(&[("pacote/bin/servidor.exe", b"exe")]);
        let package = package_for(&bytes);
        fs::create_dir_all(base.join("downloads")).unwrap();
        fs::write(base.join("downloads/teste.zip"), &bytes).unwrap();
        let mut seen = None;
        let path = package
            .download(&base, |received, total| seen = Some((received, total)))
            .unwrap();
        assert_eq!(path, base.join("downloads/teste.zip"));
        assert_eq!(seen, Some((bytes.len() as u64, bytes.len() as u64)));
        let _ = fs::remove_dir_all(&base);
    }
}
