//! Official database server packages the app can download and prepare: the catalog (one
//! pinned version per engine and system, with its SHA-256), the verified download and the
//! extraction. Nothing here starts a server.
//!
//! Every version is pinned in `database-packages.json`, next to this file. A version only
//! goes in when the SHA-256 of our download matches an independent source:
//! - MariaDB: the `sha256sums.txt` the project signs and its download API;
//! - MySQL: the GPG signature of Oracle's release engineering on the archive;
//! - PostgreSQL: the official zip is hosted by EDB, which publishes no hash list, so the
//!   SHA-256 must match the one in Scoop's manifest, at a pinned revision. EDB publishes no
//!   checksum for macOS and no current Linux binaries: PostgreSQL is Windows only (T029).
//!
//! `scripts/verify-database-packages.ps1` checks the pins against those sources in CI.
//! A download whose SHA-256 differs is refused and deleted; there is no way to skip it.

use crate::databases::DatabaseEngine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
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
    /// `windows`, `macos` or `linux` (`std::env::consts::OS`).
    pub os: String,
    /// `x86_64` or `aarch64` (`std::env::consts::ARCH`).
    pub arch: String,
    /// The oldest system the package runs on: the glibc version on Linux ("2.28"), the
    /// macOS major version ("15"). None on Windows.
    #[serde(default)]
    pub min_system: Option<String>,
    pub file_name: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    /// Disk space after preparing: the extracted package plus the data of a new server.
    pub space_bytes: u64,
    /// The single top folder inside the archive, removed on extraction.
    pub archive_root: String,
    /// Top folders of the archive that are not extracted (test suites, headers).
    #[serde(default)]
    pub skip: Vec<String>,
    /// The server program, relative to the prepared folder.
    pub server_program: String,
    /// Where the pinned SHA-256 was confirmed.
    pub provenance: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    TarGz,
    TarXz,
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

/// Every pinned package of `engine`, for every system (tests and the CI check read it).
pub fn catalog_packages(engine: DatabaseEngine) -> Vec<&'static DatabasePackage> {
    let Some(key) = engine_key(engine) else {
        return Vec::new();
    };
    catalog()
        .iter()
        .filter(|package| package.engine == key)
        .collect()
}

/// The pinned package of `engine` built for this system and processor, whether or not this
/// computer meets its other requirements (see `availability`).
pub fn catalog_package(engine: DatabaseEngine) -> Option<&'static DatabasePackage> {
    catalog_package_for(engine, std::env::consts::OS, std::env::consts::ARCH)
}

fn catalog_package_for(
    engine: DatabaseEngine,
    os: &str,
    arch: &str,
) -> Option<&'static DatabasePackage> {
    catalog_packages(engine)
        .into_iter()
        .find(|package| package.os == os && package.arch == arch)
}

/// Whether the app can prepare `engine` on this computer: the package, or why not, in a
/// sentence for the card's technical details.
pub fn availability(engine: DatabaseEngine) -> Result<&'static DatabasePackage, String> {
    let Some(package) = catalog_package(engine) else {
        return Err(format!(
            "não há pacote oficial verificável de {} para {} ({})",
            engine_name(engine),
            system_name(std::env::consts::OS),
            std::env::consts::ARCH
        ));
    };
    host::meets(package)?;
    Ok(package)
}

/// The package the app can prepare on this computer.
pub fn package_for_this_system(engine: DatabaseEngine) -> Option<&'static DatabasePackage> {
    availability(engine).ok()
}

fn engine_name(engine: DatabaseEngine) -> &'static str {
    match engine {
        DatabaseEngine::PostgreSql => "PostgreSQL",
        DatabaseEngine::MySql => "MySQL",
        DatabaseEngine::MariaDb => "MariaDB",
        DatabaseEngine::SqlServer => "SQL Server",
        DatabaseEngine::Sqlite => "SQLite",
    }
}

