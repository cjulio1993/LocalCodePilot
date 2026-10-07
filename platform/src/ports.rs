use localcodepilot_core::ports::Port;
use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener},
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

#[cfg(target_os = "windows")]
fn process_name(process_id: u32) -> Option<String> {
    let mut system = System::new();
    let pid = Pid::from_u32(process_id);
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
    system
        .process(pid)
        .map(|process| process.name().to_string_lossy().into_owned())
}

#[cfg(target_os = "windows")]
fn windows_listener_process_id(port: Port) -> Option<u32> {
    let output = hidden_windows_command("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
        .ok()?;
    listener_process_id_from_netstat(&String::from_utf8_lossy(&output.stdout), port)
}

#[cfg(target_os = "windows")]
fn hidden_windows_command(program: &str) -> Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(any(target_os = "windows", test))]
fn listener_process_id_from_netstat(output: &str, port: Port) -> Option<u32> {
    output.lines().find_map(|line| {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() < 5 || !columns[0].eq_ignore_ascii_case("TCP") {
            return None;
        }
        let state = columns[3].to_ascii_uppercase();
        let is_listening =
            state.contains("LISTEN") || state.contains("ESCUT") || state.contains("OUVIN");
        if !is_listening || endpoint_port(columns[1]) != Some(port.value()) {
            return None;
        }
        columns.last()?.parse().ok()
    })
}

#[cfg(any(target_os = "windows", test))]
fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint.rsplit_once(':')?.1.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_owner_of_ipv4_and_ipv6_listeners() {
        let output = "\
  TCP    0.0.0.0:3306       0.0.0.0:0       LISTENING       5952\n\
  TCP    [::]:5173          [::]:0          LISTENING       8420\n\
  TCP    127.0.0.1:5173     127.0.0.1:60000 ESTABLISHED     9999";

        assert_eq!(
            listener_process_id_from_netstat(output, Port::new(3306).unwrap()),
            Some(5952)
        );
        assert_eq!(
            listener_process_id_from_netstat(output, Port::new(5173).unwrap()),
            Some(8420)
        );
    }

    #[test]
    fn ignores_connections_that_are_not_listening() {
        let output = "TCP 127.0.0.1:5173 127.0.0.1:60000 ESTABLISHED 9999";

        assert_eq!(
            listener_process_id_from_netstat(output, Port::new(5173).unwrap()),
            None
        );
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
