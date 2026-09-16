use std::{fs, path::Path};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    models::BackupData,
};

const FORMAT: &str = "northstar-portable-backup";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct BackupEnvelope {
    format: String,
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn write(path: &Path, password: &str, data: &BackupData) -> AppResult<()> {
    validate_password(password)?;
    let plaintext =
        serde_json::to_vec(data).map_err(|error| AppError::Internal(error.to_string()))?;
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::Encryption)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| AppError::Encryption)?;
    let envelope = BackupEnvelope {
        format: FORMAT.into(),
        version: VERSION,
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    };
    let encoded = serde_json::to_vec_pretty(&envelope)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    fs::write(path, encoded)?;
    Ok(())
}

pub fn read(path: &Path, password: &str) -> AppResult<BackupData> {
    validate_password(password)?;
    let encoded = fs::read(path)?;
    let envelope: BackupEnvelope = serde_json::from_slice(&encoded)
        .map_err(|_| AppError::InvalidInput("This is not a valid Northstar backup".into()))?;
    if envelope.format != FORMAT || envelope.version != VERSION {
        return Err(AppError::InvalidInput(
            "This backup format is not supported".into(),
        ));
    }
    let salt = decode_fixed::<16>(&envelope.salt)?;
    let nonce = decode_fixed::<12>(&envelope.nonce)?;
    let ciphertext = STANDARD
        .decode(envelope.ciphertext)
        .map_err(|_| AppError::InvalidEncryptedData)?;
    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::Encryption)?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| {
            AppError::InvalidInput("Incorrect backup password or damaged backup".into())
        })?;
    let data: BackupData = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::InvalidInput("The backup contents are invalid".into()))?;
    if data.version != VERSION {
        return Err(AppError::InvalidInput(
            "This backup data version is not supported".into(),
        ));
    }
    Ok(data)
}

fn derive_key(password: &str, salt: &[u8]) -> AppResult<[u8; 32]> {
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(key)
}

fn validate_password(password: &str) -> AppResult<()> {
    if password.chars().count() < 8 {
        Err(AppError::InvalidInput(
            "Backup password must be at least 8 characters".into(),
        ))
    } else {
        Ok(())
    }
}

fn decode_fixed<const N: usize>(value: &str) -> AppResult<[u8; N]> {
    STANDARD
        .decode(value)
        .map_err(|_| AppError::InvalidEncryptedData)?
        .try_into()
        .map_err(|_| AppError::InvalidEncryptedData)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BackupGroup, BackupHost, Credential};

    #[test]
    fn portable_backup_round_trip_and_wrong_password_rejection() {
        let path = std::env::temp_dir().join(format!(
            "northstar-backup-test-{}.northstar",
            std::process::id()
        ));
        let data = BackupData {
            version: 1,
            groups: vec![BackupGroup {
                id: 7,
                name: "Servers".into(),
                parent_id: None,
            }],
            hosts: vec![BackupHost {
                name: "Example".into(),
                address: "example.com".into(),
                port: 22,
                username: "ubuntu".into(),
                group_id: Some(7),
                auth_type: "password".into(),
                credential: Credential::Password {
                    password: "secret-value".into(),
                },
            }],
        };

        write(&path, "strong-password", &data).expect("write backup");
        let on_disk = fs::read_to_string(&path).expect("read encrypted file");
        assert!(!on_disk.contains("secret-value"));
        assert!(read(&path, "wrong-password").is_err());
        let restored = read(&path, "strong-password").expect("read backup");
        assert_eq!(restored.groups.len(), 1);
        assert_eq!(restored.hosts.len(), 1);
        fs::remove_file(path).expect("remove backup");
    }
}