fn system_name(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// `"2.39"` is at least `"2.28"`; `"15.1"` at least `"15"`.
#[cfg_attr(windows, allow(dead_code))]
fn version_at_least(actual: &str, minimum: &str) -> bool {
    let numbers = |text: &str| {
        text.split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (actual, minimum) = (numbers(actual), numbers(minimum));
    if actual.is_empty() {
        return false;
    }
    for index in 0..minimum.len().max(actual.len()) {
        let have = actual.get(index).copied().unwrap_or(0);
        let need = minimum.get(index).copied().unwrap_or(0);
        if have != need {
            return have > need;
        }
    }
    true
}

/// What this computer must have for a package to run: checked before the app offers to
/// prepare it, so an offer never ends in a server that cannot start.
mod host {
    use super::DatabasePackage;

    pub(super) fn meets(package: &DatabasePackage) -> Result<(), String> {
        match package.os.as_str() {
            "linux" => linux(package),
            "macos" => macos(package),
            _ => Ok(()),
        }
    }

    #[cfg(target_os = "linux")]
    fn linux(package: &DatabasePackage) -> Result<(), String> {
        let Some(glibc) = glibc_version() else {
            return Err(
                "este Linux não usa a glibc (por exemplo, Alpine), e os pacotes oficiais precisam dela"
                    .into(),
            );
        };
        if let Some(minimum) = &package.min_system
            && !super::version_at_least(&glibc, minimum)
        {
            return Err(format!(
                "o pacote oficial precisa da glibc {minimum} ou mais nova, e este Linux tem a {glibc}"
            ));
        }
        if package.engine == "mysql" && super::libaio::system_library().is_none() {
            return Err(
                "falta a biblioteca libaio do sistema, que o MySQL usa (pacote libaio1 ou libaio)"
                    .into(),
            );
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn linux(_: &DatabasePackage) -> Result<(), String> {
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn glibc_version() -> Option<String> {
        // SAFETY: glibc returns a pointer to a static, NUL-terminated string.
        let version = unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) };
        Some(version.to_string_lossy().into_owned())
    }

    #[cfg(all(target_os = "linux", not(target_env = "gnu")))]
    fn glibc_version() -> Option<String> {
        None
    }

    #[cfg(target_os = "macos")]
    fn macos(package: &DatabasePackage) -> Result<(), String> {
        // Read once: the card asks on every frame, and the version does not change while
        // the app runs.
        static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let version = VERSION.get_or_init(|| {
            let mut command = std::process::Command::new("sw_vers");
            command.arg("-productVersion");
            command
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .unwrap_or_default()
        });
        match &package.min_system {
            Some(minimum) if !super::version_at_least(version, minimum) => Err(format!(
                "o pacote oficial precisa do macOS {minimum} ou mais novo, e este Mac tem o {}",
                if version.is_empty() {
                    "?"
                } else {
                    version.as_str()
                }
            )),
            _ => Ok(()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn macos(_: &DatabasePackage) -> Result<(), String> {
        Ok(())
    }
}

/// MySQL's Linux builds load `libaio.so.1`. Ubuntu 24.04 and later ship it only as
/// `libaio.so.1t64`, so the app links that name inside the package's own `lib/private`
/// folder, which the server already searches (rpath): nothing changes on the system.
pub mod libaio {
    use std::path::{Path, PathBuf};

    const FOLDERS: [&str; 8] = [
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib64",
        "/usr/lib64",
        "/lib",
        "/usr/lib",
    ];

    /// The system's libaio: the classic name first, then the 64-bit time one.
    pub fn system_library() -> Option<PathBuf> {
        ["libaio.so.1", "libaio.so.1t64"].iter().find_map(|name| {
            FOLDERS
                .iter()
                .map(|folder| Path::new(folder).join(name))
                .find(|path| path.exists())
        })
    }

    /// Links `libaio.so.1` in `<program>/lib/private` when the system has only the new
    /// name. Nothing to do when the classic name exists.
    pub fn link_if_needed(program_folder: &Path) -> std::io::Result<()> {
        let Some(library) = system_library() else {
            return Ok(());
        };
        if library
            .file_name()
            .is_some_and(|name| name == "libaio.so.1")
        {
            return Ok(());
        }
        let private = program_folder.join("lib").join("private");
        std::fs::create_dir_all(&private)?;
        let link = private.join("libaio.so.1");
        if link.exists() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&library, &link)
        }
        #[cfg(not(unix))]
        {
            let _ = link;
            Ok(())
        }
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
    fn format(&self) -> Format {
        if self.file_name.ends_with(".tar.gz") || self.file_name.ends_with(".tgz") {
            Format::TarGz
        } else if self.file_name.ends_with(".tar.xz") {
            Format::TarXz
        } else {
            Format::Zip
        }
    }

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
        let partial = target.with_file_name(format!("{}.part", self.file_name));
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
        let result = extract(
            archive,
            self.format(),
            &self.archive_root,
            &self.skip,
            &staging,
        )
        .and_then(|()| {
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

fn extract(
    archive: &Path,
    format: Format,
    root: &str,
    skip: &[String],
    target: &Path,
) -> Result<(), PackageError> {
    let open = || File::open(archive).map_err(io_error("não foi possível abrir o pacote"));
    match format {
        Format::Zip => extract_zip(archive, root, skip, target),
        Format::TarGz => extract_tar(
            flate2::read::GzDecoder::new(BufReader::new(open()?)),
            root,
            skip,
            target,
        ),
        Format::TarXz => {
            // lzma-rs decompresses into a writer: the tar goes to a file next to the
            // extraction and is read from there, then removed.
            let tar = target.with_extension("tar");
            let result = (|| {
                let mut output = BufWriter::new(
                    File::create(&tar)
                        .map_err(io_error("não foi possível descompactar o pacote"))?,
                );
                lzma_rs::xz_decompress(&mut BufReader::new(open()?), &mut output)
                    .map_err(|error| PackageError::Archive(error.to_string()))?;
                output
                    .flush()
                    .map_err(io_error("não foi possível descompactar o pacote"))?;
                drop(output);
                extract_tar(
                    BufReader::new(
                        File::open(&tar).map_err(io_error("não foi possível abrir o pacote"))?,
                    ),
                    root,
                    skip,
                    target,
                )
            })();
            let _ = fs::remove_file(&tar);
            result
        }
    }
}

/// Extracts the entries of a tar under `root/` into `target`, with the same rules as the
/// zip: nothing absolute, no `..`, nothing outside `root/`. File modes are kept (the
/// programs must stay executable); a link may only point inside the package.
fn extract_tar(
    reader: impl Read,
    root: &str,
    skip: &[String],
    target: &Path,
) -> Result<(), PackageError> {
    let archive_error = |error: io::Error| PackageError::Archive(error.to_string());
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    archive.set_unpack_xattrs(false);
    fs::create_dir_all(target).map_err(io_error("não foi possível criar a pasta do pacote"))?;
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        let name = entry
            .path()
            .map_err(archive_error)?
            .to_string_lossy()
            .into_owned();
        let Some(relative) = entry_path(&name, root)? else {
            continue;
        };
        if skipped(&relative, skip) {
            continue;
        }
        let destination = target.join(&relative);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(io_error("não foi possível criar uma pasta do pacote"))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(io_error("não foi possível criar uma pasta do pacote"))?;
        }
        if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(archive_error)?
                .ok_or_else(|| PackageError::Archive(format!("link sem destino: {name}")))?;
            if !link_stays_inside(&relative, &link) {
                return Err(PackageError::Archive(format!(
                    "link para fora do pacote: {name}"
                )));
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &destination)
                .map_err(io_error("não foi possível criar um link do pacote"))?;
            #[cfg(not(unix))]
            return Err(PackageError::Archive(format!(
                "links não são extraídos neste sistema: {name}"
            )));
            #[cfg(unix)]
            continue;
        }
        if kind.is_hard_link() {
            let link = entry
                .link_name()
                .map_err(archive_error)?
                .ok_or_else(|| PackageError::Archive(format!("link sem destino: {name}")))?;
            let Some(source) = entry_path(&link.to_string_lossy(), root)? else {
                return Err(PackageError::Archive(format!("link inválido: {name}")));
            };
            fs::copy(target.join(source), &destination)
                .map_err(io_error("não foi possível extrair um arquivo"))?;
            continue;
        }
        if !kind.is_file() {
            return Err(PackageError::Archive(format!(
                "entrada inesperada no pacote: {name}"
            )));
        }
        entry
            .unpack(&destination)
            .map_err(|error| PackageError::Archive(format!("{}: {error}", relative.display())))?;
    }
    Ok(())
}

/// Entries under a top folder the catalog leaves out.
fn skipped(relative: &Path, skip: &[String]) -> bool {
    relative.components().next().is_some_and(|first| {
        skip.iter()
            .any(|folder| first.as_os_str() == folder.as_str())
    })
}

/// Whether a link at `relative` (inside the package) pointing to `link` stays inside it.
fn link_stays_inside(relative: &Path, link: &Path) -> bool {
    let mut depth: Vec<&std::ffi::OsStr> = relative
        .parent()
        .map(|parent| parent.iter().collect())
        .unwrap_or_default();
    for part in link.components() {
        match part {
            Component::Normal(name) => depth.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth.pop().is_none() {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// Extracts the entries under `root/` into `target`, refusing absolute paths, `..`,
/// and anything outside `root/`.
fn extract_zip(
    archive: &Path,
    root: &str,
    skip: &[String],
    target: &Path,
) -> Result<(), PackageError> {
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
        if skipped(&relative, skip) {
            continue;
        }
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
            os: "windows".into(),
            arch: "x86_64".into(),
            min_system: None,
            file_name: "teste.zip".into(),
            url: "https://example.invalid/teste.zip".into(),
            sha256: hash,
            size_bytes: bytes.len() as u64,
            space_bytes: 1,
            archive_root: "pacote".into(),
            skip: vec![],
            server_program: "bin/servidor.exe".into(),
            provenance: "teste".into(),
        }
    }

    #[test]
    fn every_pinned_package_is_complete_and_unique_per_system() {
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::MySql,
            DatabaseEngine::MariaDb,
        ] {
            assert!(catalog_package_for(engine, "windows", "x86_64").is_some());
            for package in catalog_packages(engine) {
                assert_eq!(package.sha256.len(), 64, "{}", package.file_name);
                assert!(package.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(package.url.starts_with("https://"), "{}", package.url);
                assert!(package.url.ends_with(&package.file_name), "{}", package.url);
                assert!(package.size_bytes > 10_000_000, "{}", package.file_name);
                assert!(
                    package.space_bytes > package.size_bytes,
                    "{}",
                    package.file_name
                );
                assert!(!package.provenance.is_empty());
                assert!(["windows", "macos", "linux"].contains(&package.os.as_str()));
                assert!(["x86_64", "aarch64"].contains(&package.arch.as_str()));
                // Outside Windows the minimum system is always stated and checked.
                assert_eq!(package.os != "windows", package.min_system.is_some());
                assert_eq!(
                    package.os == "windows",
                    package.server_program.ends_with(".exe")
                );
                let same_system = catalog_packages(engine)
                    .into_iter()
                    .filter(|other| other.os == package.os && other.arch == package.arch)
                    .count();
                assert_eq!(same_system, 1, "{}", package.file_name);
            }
        }
        for engine in [DatabaseEngine::SqlServer, DatabaseEngine::Sqlite] {
            assert!(catalog_packages(engine).is_empty());
        }
        assert_eq!(catalog().len(), 7);
    }

    #[test]
    fn macos_and_linux_get_only_the_engines_with_a_verifiable_package() {
        let has = |engine, os, arch| catalog_package_for(engine, os, arch).is_some();
        assert!(has(DatabaseEngine::MySql, "linux", "x86_64"));
        assert!(has(DatabaseEngine::MySql, "macos", "aarch64"));
        assert!(has(DatabaseEngine::MySql, "macos", "x86_64"));
        assert!(has(DatabaseEngine::MariaDb, "linux", "x86_64"));
        // No official checksum (EDB for macOS) or no official build (the rest).
        assert!(!has(DatabaseEngine::PostgreSql, "macos", "aarch64"));
        assert!(!has(DatabaseEngine::PostgreSql, "linux", "x86_64"));
        assert!(!has(DatabaseEngine::MariaDb, "macos", "aarch64"));
        assert!(!has(DatabaseEngine::MySql, "linux", "aarch64"));
        // Never another engine in its place.
        let mysql = catalog_package_for(DatabaseEngine::MySql, "linux", "x86_64").unwrap();
        assert_eq!(mysql.engine, "mysql");
        assert_eq!(mysql.format(), Format::TarXz);
        let mariadb = catalog_package_for(DatabaseEngine::MariaDb, "linux", "x86_64").unwrap();
        assert_eq!(mariadb.format(), Format::TarGz);
        assert!(mariadb.skip.contains(&"mariadb-test".to_owned()));
    }

    #[test]
    fn system_versions_compare_by_number() {
        assert!(version_at_least("2.39", "2.28"));
        assert!(version_at_least("2.28", "2.28"));
        assert!(!version_at_least("2.17", "2.28"));
        assert!(version_at_least("15.1", "15"));
        assert!(version_at_least("26.0", "15"));
        assert!(!version_at_least("14.7.1", "15"));
        assert!(!version_at_least("", "15"));
    }

    fn tar_gz_with(entries: &[(&str, &[u8])], links: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Regular);
            builder.append_data(&mut header, name, *contents).unwrap();
        }
        for (name, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Symlink);
            builder.append_link(&mut header, name, target).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn tar_package(bytes: &[u8], name: &str) -> DatabasePackage {
        DatabasePackage {
            os: "linux".into(),
            min_system: Some("2.28".into()),
            file_name: name.into(),
            server_program: "bin/servidor".into(),
            skip: vec!["testes".into()],
            ..package_for(bytes)
        }
    }

    #[test]
    fn a_tar_gz_package_is_extracted_without_its_skipped_folders() {
        let base = temp_dir("tar-gz");
        let bytes = tar_gz_with(
            &[
                ("pacote/bin/servidor", b"elf"),
                ("pacote/share/dados.txt", b"x"),
                ("pacote/testes/suite.pl", b"t"),
            ],
            &[],
        );
        let package = tar_package(&bytes, "teste.tar.gz");
        let archive = base.join("teste.tar.gz");
        fs::write(&archive, &bytes).unwrap();
        let folder = package.prepare(&archive, &base).unwrap();
        assert_eq!(fs::read(folder.join("bin/servidor")).unwrap(), b"elf");
        assert!(folder.join("share/dados.txt").is_file());
        assert!(!folder.join("testes").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(folder.join("bin/servidor"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "the server stays executable");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_tar_xz_package_is_decompressed_and_extracted() {
        let base = temp_dir("tar-xz");
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        tar.append_data(&mut header, "pacote/bin/servidor", &b"elf"[..])
            .unwrap();
        let tar = tar.into_inner().unwrap();
        let mut bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar[..], &mut bytes).unwrap();
        let package = tar_package(&bytes, "teste.tar.xz");
        let archive = base.join("teste.tar.xz");
        fs::write(&archive, &bytes).unwrap();
        let folder = package.prepare(&archive, &base).unwrap();
        assert_eq!(fs::read(folder.join("bin/servidor")).unwrap(), b"elf");
        // The decompressed tar is not left behind.
        let tars = fs::read_dir(&base)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tar"))
            .count();
        assert_eq!(tars, 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn links_may_only_point_inside_the_package() {
        let inside = |at: &str, to: &str| link_stays_inside(Path::new(at), Path::new(to));
        assert!(inside("lib/libx.so", "libx.so.24"));
        assert!(inside("lib/galera/libg.so", "../libg.so"));
        assert!(inside("mtr", "./run.pl"));
        assert!(!inside("lib/libx.so", "../../fora.so"));
        assert!(!inside("libx.so", "../fora.so"));
        assert!(!inside("lib/libx.so", "/etc/passwd"));
    }

    #[cfg(unix)]
    #[test]
    fn a_tar_with_a_link_out_of_the_package_leaves_nothing_behind() {
        let base = temp_dir("tar-link");
        let good = tar_gz_with(
            &[
                ("pacote/bin/servidor", b"elf"),
                ("pacote/lib/libx.so.1", b"so"),
            ],
            &[("pacote/lib/libx.so", "libx.so.1")],
        );
        let package = tar_package(&good, "bom.tar.gz");
        let archive = base.join("bom.tar.gz");
        fs::write(&archive, &good).unwrap();
        let folder = package.prepare(&archive, &base).unwrap();
        assert_eq!(fs::read(folder.join("lib/libx.so")).unwrap(), b"so");

        let bad = tar_gz_with(
            &[("pacote/bin/servidor", b"elf")],
            &[("pacote/lib/fora", "../../../../etc")],
        );
        let package = DatabasePackage {
            version: "0.0.2".into(),
            ..tar_package(&bad, "ruim.tar.gz")
        };
        let archive = base.join("ruim.tar.gz");
        fs::write(&archive, &bad).unwrap();
        assert!(matches!(
            package.prepare(&archive, &base),
            Err(PackageError::Archive(_))
        ));
        assert!(!package.folder(&base).exists());
        let _ = fs::remove_dir_all(&base);
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
