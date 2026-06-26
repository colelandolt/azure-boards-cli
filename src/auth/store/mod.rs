pub mod file;

use crate::error::CliError;

pub const ENTRA_KEY: &str = "entra";
pub const PAT_KEY: &str = "pat";

/// Secret storage: OS keychain when available, 0600 file fallback otherwise
/// (the normal case on WSL2, where no secret-service daemon runs).
pub enum SecretStore {
    Keyring,
    File(file::FileStore),
}

const SERVICE: &str = "azure-boards";

impl SecretStore {
    /// Pick a backend. Probes the keyring once; PlatformFailure or
    /// NoStorageAccess selects the file fallback silently (logged at debug).
    pub fn open() -> Self {
        match keyring::Entry::new(SERVICE, "probe").and_then(|e| match e.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e),
        }) {
            Ok(()) => SecretStore::Keyring,
            Err(e) => {
                tracing::debug!("keyring unavailable ({e}); using file credential store");
                SecretStore::File(file::FileStore::default_location())
            }
        }
    }

    pub fn backend_name(&self) -> &'static str {
        match self {
            SecretStore::Keyring => "keychain (secret-service)",
            SecretStore::File(_) => "file (~/.config/azure-boards/credentials.json, mode 0600)",
        }
    }

    pub fn load(&self, key: &str) -> Result<Option<String>, CliError> {
        match self {
            SecretStore::Keyring => match keyring::Entry::new(SERVICE, key)
                .map_err(keyring_err)?
                .get_password()
            {
                Ok(v) => Ok(Some(v)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(keyring_err(e)),
            },
            SecretStore::File(f) => f.load(key),
        }
    }

    pub fn save(&self, key: &str, value: &str) -> Result<(), CliError> {
        match self {
            SecretStore::Keyring => keyring::Entry::new(SERVICE, key)
                .map_err(keyring_err)?
                .set_password(value)
                .map_err(keyring_err),
            SecretStore::File(f) => f.save(key, value),
        }
    }

    /// Returns true if an entry existed and was removed.
    pub fn delete(&self, key: &str) -> Result<bool, CliError> {
        match self {
            SecretStore::Keyring => match keyring::Entry::new(SERVICE, key)
                .map_err(keyring_err)?
                .delete_credential()
            {
                Ok(()) => Ok(true),
                Err(keyring::Error::NoEntry) => Ok(false),
                Err(e) => Err(keyring_err(e)),
            },
            SecretStore::File(f) => f.delete(key),
        }
    }
}

fn keyring_err(e: keyring::Error) -> CliError {
    CliError::General(format!("credential store: {e}"))
}
