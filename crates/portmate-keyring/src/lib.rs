use keyring_core::{set_default_store, Error, Result};

pub fn initialize_persistent_native_store() -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        set_default_store(dbus_secret_service_keyring_store::Store::new()?);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        set_default_store(apple_native_keyring_store::keychain::Store::new()?);
        return Ok(());
    }

    #[cfg(windows)]
    {
        set_default_store(windows_native_keyring_store::Store::new()?);
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(Error::NotSupportedByStore(
        "PortMate has no persistent native credential store for this platform".to_string(),
    ))
}
