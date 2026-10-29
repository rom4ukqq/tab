use anyhow::{bail, Result};
use argon2::{
    password_hash::{PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::path::{Path, PathBuf};

pub fn random_secret() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::STANDARD.encode(b)
}
pub fn random_pairing_code() -> String {
    use rand::Rng;
    format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32))
}
fn pin_bytes(pin: &str, pepper: &str) -> Vec<u8> {
    let mut h =
        Hmac::<Sha256>::new_from_slice(pepper.as_bytes()).expect("HMAC accepts arbitrary length");
    h.update(pin.as_bytes());
    h.finalize().into_bytes().to_vec()
}
pub fn hash_pin(pin: &str, pepper: &str) -> Result<String> {
    if !(4..=6).contains(&pin.len()) || !pin.bytes().all(|x| x.is_ascii_digit()) {
        bail!("PIN должен содержать 4–6 цифр")
    }
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let a = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 1, None).map_err(|_| anyhow::anyhow!("Invalid Argon2 parameters"))?,
    );
    a.hash_password(&pin_bytes(pin, pepper), &salt)
        .map(|h| h.to_string())
        .map_err(|_| anyhow::anyhow!("PIN hashing failed"))
}
pub fn verify_pin(pin: &str, pepper: &str, hash: &str) -> bool {
    use argon2::PasswordHash;
    (4..=6).contains(&pin.len())
        && pin.bytes().all(|x| x.is_ascii_digit())
        && PasswordHash::new(hash)
            .map(|h| {
                Argon2::default()
                    .verify_password(&pin_bytes(pin, pepper), &h)
                    .is_ok()
            })
            .unwrap_or(false)
}
pub fn safe_root(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Нужен абсолютный путь")
    }
    if !path.is_dir() {
        bail!("Каталог не существует")
    }
    reject_links(path)?;
    Ok(path.canonicalize()?)
}
pub fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let meta = std::fs::symlink_metadata(ancestor)?;
        if meta.file_type().is_symlink() {
            bail!("Symlink запрещён для этого пути")
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                bail!("Reparse point запрещён для этого пути")
            }
        }
    }
    Ok(())
}
pub fn safe_file(path: &Path, root: &Path, max: u64) -> Result<PathBuf> {
    reject_links(path)?;
    let p = path.canonicalize()?;
    let root = root.canonicalize()?;
    if !p.starts_with(root) || !p.is_file() {
        bail!("Файл вне разрешённого каталога")
    }
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.contains(':') || name.is_empty() {
        bail!("Недопустимое имя файла")
    }
    if p.metadata()?.len() > max {
        bail!("Файл превышает лимит")
    };
    Ok(p)
}

/// Deny concurrent replacement/write on Windows while reading the approved export.
/// Reparse parents are checked again after opening; this is not an OS sandbox for the CLI.
pub fn read_safe_file(path: &Path, root: &Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let canonical = safe_file(path, root, max)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(&canonical)?;
    reject_links(&canonical)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
        };
        let mut buffer = vec![0u16; 32768];
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let count = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        };
        if count == 0 || count as usize >= buffer.len() {
            bail!("Не удалось проверить final path")
        }
        let final_path = PathBuf::from(String::from_utf16(&buffer[..count as usize])?);
        if final_path != canonical || !final_path.starts_with(root.canonicalize()?) {
            bail!("Путь изменился во время открытия")
        }
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
            || info.nNumberOfLinks != 1
            || info.dwFileAttributes & 0x400 != 0
        {
            bail!("Reparse point / hard link не разрешён для экспорта")
        }
    }
    if !file.metadata()?.is_file() || file.metadata()?.len() > max {
        bail!("Неверный размер файла")
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bail!("Файл превышает лимит")
    };
    Ok(bytes)
}

#[cfg(windows)]
pub fn secret_get(name: &str) -> Result<Option<String>> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::*;
    let target: Vec<u16> = format!("AgentHub/{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let mut cred = std::ptr::null_mut();
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 {
            if GetLastError() == ERROR_NOT_FOUND {
                return Ok(None);
            }
            bail!("Credential Manager: чтение недоступно")
        }
        let bytes =
            std::slice::from_raw_parts((*cred).CredentialBlob, (*cred).CredentialBlobSize as usize);
        let result = String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(Into::into);
        CredFree(cred.cast());
        result
    }
}
#[cfg(windows)]
pub fn secret_set(name: &str, value: &str) -> Result<()> {
    use windows_sys::Win32::Security::Credentials::*;
    let mut target: Vec<u16> = format!("AgentHub/{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut bytes = value.as_bytes().to_vec();
    let result = unsafe {
        let mut c: CREDENTIALW = std::mem::zeroed();
        c.Type = CRED_TYPE_GENERIC;
        c.TargetName = target.as_mut_ptr();
        c.CredentialBlob = bytes.as_mut_ptr();
        c.CredentialBlobSize = bytes.len() as u32;
        c.Persist = CRED_PERSIST_LOCAL_MACHINE;
        CredWriteW(&c, 0)
    };
    use zeroize::Zeroize;
    bytes.zeroize();
    if result == 0 {
        bail!("Credential Manager: запись недоступна")
    };
    Ok(())
}
#[cfg(not(windows))]
pub fn secret_get(_: &str) -> Result<Option<String>> {
    bail!("Этот backend секретов доступен только в Windows")
}
#[cfg(not(windows))]
pub fn secret_set(_: &str, _: &str) -> Result<()> {
    bail!("Этот backend секретов доступен только в Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin_is_peppered() {
        let h = hash_pin("123456", "one").unwrap();
        assert!(verify_pin("123456", "one", &h));
        assert!(!verify_pin("123456", "two", &h));
        assert!(!verify_pin("654321", "one", &h));
        assert!(hash_pin("12ab", "one").is_err());
    }
    #[test]
    fn rejects_escape() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::NamedTempFile::new().unwrap();
        assert!(safe_file(other.path(), root.path(), 1024).is_err());
    }
}
