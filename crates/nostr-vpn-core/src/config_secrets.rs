use std::path::Path;

use anyhow::Result;

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretPersistence {
    Platform,
    Plaintext,
}

pub(crate) fn hydrate_config_secrets(path: &Path, config: &mut AppConfig) -> Result<()> {
    platform::hydrate_config_secrets(path, config)
}

pub(crate) fn prepare_config_secrets_for_save(
    path: &Path,
    config: &mut AppConfig,
    persistence: SecretPersistence,
) -> Result<()> {
    if persistence == SecretPersistence::Plaintext {
        return Ok(());
    }
    platform::prepare_config_secrets_for_save(path, config)
}

#[derive(Debug, Clone, Copy)]
#[cfg(target_os = "macos")]
enum ConfigSecret {
    Nostr,
    WireGuardExitPrivate,
    WireGuardExitPeerPreshared,
}

#[cfg(target_os = "macos")]
impl ConfigSecret {
    fn account_suffix(self) -> &'static str {
        match self {
            Self::Nostr => "nostr-secret-key",
            Self::WireGuardExitPrivate => "wireguard-exit-private-key",
            Self::WireGuardExitPeerPreshared => "wireguard-exit-peer-preshared-key",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Nostr => "Nostr secret key",
            Self::WireGuardExitPrivate => "WireGuard exit private key",
            Self::WireGuardExitPeerPreshared => "WireGuard exit peer preshared key",
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use anyhow::{Context, Result, anyhow};
    use security_framework::os::macos::keychain::SecKeychain;
    use sha2::{Digest as _, Sha256};

    use super::ConfigSecret;
    use crate::config::{AppConfig, normalize_nostr_pubkey};

    const REDACTED_SECRET_MARKER: &str = "stored-in-system-keychain";
    const SERVICE: &str = "to.nostrvpn.nvpn.config-secrets";
    const SYSTEM_KEYCHAIN: &str = "/Library/Keychains/System.keychain";
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub(super) fn hydrate_config_secrets(path: &Path, config: &mut AppConfig) -> Result<()> {
        if is_redacted_secret(&config.nostr.secret_key) {
            config.nostr.secret_key = read_required_secret(path, ConfigSecret::Nostr)?;
        } else if config.nostr.secret_key.trim().is_empty()
            && normalize_nostr_pubkey(&config.nostr.public_key).is_ok()
            && let Some(value) = read_secret(path, ConfigSecret::Nostr)?
        {
            config.nostr.secret_key = value;
        }
        if is_redacted_secret(&config.wireguard_exit.private_key) {
            config.wireguard_exit.private_key =
                read_required_secret(path, ConfigSecret::WireGuardExitPrivate)?;
        }
        if is_redacted_secret(&config.wireguard_exit.peer_preshared_key) {
            config.wireguard_exit.peer_preshared_key =
                read_required_secret(path, ConfigSecret::WireGuardExitPeerPreshared)?;
        }

        if config.nostr.secret_key.trim().is_empty()
            && normalize_nostr_pubkey(&config.nostr.public_key).is_ok()
        {
            return Err(anyhow!(
                "config {} references a Nostr public key but its secret key is missing from the System Keychain",
                path.display()
            ));
        }

        Ok(())
    }

    pub(super) fn prepare_config_secrets_for_save(
        path: &Path,
        config: &mut AppConfig,
    ) -> Result<()> {
        persist_field(path, ConfigSecret::Nostr, &mut config.nostr.secret_key)?;
        persist_field(
            path,
            ConfigSecret::WireGuardExitPrivate,
            &mut config.wireguard_exit.private_key,
        )?;
        persist_field(
            path,
            ConfigSecret::WireGuardExitPeerPreshared,
            &mut config.wireguard_exit.peer_preshared_key,
        )
    }

    fn persist_field(path: &Path, kind: ConfigSecret, value: &mut String) -> Result<()> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            let _ = delete_secret(path, kind);
            return Ok(());
        }
        if is_redacted_secret(trimmed) {
            return Ok(());
        }

        match write_secret(path, kind, trimmed) {
            Ok(()) => {
                *value = REDACTED_SECRET_MARKER.to_string();
                Ok(())
            }
            Err(write_error) => match read_secret(path, kind) {
                Ok(Some(existing)) if existing == trimmed => {
                    *value = REDACTED_SECRET_MARKER.to_string();
                    Ok(())
                }
                Ok(Some(_)) => Err(write_error).with_context(|| {
                    format!(
                        "{} changed but updating the System Keychain requires administrator privileges",
                        kind.display_name()
                    )
                }),
                Ok(None) | Err(_) => Ok(()),
            },
        }
    }

    fn read_required_secret(path: &Path, kind: ConfigSecret) -> Result<String> {
        read_secret(path, kind)?.ok_or_else(|| {
            anyhow!(
                "{} is marked as stored in the System Keychain, but no matching keychain item exists",
                kind.display_name()
            )
        })
    }

    fn read_secret(path: &Path, kind: ConfigSecret) -> Result<Option<String>> {
        let keychain = system_keychain()?;
        let account = account_name(path, kind);
        match keychain.find_generic_password(SERVICE, &account) {
            Ok((password, _item)) => {
                let bytes = password.as_ref().to_vec();
                let value = String::from_utf8(bytes).with_context(|| {
                    format!(
                        "{} in the System Keychain is not valid UTF-8",
                        kind.display_name()
                    )
                })?;
                Ok(Some(value))
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to read {} from the System Keychain",
                    kind.display_name()
                )
            }),
        }
    }

    fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        let keychain = system_keychain()?;
        let account = account_name(path, kind);
        match keychain.find_generic_password(SERVICE, &account) {
            Ok((_password, item)) => {
                item.delete();
                Ok(())
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to delete {} from the System Keychain",
                    kind.display_name()
                )
            }),
        }
    }

    fn write_secret(path: &Path, kind: ConfigSecret, value: &str) -> Result<()> {
        let keychain = system_keychain()?;
        let account = account_name(path, kind);
        keychain
            .set_generic_password(SERVICE, &account, value.as_bytes())
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!(
                    "failed to write {} to the System Keychain",
                    kind.display_name()
                )
            })
    }

    fn system_keychain() -> Result<SecKeychain> {
        SecKeychain::open(SYSTEM_KEYCHAIN)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("failed to open {SYSTEM_KEYCHAIN}"))
    }

    fn account_name(path: &Path, kind: ConfigSecret) -> String {
        format!("{}:{}", config_scope(path), kind.account_suffix())
    }

    fn config_scope(path: &Path) -> String {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_os_str().as_bytes());
        hex::encode(hasher.finalize())
    }

    fn is_redacted_secret(value: &str) -> bool {
        value.trim() == REDACTED_SECRET_MARKER
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::Path;

    use anyhow::Result;

    use crate::config::AppConfig;

    pub(super) fn hydrate_config_secrets(_path: &Path, _config: &mut AppConfig) -> Result<()> {
        Ok(())
    }

    pub(super) fn prepare_config_secrets_for_save(
        _path: &Path,
        _config: &mut AppConfig,
    ) -> Result<()> {
        Ok(())
    }
}
