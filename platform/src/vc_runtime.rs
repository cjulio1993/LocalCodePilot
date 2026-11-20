//! Whether the Microsoft Visual C++ runtime a program needs is on this computer, checked
//! without running the program (running it would open the Windows "DLL not found" box).
//!
//! The check compares the linker version in the program's PE header (for example 14.44)
//! with the file version of each runtime DLL in System32. It is a prior diagnosis, not a
//! proof: the first real start of the program confirms it.

use std::path::Path;

/// `(major, minor)` linker version from the PE optional header of `executable`.
pub fn linker_version(executable: &Path) -> Option<(u8, u8)> {
    let bytes = std::fs::read(executable).ok()?;
    linker_version_in(&bytes)
}

fn linker_version_in(bytes: &[u8]) -> Option<(u8, u8)> {
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let offset = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if bytes.get(offset..offset + 4)? != b"PE\0\0" {
        return None;
    }
    // Signature (4) + COFF header (20) = optional header; then Magic (2), Major, Minor.
    let optional = offset + 24;
    Some((*bytes.get(optional + 2)?, *bytes.get(optional + 3)?))
}

/// The runtime DLLs of `required` that are missing from System32, or older than the
/// linker that built `executable`, as messages for "Detalhes técnicos". Empty when all
/// are there (and always outside Windows).
pub fn missing_runtime(executable: &Path, required: &[&str]) -> Vec<String> {
    if !cfg!(windows) || required.is_empty() {
        return vec![];
    }
    let Some((major, minor)) = linker_version(executable) else {
        return vec![format!(
            "não foi possível ler a versão de {}",
            executable.display()
        )];
    };
    let Some(system) = std::env::var_os("SystemRoot").map(|root| Path::new(&root).join("System32"))
    else {
        return vec!["a pasta do Windows não foi encontrada".into()];
    };
    required
        .iter()
        .filter_map(|dll| {
            let path = system.join(dll);
            if !path.is_file() {
                return Some(format!("{dll} não está instalado"));
            }
            match file_version(&path) {
                Some((dll_major, dll_minor, _, _))
                    if (dll_major, dll_minor) >= (u16::from(major), u16::from(minor)) =>
                {
                    None
                }
                Some((dll_major, dll_minor, build, _)) => Some(format!(
                    "{dll} {dll_major}.{dll_minor}.{build} é anterior à {major}.{minor} exigida"
                )),
                None => Some(format!("não foi possível ler a versão de {dll}")),
            }
        })
        .collect()
}

#[cfg(windows)]
fn file_version(path: &Path) -> Option<(u16, u16, u16, u16)> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL terminated; the buffer has the size Windows asked for and
    // `info` points inside it while it lives.
    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0_u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let root: Vec<u16> = "\\".encode_utf16().chain(Some(0)).collect();
        let mut info: *mut c_void = ptr::null_mut();
        let mut length = 0_u32;
        if VerQueryValueW(
            buffer.as_ptr().cast(),
            root.as_ptr(),
            &mut info,
            &mut length,
        ) == 0
            || info.is_null()
            || (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let fixed = &*(info as *const VS_FIXEDFILEINFO);
        Some((
            (fixed.dwFileVersionMS >> 16) as u16,
            (fixed.dwFileVersionMS & 0xffff) as u16,
            (fixed.dwFileVersionLS >> 16) as u16,
            (fixed.dwFileVersionLS & 0xffff) as u16,
        ))
    }
}

#[cfg(not(windows))]
fn file_version(_: &Path) -> Option<(u16, u16, u16, u16)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe_with_linker(major: u8, minor: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x200];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x80 + 24 + 2] = major;
        bytes[0x80 + 24 + 3] = minor;
        bytes
    }

    #[test]
    fn reads_the_linker_version_of_a_pe_file() {
        assert_eq!(linker_version_in(&pe_with_linker(14, 44)), Some((14, 44)));
        assert_eq!(linker_version_in(b"not a program"), None);
        let mut broken = pe_with_linker(14, 44);
        broken[0x80] = b'X';
        assert_eq!(linker_version_in(&broken), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_runtime_of_this_test_is_installed() {
        // The test binary itself needs the C runtime the toolchain links.
        let me = std::env::current_exe().unwrap();
        assert_eq!(linker_version(&me).map(|(major, _)| major), Some(14));
        assert_eq!(
            missing_runtime(&me, &["esta-dll-nao-existe.dll"]),
            ["esta-dll-nao-existe.dll não está instalado"]
        );
    }
}
