//! Local update feed for end-to-end tests of the in-app update.
//!
//! Serves, on 127.0.0.1 only, a folder laid out like a GitHub release:
//!
//! - `GET /releases` returns `<dir>/releases.json`;
//! - `GET /download/<tag>/<file>` returns `<dir>/download/<tag>/<file>`.
//!
//! A test build of the app compiled with the `update-test-feed` feature uses it
//! when `LOCALCODEPILOT_TEST_FEED=http://127.0.0.1:<port>`. See
//! `scripts/e2e-update-windows.ps1`.
//!
//! ```text
//! cargo run -p localcodepilot-desktop --example update_test_feed -- \
//!     --dir <folder> [--port 0] [--port-file <file>] \
//!     [--drop-after-bytes <n>] [--bytes-per-second <n>]
//! ```
//!
//! `--drop-after-bytes` sends that many bytes of an `.exe` and then stops the
//! whole server, as if it went down in the middle of the download.
//! `--bytes-per-second` slows `.exe` downloads so the progress can be watched.

use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Options {
    directory: PathBuf,
    port: u16,
    port_file: Option<PathBuf>,
    drop_after_bytes: Option<u64>,
    bytes_per_second: Option<u64>,
}

fn main() {
    let options = match parse_options(env::args().skip(1)) {
        Ok(options) => Arc::new(options),
        Err(error) => {
            eprintln!("update_test_feed: {error}");
            process::exit(2);
        }
    };
    let listener = TcpListener::bind(("127.0.0.1", options.port)).unwrap_or_else(|error| {
        eprintln!(
            "update_test_feed: cannot listen on 127.0.0.1:{}: {error}",
            options.port
        );
        process::exit(1);
    });
    let port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or(options.port);
    if let Some(port_file) = &options.port_file {
        // Written last and renamed, so a reader never sees a partial number.
        let partial = port_file.with_extension("part");
        if fs::write(&partial, port.to_string())
            .and_then(|()| fs::rename(&partial, port_file))
            .is_err()
        {
            eprintln!("update_test_feed: cannot write {}", port_file.display());
            process::exit(1);
        }
    }
    println!(
        "update_test_feed: serving {} on http://127.0.0.1:{port}",
        options.directory.display()
    );
    for stream in listener.incoming().flatten() {
        let options = Arc::clone(&options);
        thread::spawn(move || {
            if let Err(error) = handle(stream, &options) {
                eprintln!("update_test_feed: {error}");
            }
        });
    }
}

fn parse_options(mut arguments: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut directory = None;
    let mut port = 0;
    let mut port_file = None;
    let mut drop_after_bytes = None;
    let mut bytes_per_second = None;
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--dir" => directory = Some(PathBuf::from(value()?)),
            "--port" => port = value()?.parse().map_err(|_| "invalid --port")?,
            "--port-file" => port_file = Some(PathBuf::from(value()?)),
            "--drop-after-bytes" => {
                drop_after_bytes = Some(value()?.parse().map_err(|_| "invalid --drop-after-bytes")?)
            }
            "--bytes-per-second" => {
                bytes_per_second = Some(
                    value()?
                        .parse()
                        .ok()
                        .filter(|rate| *rate > 0)
                        .ok_or("invalid --bytes-per-second")?,
                )
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let directory = directory.ok_or("--dir is required")?;
    if !directory.is_dir() {
        return Err(format!("{} is not a folder", directory.display()));
    }
    Ok(Options {
        directory,
        port,
        port_file,
        drop_after_bytes,
        bytes_per_second,
    })
}

fn handle(stream: TcpStream, options: &Options) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| error.to_string())?;
    loop {
        let mut header = String::new();
        let read = reader
            .read_line(&mut header)
            .map_err(|error| error.to_string())?;
        if read == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    println!("update_test_feed: {method} {target}");
    let mut stream = stream;
    if method != "GET" {
        return respond_status(&mut stream, "405 Method Not Allowed");
    }
    let Some(path) = resolve(&options.directory, target) else {
        return respond_status(&mut stream, "404 Not Found");
    };
    let Ok(mut file) = File::open(&path) else {
        return respond_status(&mut stream, "404 Not Found");
    };
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let content_type = if target == "/releases" {
        "application/json"
    } else {
        "application/octet-stream"
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| error.to_string())?;
    let executable = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    send_body(&mut file, &mut stream, executable, options)
}

/// Maps the request to a file inside the served folder; anything that could
/// leave it (`..`, absolute paths, odd characters) is refused.
fn resolve(directory: &Path, target: &str) -> Option<PathBuf> {
    if target == "/releases" {
        return Some(directory.join("releases.json"));
    }
    let relative = target.strip_prefix("/download/")?;
    if relative.is_empty()
        || !relative
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/'))
    {
        return None;
    }
    let relative = Path::new(relative);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(directory.join("download").join(relative))
}

fn send_body(
    file: &mut File,
    stream: &mut TcpStream,
    executable: bool,
    options: &Options,
) -> Result<(), String> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut sent = 0_u64;
    let started = Instant::now();
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            return stream.flush().map_err(|error| error.to_string());
        }
        let mut chunk = &buffer[..read];
        if executable && let Some(limit) = options.drop_after_bytes {
            let allowed = limit.saturating_sub(sent).min(chunk.len() as u64) as usize;
            chunk = &chunk[..allowed];
            if chunk.is_empty() {
                let _ = stream.flush();
                println!("update_test_feed: stopping after {sent} bytes (--drop-after-bytes)");
                process::exit(0);
            }
        }
        stream.write_all(chunk).map_err(|error| error.to_string())?;
        sent += chunk.len() as u64;
        if executable && let Some(rate) = options.bytes_per_second {
            let due = Duration::from_secs_f64(sent as f64 / rate as f64);
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                thread::sleep(wait);
            }
        }
    }
}

fn respond_status(stream: &mut TcpStream, status: &str) -> Result<(), String> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| error.to_string())
}
