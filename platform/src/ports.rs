use localcodepilot_core::ports::Port;
use std::{
    io::{ErrorKind, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    time::Duration,
};
#[cfg(target_os = "windows")]
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortOwner {
    pub process_id: u32,
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortAvailability {
    Available,
    Occupied { owner: Option<PortOwner> },
}

pub fn inspect_tcp_port(port: Port) -> PortAvailability {
    #[cfg(target_os = "windows")]
    if let Some(process_id) = windows_listener_process_id(port) {
        return PortAvailability::Occupied {
            owner: Some(PortOwner {
                process_id,
                process_name: process_name(process_id),
            }),
        };
    }

    let ipv4 = loopback_port_available(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let ipv6 = loopback_port_available(IpAddr::V6(Ipv6Addr::LOCALHOST), port);
    if matches!(ipv4, Some(false)) || matches!(ipv6, Some(false)) {
        PortAvailability::Occupied { owner: None }
    } else {
        PortAvailability::Available
    }
}

/// Whether `process_id` is `ancestor` or one of its descendants. A server started through a
/// shell (`cmd /C npm run dev`) listens from a child process, not from the one started.
pub fn process_descends_from(process_id: u32, ancestor: u32) -> bool {
    if process_id == ancestor {
        return true;
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, false);
    let mut current = Pid::from_u32(process_id);
    // Bounded: a corrupted process table must not loop forever.
    for _ in 0..64 {
        let Some(parent) = system.process(current).and_then(|process| process.parent()) else {
            return false;
        };
        if parent.as_u32() == ancestor {
            return true;
        }
        current = parent;
    }
    false
}

fn loopback_port_available(address: IpAddr, port: Port) -> Option<bool> {
    match TcpListener::bind((address, port.value())) {
        Ok(listener) => {
            drop(listener);
            Some(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::AddrNotAvailable | ErrorKind::Unsupported
            ) =>
        {
            None
        }
        Err(_) => Some(false),
    }
}

pub fn terminate_process_tree(process_id: u32) -> Result<(), String> {
    if process_id <= 4 || process_id == std::process::id() {
        return Err(
            "Esse processo é protegido e não pode ser encerrado pelo LocalCodePilot.".into(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = hidden_windows_command("taskkill");
        let status = command
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("Não foi possível executar taskkill: {error}"))?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("O Windows não permitiu encerrar o PID {process_id}."))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut system = System::new();
        let pid = Pid::from_u32(process_id);
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
        let process = system
            .process(pid)
            .ok_or_else(|| format!("O PID {process_id} não está mais em execução."))?;
        process
            .kill()
            .then_some(())
            .ok_or_else(|| format!("Não foi possível encerrar o PID {process_id}."))
    }
}

pub fn process_name(process_id: u32) -> Option<String> {
    let mut system = System::new();
    let pid = Pid::from_u32(process_id);
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
    system
        .process(pid)
        .map(|process| process.name().to_string_lossy().into_owned())
}

#[cfg(target_os = "windows")]
fn windows_listener_process_id(port: Port) -> Option<u32> {
    windows_tcp_listeners()
        .into_iter()
        .find(|(_, listener, _)| *listener == port.value())
        .map(|(_, _, process_id)| process_id)
}

/// Every TCP listener of this computer, from the system table (`GetExtendedTcpTable`), as
/// `(address, port, process id)`. `netstat` reads the same table but prints translated text
/// that was seen with wrong state labels and missing rows.
#[cfg(target_os = "windows")]
fn windows_tcp_listeners() -> Vec<(IpAddr, u16, u32)> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    /// One table: a `u32` row count followed by the rows, all 4-byte aligned.
    fn table<Row: Copy>(family: u16) -> Vec<Row> {
        let mut size = 0_u32;
        // SAFETY: a null buffer with a size of 0 only asks for the size the table needs.
        unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                u32::from(family),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        // The table may grow between calls.
        for _ in 0..3 {
            let mut buffer = vec![0_u32; (size as usize).div_ceil(4) + 1];
            let mut length = (buffer.len() * 4) as u32;
            // SAFETY: `length` is the real size of `buffer`, which is 4-byte aligned.
            let result = unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut length,
                    0,
                    u32::from(family),
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if result == ERROR_INSUFFICIENT_BUFFER {
                size = length;
                continue;
            }
            if result != 0 {
                return Vec::new();
            }
            let count = buffer[0] as usize;
            let row_words = std::mem::size_of::<Row>() / 4;
            if 1 + count * row_words > buffer.len() {
                return Vec::new();
            }
            let rows = buffer[1..].as_ptr().cast::<Row>();
            // SAFETY: the bounds were checked above and `Row` is plain data.
            return (0..count)
                .map(|index| unsafe { rows.add(index).read_unaligned() })
                .collect();
        }
        Vec::new()
    }

    let port = |raw: u32| u16::from_be((raw & 0xFFFF) as u16);
    let ipv4 = table::<MIB_TCPROW_OWNER_PID>(AF_INET)
        .into_iter()
        .map(|row| {
            (
                IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())),
                port(row.dwLocalPort),
                row.dwOwningPid,
            )
        });
    let ipv6 = table::<MIB_TCP6ROW_OWNER_PID>(AF_INET6)
        .into_iter()
        .map(|row| {
            (
                IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)),
                port(row.dwLocalPort),
                row.dwOwningPid,
            )
        });
    ipv4.chain(ipv6).collect()
}

