use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::Aead};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

mod key_file;

const SEALED_PREFIX: &str = "flamel:v1:";

#[derive(Clone)]
pub struct CredentialVault {
    key: [u8; 32],
}

impl CredentialVault {
    pub fn load() -> Result<Self, String> {
        let secret = if let Ok(value) = std::env::var("FLAMEL_VAULT_MASTER_KEY")
            && !value.trim().is_empty()
        {
            if value.trim().len() < 24 {
                return Err("FLAMEL_VAULT_MASTER_KEY must contain at least 24 characters".into());
            }
            value
        } else {
            let path = std::env::var_os("FLAMEL_VAULT_KEY_PATH")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("FLAMEL_HOME")
                        .map(PathBuf::from)
                        .map(|home| home.join(".secrets").join("vault.key"))
                })
                .unwrap_or_else(|| {
                    dirs::config_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join("@flamel")
                        .join("vault.key")
                });
            key_file::load_or_create(&path)?
        };
        let digest = Sha256::digest(secret.as_bytes());
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Ok(Self { key })
    }
    pub fn encrypt(&self, value: &str) -> Result<String, String> {
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let nonce_bytes = rand::random::<[u8; 12]>();
        let encrypted = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), value.as_bytes())
            .map_err(|_| "Credential encryption failed".to_string())?;
        let mut output = nonce_bytes.to_vec();
        output.extend(encrypted);
        Ok(STANDARD.encode(output))
    }
    pub fn decrypt(&self, value: &str) -> Result<String, String> {
        let raw = STANDARD
            .decode(value)
            .map_err(|_| "Credential is corrupt".to_string())?;
        if raw.len() < 13 {
            return Err("Credential is corrupt".into());
        }
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let plain = cipher
            .decrypt(Nonce::from_slice(&raw[..12]), &raw[12..])
            .map_err(|_| "Credential could not be decrypted".to_string())?;
        String::from_utf8(plain).map_err(|_| "Credential is not valid UTF-8".into())
    }

    /// Encrypt a database credential with an explicit format marker.
    pub fn seal(&self, value: &str) -> Result<String, String> {
        self.encrypt(value)
            .map(|encrypted| format!("{SEALED_PREFIX}{encrypted}"))
    }

    /// Open a database credential. Untagged values are legacy plaintext and are
    /// accepted only so startup can migrate existing installations atomically.
    pub fn open(&self, value: &str) -> Result<String, String> {
        match value.strip_prefix(SEALED_PREFIX) {
            Some(encrypted) => self.decrypt(encrypted),
            None => Ok(value.to_string()),
        }
    }

    fn is_sealed(value: &str) -> bool {
        value.starts_with(SEALED_PREFIX)
    }

    /// Seal every legacy provider credential in one transaction. Already sealed
    /// values are authenticated before startup continues, which prevents a wrong
    /// vault key or corrupt database from being mistaken for missing credentials.
    pub fn migrate_database_credentials(&self, db: &crate::db::Db) -> Result<usize, String> {
        let mut conn = db.conn();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut migrated = 0usize;

        let provider_rows = {
            let mut stmt = tx
                .prepare(
                    "SELECT user_id, provider, access_token, refresh_token, id_token FROM provider_credentials",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        for (user_id, provider, access, refresh, id_token) in provider_rows {
            let access = self.migrate_value(&access, &mut migrated)?;
            let refresh = self.migrate_optional(refresh, &mut migrated)?;
            let id_token = self.migrate_optional(id_token, &mut migrated)?;
            tx.execute(
                "UPDATE provider_credentials SET access_token=?3, refresh_token=?4, id_token=?5 WHERE user_id=?1 AND provider=?2",
                rusqlite::params![user_id, provider, access, refresh, id_token],
            )
            .map_err(|e| e.to_string())?;
        }

        let claude_rows = {
            let mut stmt = tx
                .prepare("SELECT user_id, access_token, refresh_token FROM claude_credentials")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        for (user_id, access, refresh) in claude_rows {
            let access = self.migrate_value(&access, &mut migrated)?;
            let refresh = self.migrate_optional(refresh, &mut migrated)?;
            tx.execute(
                "UPDATE claude_credentials SET access_token=?2, refresh_token=?3 WHERE user_id=?1",
                rusqlite::params![user_id, access, refresh],
            )
            .map_err(|e| e.to_string())?;
        }

        let git_rows = {
            let mut stmt = tx
                .prepare("SELECT id, token FROM git_accounts")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        for (id, token) in git_rows {
            let token = self.migrate_value(&token, &mut migrated)?;
            tx.execute(
                "UPDATE git_accounts SET token=?2 WHERE id=?1",
                rusqlite::params![id, token],
            )
            .map_err(|e| e.to_string())?;
        }

        let capability_values = {
            let mut stmt = tx
                .prepare("SELECT encrypted_value FROM capability_credentials")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for encrypted in capability_values {
            self.decrypt(&encrypted)?;
        }

        let tagged_settings = {
            let mut stmt = tx
                .prepare(
                    "SELECT key, value FROM app_settings WHERE key IN ('anthropic_api_key', 'mcp_token')",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };
        for (key, value) in tagged_settings {
            let value = self.migrate_value(&value, &mut migrated)?;
            tx.execute(
                "UPDATE app_settings SET value=?2 WHERE key=?1",
                rusqlite::params![key, value],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(migrated)
    }

    pub fn migrate_value(&self, value: &str, migrated: &mut usize) -> Result<String, String> {
        if Self::is_sealed(value) {
            self.open(value)?;
            Ok(value.to_string())
        } else {
            *migrated += 1;
            self.seal(value)
        }
    }

    pub fn migrate_optional(
        &self,
        value: Option<String>,
        migrated: &mut usize,
    ) -> Result<Option<String>, String> {
        value
            .map(|value| self.migrate_value(&value, migrated))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let vault = CredentialVault { key: [7; 32] };
        let encrypted = vault.encrypt("secret").unwrap();
        assert!(!encrypted.contains("secret"));
        assert_eq!(vault.decrypt(&encrypted).unwrap(), "secret");
    }

    #[test]
    fn sealed_values_are_versioned_and_legacy_plaintext_still_opens() {
        let vault = CredentialVault { key: [7; 32] };
        let sealed = vault.seal("secret").unwrap();
        assert!(sealed.starts_with(SEALED_PREFIX));
        assert_eq!(vault.open(&sealed).unwrap(), "secret");
        assert_eq!(vault.open("legacy-secret").unwrap(), "legacy-secret");
    }

    #[test]
    fn local_secrets_are_encrypted_and_user_scoped() {
        let db = crate::db::Db::open(":memory:");
        let vault = CredentialVault { key: [7; 32] };
        crate::db::settings::set_local_secret(&db, &vault, "user-1", "sudo", "hunter2").unwrap();

        let stored: String = db
            .conn()
            .query_row(
                "SELECT value FROM app_settings WHERE key='local_secret.user-1.sudo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored.starts_with(SEALED_PREFIX));
        assert!(!stored.contains("hunter2"));
        assert_eq!(
            crate::db::settings::get_local_secret(&db, &vault, "user-1", "sudo").unwrap(),
            Some("hunter2".into())
        );
        assert_eq!(
            crate::db::settings::get_local_secret(&db, &vault, "user-2", "sudo").unwrap(),
            None
        );
    }

    #[test]
    fn corrupt_sealed_values_are_rejected() {
        let vault = CredentialVault { key: [7; 32] };
        assert!(vault.open("flamel:v1:not-valid-ciphertext").is_err());
    }

    #[test]
    fn database_migration_is_atomic_and_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "flamel-credential-migration-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = crate::db::Db::open(path.to_str().unwrap());
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO provider_credentials (user_id, provider, access_token, refresh_token, id_token, expires_at, scopes, updated_at) VALUES ('user', 'codex', 'access-plain', 'refresh-plain', 'id-plain', 1, '', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO claude_credentials (user_id, access_token, refresh_token, expires_at, scopes, updated_at) VALUES ('user', 'claude-plain', 'claude-refresh-plain', 1, '', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO git_accounts (id, type, label, base_url, token, created_at) VALUES ('git', 'github', 'GitHub', 'https://api.github.com', 'git-plain', 'now')",
                [],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES ('mcp_token', 'mcp-plain')",
                [],
            )
            .unwrap();
        }

        let vault = CredentialVault { key: [7; 32] };
        assert_eq!(vault.migrate_database_credentials(&db).unwrap(), 7);
        assert_eq!(vault.migrate_database_credentials(&db).unwrap(), 0);

        let conn = db.conn();
        let provider: (String, String, String) = conn
            .query_row(
                "SELECT access_token, refresh_token, id_token FROM provider_credentials",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(vault.open(&provider.0).unwrap(), "access-plain");
        assert_eq!(vault.open(&provider.1).unwrap(), "refresh-plain");
        assert_eq!(vault.open(&provider.2).unwrap(), "id-plain");
        assert!(provider.0.starts_with(SEALED_PREFIX));

        let claude: (String, String) = conn
            .query_row(
                "SELECT access_token, refresh_token FROM claude_credentials",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(vault.open(&claude.0).unwrap(), "claude-plain");
        assert_eq!(vault.open(&claude.1).unwrap(), "claude-refresh-plain");

        let git: String = conn
            .query_row("SELECT token FROM git_accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(vault.open(&git).unwrap(), "git-plain");

        let mcp: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key='mcp_token'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(vault.open(&mcp).unwrap(), "mcp-plain");
        assert_ne!(provider.0, "access-plain");
        assert_ne!(provider.1, "refresh-plain");
        assert_ne!(provider.2, "id-plain");
        assert_ne!(claude.0, "claude-plain");
        assert_ne!(claude.1, "claude-refresh-plain");
        assert_ne!(git, "git-plain");

        assert_ne!(mcp, "mcp-plain");
        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
