use crate::context::config;
use crate::error::CliError;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// JSON file credential store: ~/.config/azure-boards/credentials.json,
/// created 0600 inside a 0700 directory. Used when no OS keychain exists.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn default_location() -> Self {
        Self {
            path: config::config_dir().join("credentials.json"),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_all(&self) -> Result<BTreeMap<String, String>, CliError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        self.check_permissions();
        let text = std::fs::read_to_string(&self.path)?;
        serde_json::from_str(&text).map_err(|e| {
            CliError::General(format!(
                "corrupt credential store {}: {e}",
                self.path.display()
            ))
        })
    }

    fn write_all(&self, map: &BTreeMap<String, String>) -> Result<(), CliError> {
        if let Some(parent) = self.path.parent() {
            config::create_private_dir(parent)?;
        }
        let text = serde_json::to_string_pretty(map)?;
        // Write then tighten permissions before anything can read it: create
        // with 0600 from the start on Unix.
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)?;
            f.write_all(text.as_bytes())?;
            // In case the file pre-existed with looser permissions.
            std::fs::set_permissions(&self.path, {
                use std::os::unix::fs::PermissionsExt;
                std::fs::Permissions::from_mode(0o600)
            })?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&self.path, text)?;
        }
        Ok(())
    }

    fn check_permissions(&self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&self.path) {
                let mode = meta.permissions().mode() & 0o777;
                if mode != 0o600 {
                    tracing::warn!(
                        "credential store {} has permissions {mode:o}; tightening to 600",
                        self.path.display()
                    );
                    let _ = std::fs::set_permissions(
                        &self.path,
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
            }
        }
    }

    pub fn load(&self, key: &str) -> Result<Option<String>, CliError> {
        Ok(self.read_all()?.get(key).cloned())
    }

    pub fn save(&self, key: &str, value: &str) -> Result<(), CliError> {
        let mut map = self.read_all()?;
        map.insert(key.to_string(), value.to_string());
        self.write_all(&map)
    }

    pub fn delete(&self, key: &str) -> Result<bool, CliError> {
        let mut map = self.read_all()?;
        let existed = map.remove(key).is_some();
        if existed {
            self.write_all(&map)?;
        }
        Ok(existed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (FileStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ab-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        (FileStore::at(path), dir)
    }

    #[test]
    fn roundtrip_and_permissions() {
        let (store, dir) = temp_store();
        store.save("entra", "{\"x\":1}").unwrap();
        assert_eq!(store.load("entra").unwrap().as_deref(), Some("{\"x\":1}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(store.delete("entra").unwrap());
        assert!(!store.delete("entra").unwrap());
        assert_eq!(store.load("entra").unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }
}