#[cfg(target_os = "windows")]
fn hidden_windows_command(program: &str) -> Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// Whether something answering HTTP listens on `port` of this computer. Tools listen on
/// ports of their own (the Gradle daemon, language servers, debuggers); only a port that
/// answers like a web server can be the address of a site.
pub fn answers_http(port: Port) -> bool {
    let timeout = Duration::from_millis(1500);
    [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]
    .into_iter()
    .any(|address| {
        let Ok(mut stream) =
            TcpStream::connect_timeout(&SocketAddr::new(address, port.value()), timeout)
        else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
        if stream
            .write_all(b"HEAD / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut start = [0_u8; 5];
        stream.read_exact(&mut start).is_ok() && &start == b"HTTP/"
    })
}

/// Every process listening on `port` on this computer. A project and another program can
/// listen on the same port at once on Windows, for example one on `127.0.0.1` and the other
/// on `[::]`.
pub fn listener_process_ids(port: Port) -> Vec<u32> {
    let mut ids = local_listeners()
        .into_iter()
        .filter(|(listener, _)| *listener == port.value())
        .map(|(_, process_id)| process_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Ports on this computer where `root` or one of its descendants listens. Servers that do not
/// print their address (Express, Spring, Go, many Rust servers) are found this way.
pub fn listening_ports(root: u32) -> Vec<Port> {
    let listeners = local_listeners();
    if listeners.is_empty() {
        return Vec::new();
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, false);
    let descends = |process_id: u32| {
        let mut current = Pid::from_u32(process_id);
        // Bounded: a corrupted process table must not loop forever.
        for _ in 0..64 {
            if current.as_u32() == root {
                return true;
            }
            let Some(parent) = system.process(current).and_then(|process| process.parent()) else {
                return false;
            };
            current = parent;
        }
        false
    };
    let mut ports = listeners
        .into_iter()
        .filter(|(_, process_id)| descends(*process_id))
        .filter_map(|(port, _)| Port::new(port))
        .collect::<Vec<_>>();
    ports.sort_by_key(|port| port.value());
    ports.dedup();
    ports
}

/// TCP listeners reachable from this computer, as `(port, process id)`.
fn local_listeners() -> Vec<(u16, u32)> {
    #[cfg(target_os = "windows")]
    {
        windows_tcp_listeners()
            .into_iter()
            .filter(|(address, _, _)| address.is_unspecified() || address.is_loopback())
            .map(|(_, port, process_id)| (port, process_id))
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let lsof = std::process::Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
            .output();
        if let Ok(output) = lsof
            && output.status.success()
        {
            return lsof_listeners(&String::from_utf8_lossy(&output.stdout));
        }
        // Linux without lsof.
        std::process::Command::new("ss")
            .args(["-ltnpH"])
            .output()
            .map(|output| ss_listeners(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default()
    }
}

/// `host:port` on an address the browser reaches from this computer.
#[cfg(any(not(target_os = "windows"), test))]
fn local_endpoint_port(endpoint: &str) -> Option<u16> {
    let (host, port) = endpoint.rsplit_once(':')?;
    matches!(
        host,
        "0.0.0.0" | "127.0.0.1" | "*" | "localhost" | "[::]" | "[::1]" | "::" | "::1"
    )
    .then(|| port.parse().ok())
    .flatten()
}

/// `lsof -F pn`: a `p<pid>` line, then one `n<host>:<port>` line per listening socket.
#[cfg(any(not(target_os = "windows"), test))]
fn lsof_listeners(output: &str) -> Vec<(u16, u32)> {
    let mut process_id = None;
    let mut listeners = Vec::new();
    for line in output.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            process_id = pid.parse().ok();
        } else if let (Some(endpoint), Some(pid)) = (line.strip_prefix('n'), process_id)
            && let Some(port) = local_endpoint_port(endpoint)
        {
            listeners.push((port, pid));
        }
    }
    listeners
}

/// `ss -ltnpH`: `LISTEN 0 511 *:3000 *:* users:(("node",pid=123,fd=19))`.
#[cfg(any(not(target_os = "windows"), test))]
fn ss_listeners(output: &str) -> Vec<(u16, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            let port = local_endpoint_port(columns.get(3)?)?;
            let pid = line.split("pid=").nth(1)?;
            let pid = pid[..pid.find(|c: char| !c.is_ascii_digit())?]
                .parse()
                .ok()?;
            Some((port, pid))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_port_that_speaks_http_counts_as_a_site() {
        // A server that answers HTTP.
        let web = TcpListener::bind("127.0.0.1:0").unwrap();
        let web_port = Port::new(web.local_addr().unwrap().port()).unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = web.accept() {
                let mut request = [0_u8; 64];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        });
        assert!(answers_http(web_port));
        server.join().unwrap();

        // A tool port that accepts and closes without HTTP, like a daemon.
        let tool = TcpListener::bind("127.0.0.1:0").unwrap();
        let tool_port = Port::new(tool.local_addr().unwrap().port()).unwrap();
        let daemon = std::thread::spawn(move || {
            if let Ok((stream, _)) = tool.accept() {
                drop(stream);
            }
        });
        assert!(!answers_http(tool_port));
        daemon.join().unwrap();
    }

    #[test]
    fn listeners_from_lsof_and_ss_skip_other_interfaces() {
        let lsof = "p101\nf19\nn*:3000\nf20\nn[::1]:3001\np202\nf7\nn10.0.0.2:22\n";
        assert_eq!(lsof_listeners(lsof), [(3000, 101), (3001, 101)]);

        let ss = "\
LISTEN 0 511 *:3000 *:* users:((\"node\",pid=123,fd=19))
LISTEN 0 128 127.0.0.1:631 0.0.0.0:* users:((\"cupsd\",pid=9,fd=7))
LISTEN 0 128 10.0.0.2:22 0.0.0.0:* users:((\"sshd\",pid=8,fd=3))";
        assert_eq!(ss_listeners(ss), [(3000, 123), (631, 9)]);
    }

    #[test]
    fn finds_the_port_a_child_server_listens_on() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // This test process listens; it is its own root.
        let ports = listening_ports(std::process::id());
        assert!(
            ports.iter().any(|found| found.value() == port),
            "{port} not in {ports:?}"
        );
        drop(listener);
    }

    #[test]
    fn recognizes_a_child_process_and_not_an_unrelated_one() {
        let mut child = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/C", "ping", "-n", "3", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("sleep")
                .arg("2")
                .spawn()
                .unwrap()
        };
        let own = std::process::id();
        assert!(process_descends_from(child.id(), own));
        assert!(process_descends_from(own, own));
        assert!(!process_descends_from(own, child.id()));
        child.kill().ok();
        child.wait().ok();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_owner_of_a_port_comes_from_the_system_table() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = Port::new(listener.local_addr().unwrap().port()).unwrap();
        assert_eq!(windows_listener_process_id(port), Some(std::process::id()));
    }

    #[test]
    fn reports_a_bound_local_port_as_occupied() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(matches!(
            inspect_tcp_port(Port::new(port).unwrap()),
            PortAvailability::Occupied { .. }
        ));
    }

    #[test]
    fn reports_an_ipv6_loopback_listener_as_occupied() {
        let Ok(listener) = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)) else {
            return;
        };
        let port = listener.local_addr().unwrap().port();

        assert!(matches!(
            inspect_tcp_port(Port::new(port).unwrap()),
            PortAvailability::Occupied { .. }
        ));
    }
}
