use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{rngs::OsRng, RngCore};

use crate::error::{AppError, AppResult};

const SERVICE: &str = "app.northstar.ssh";
const ACCOUNT: &str = "database-master-key";

pub struct Vault {
    key: [u8; 32],
}

impl Vault {
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self { key: [42; 32] }
    }

    pub fn load() -> AppResult<Self> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)?;
        let key = match entry.get_password() {
            Ok(encoded) => {
                let bytes = STANDARD
                    .decode(encoded)
                    .map_err(|_| AppError::InvalidEncryptedData)?;
                bytes
                    .try_into()
                    .map_err(|_| AppError::InvalidEncryptedData)?
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = [0_u8; 32];
                OsRng.fill_bytes(&mut key);
                entry.set_password(&STANDARD.encode(key))?;
                key
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| AppError::Encryption)?;
        let mut nonce_bytes = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .map_err(|_| AppError::Encryption)?;

        let mut result = nonce_bytes.to_vec();
        result.extend(ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted: &[u8]) -> AppResult<Vec<u8>> {
        if encrypted.len() < 13 {
            return Err(AppError::InvalidEncryptedData);
        }
        let (nonce, ciphertext) = encrypted.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|_| AppError::Encryption)?;
        cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| AppError::InvalidEncryptedData)
    }
}
