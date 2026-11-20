//! The password vault: Windows Credential Manager (generic credentials, encrypted by
//! Windows with the user's login through DPAPI). The app never writes its own crypto.
//!
//! Requirements agreed in `.internal/conversas/2026-11-20-t019-02-servidor-gerenciado.md`:
//! a secret is written only when its name is new (never overwritten), read back and
//! compared before anything depends on it, and kept only for this user on this computer.
//! The vault protects the stored secret; other programs running as the same user can
//! still read it, as with any system vault.
//!
//! Other systems have no vault in this version: every call fails with a message.

/// Saves `secret` under a new `target`. Fails if the name already exists, so a secret
/// is never replaced, and reads it back to confirm it is stored.
pub fn save_new(target: &str, user: &str, secret: &str) -> Result<(), String> {
    if read(target)?.is_some() {
        return Err(format!(
            "já existe uma credencial com o nome {target}; nada foi alterado"
        ));
    }
    imp::write(target, user, secret)?;
    match read(target)? {
        Some(stored) if stored == secret => Ok(()),
        _ => {
            let _ = imp::delete(target);
            Err("a credencial gravada não pôde ser conferida".into())
        }
    }
}

/// The secret stored under `target`, or `None` when there is none.
pub fn read(target: &str) -> Result<Option<String>, String> {
    imp::read(target)
}

/// Removes `target`. A missing name is not an error.
pub fn delete(target: &str) -> Result<(), String> {
    imp::delete(target)
}

#[cfg(windows)]
mod imp {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, GetLastError},
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
            CredReadW, CredWriteW,
        },
    };

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn write(target: &str, user: &str, secret: &str) -> Result<(), String> {
        let mut target = wide(target);
        let mut user = wide(user);
        let mut blob = secret.as_bytes().to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            UserName: user.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            // This user, this computer: not copied to a roaming profile.
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: every pointer refers to a buffer that lives until the call returns.
        let written = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        if written == 0 {
            return Err(format!(
                "o Gerenciador de Credenciais recusou a gravação (erro {})",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }

    pub(super) fn read(target: &str) -> Result<Option<String>, String> {
        let target = wide(target);
        let mut credential: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: `target` is NUL terminated; Windows allocates `credential`, freed below.
        let found = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if found == 0 {
            let error = unsafe { GetLastError() };
            return if error == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(format!(
                    "o Gerenciador de Credenciais não pôde ser lido (erro {error})"
                ))
            };
        }
        // SAFETY: on success `credential` points to a valid CREDENTIALW until CredFree.
        let secret = unsafe {
            let entry = &*credential;
            let bytes = if entry.CredentialBlob.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(entry.CredentialBlob, entry.CredentialBlobSize as usize)
            };
            let secret = String::from_utf8(bytes.to_vec());
            CredFree(credential.cast());
            secret
        };
        secret
            .map(Some)
            .map_err(|_| "a credencial guardada não é texto válido".into())
    }

    pub(super) fn delete(target: &str) -> Result<(), String> {
        let target = wide(target);
        // SAFETY: `target` is NUL terminated.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_NOT_FOUND {
                return Err(format!(
                    "o Gerenciador de Credenciais não pôde apagar (erro {error})"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    const UNSUPPORTED: &str = "o cofre de senhas só existe no Windows nesta versão";

    pub(super) fn write(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub(super) fn read(_: &str) -> Result<Option<String>, String> {
        Err(UNSUPPORTED.into())
    }

    pub(super) fn delete(_: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_saved_once_read_back_and_never_overwritten() {
        let target = format!("LocalCodePilot/teste/{}-{}", std::process::id(), line!());
        let _ = delete(&target);
        assert_eq!(read(&target).unwrap(), None);
        save_new(&target, "usuario-teste", "segredo-1").unwrap();
        assert_eq!(read(&target).unwrap().as_deref(), Some("segredo-1"));
        // A second save under the same name is refused and keeps the first secret.
        assert!(save_new(&target, "usuario-teste", "segredo-2").is_err());
        assert_eq!(read(&target).unwrap().as_deref(), Some("segredo-1"));
        delete(&target).unwrap();
        assert_eq!(read(&target).unwrap(), None);
        // Deleting what is not there is fine.
        delete(&target).unwrap();
    }
}
