#[cfg(target_os = "linux")]
use std::{ffi::CString, fs::OpenOptions, io::Read as _, os::unix::ffi::OsStrExt as _, path::Path};

#[cfg(target_os = "linux")]
pub(crate) fn validate_libssh_gssapi_credential_cache(
    cache_name: Option<&std::ffi::OsStr>,
) -> Result<(), String> {
    let Some(cache_name) = cache_name else {
        return Ok(());
    };
    let raw = cache_name.as_bytes();
    let file_name = if let Some(file_name) = raw.strip_prefix(b"FILE:") {
        file_name
    } else if raw.contains(&b':') {
        return Ok(());
    } else {
        raw
    };
    if file_name.is_empty() {
        return Err(
            "GSSAPI authentication refused: Kerberos FILE credential cache path is empty"
                .to_string(),
        );
    }

    let path = Path::new(std::ffi::OsStr::from_bytes(file_name));
    let mut cache = match OpenOptions::new().read(true).open(path) {
        Ok(cache) => cache,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "GSSAPI authentication refused: Kerberos FILE credential cache cannot be opened: {error}"
            ));
        }
    };
    let metadata = cache.metadata().map_err(|error| {
        format!(
            "GSSAPI authentication refused: Kerberos FILE credential cache metadata is unavailable: {error}"
        )
    })?;
    if !metadata.is_file() {
        return Err(
            "GSSAPI authentication refused: Kerberos FILE credential cache is not a regular file"
                .to_string(),
        );
    }
    if metadata.len() < 16 {
        return Err(
            "GSSAPI authentication refused: Kerberos FILE credential cache is truncated"
                .to_string(),
        );
    }
    if let Some(validation) = validate_kerberos_file_cache(cache_name) {
        return validation;
    }

    let mut header = [0_u8; 2];
    cache.read_exact(&mut header).map_err(|_| {
        "GSSAPI authentication refused: Kerberos FILE credential cache is truncated".to_string()
    })?;
    if header[0] != 5 || !(1..=4).contains(&header[1]) {
        return Err(
            "GSSAPI authentication refused: Kerberos FILE credential cache header is invalid"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_kerberos_file_cache(cache_name: &std::ffi::OsStr) -> Option<Result<(), String>> {
    type Context = *mut std::ffi::c_void;
    type Cache = *mut std::ffi::c_void;
    type Principal = *mut std::ffi::c_void;
    type InitContext = unsafe extern "C" fn(*mut Context) -> libc::c_int;
    type ResolveCache =
        unsafe extern "C" fn(Context, *const libc::c_char, *mut Cache) -> libc::c_int;
    type GetPrincipal = unsafe extern "C" fn(Context, Cache, *mut Principal) -> libc::c_int;
    type FreePrincipal = unsafe extern "C" fn(Context, Principal);
    type CloseCache = unsafe extern "C" fn(Context, Cache) -> libc::c_int;
    type FreeContext = unsafe extern "C" fn(Context);

    let cache_name = match CString::new(cache_name.as_bytes()) {
        Ok(cache_name) => cache_name,
        Err(_) => {
            return Some(Err(
                "GSSAPI authentication refused: Kerberos FILE credential cache path contains NUL"
                    .to_string(),
            ));
        }
    };

    // GSSAPI-capable libssh already has a Kerberos runtime. Resolve it dynamically so
    // non-GSSAPI builds do not gain a link-time libkrb5 dependency.
    unsafe {
        let library = ["libkrb5.so.3", "libkrb5.so"]
            .into_iter()
            .find_map(|name| libloading::Library::new(name).ok())?;
        let init_context = *library.get::<InitContext>(b"krb5_init_context\0").ok()?;
        let resolve_cache = *library.get::<ResolveCache>(b"krb5_cc_resolve\0").ok()?;
        let get_principal = *library
            .get::<GetPrincipal>(b"krb5_cc_get_principal\0")
            .ok()?;
        let free_principal = *library
            .get::<FreePrincipal>(b"krb5_free_principal\0")
            .ok()?;
        let close_cache = *library.get::<CloseCache>(b"krb5_cc_close\0").ok()?;
        let free_context = *library.get::<FreeContext>(b"krb5_free_context\0").ok()?;

        let mut context: Context = std::ptr::null_mut();
        if init_context(&mut context) != 0 || context.is_null() {
            return Some(Err(
                "GSSAPI authentication refused: Kerberos credential context is unavailable"
                    .to_string(),
            ));
        }

        let mut cache: Cache = std::ptr::null_mut();
        if resolve_cache(context, cache_name.as_ptr(), &mut cache) != 0 || cache.is_null() {
            free_context(context);
            return Some(Err(
                "GSSAPI authentication refused: Kerberos FILE credential cache cannot be resolved"
                    .to_string(),
            ));
        }

        let mut principal: Principal = std::ptr::null_mut();
        let principal_status = get_principal(context, cache, &mut principal);
        if !principal.is_null() {
            free_principal(context, principal);
        }
        let _ = close_cache(context, cache);
        free_context(context);
        Some(if principal_status == 0 {
            Ok(())
        } else {
            Err(
                "GSSAPI authentication refused: Kerberos FILE credential cache is invalid or unreadable"
                    .to_string(),
            )
        })
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_libssh_gssapi_credential_cache(
    _cache_name: Option<&std::ffi::OsStr>,
) -> Result<(), String> {
    Ok(())
}
