use async_trait::async_trait;

use crate::error::LauncherError;
use crate::runtime::RuntimePaths;

const SERVICE_NAME: &str = "org.orbit.launcher";

#[async_trait]
pub trait SecretStore: Send + Sync {
    fn backend_name(&self) -> &'static str;

    async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, LauncherError>;

    async fn replace(&self, key: &str, secret: &[u8]) -> Result<(), LauncherError>;

    async fn delete(&self, key: &str) -> Result<(), LauncherError>;
}

pub fn native_secret_store(paths: &RuntimePaths) -> Result<Box<dyn SecretStore>, LauncherError> {
    platform::native_secret_store(paths)
}

fn validate_key(key: &str) -> Result<(), LauncherError> {
    if key.is_empty() || key.len() > 160 || key.trim() != key || key.chars().any(char::is_control) {
        return Err(LauncherError::SecretStore(
            "secret record key is invalid".to_string(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;
    use std::ptr;

    use async_trait::async_trait;
    use sha2::{Digest, Sha256};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    use super::{SERVICE_NAME, SecretStore, validate_key};
    use crate::atomic_io::write_atomic;
    use crate::error::LauncherError;
    use crate::runtime::RuntimePaths;

    const FILE_MAGIC: &[u8] = b"ORBIT-DPAPI-1\0";

    pub(super) fn native_secret_store(
        paths: &RuntimePaths,
    ) -> Result<Box<dyn SecretStore>, LauncherError> {
        Ok(Box::new(DpapiSecretStore {
            directory: paths.data_dir().join("credentials"),
        }))
    }

    #[derive(Debug, Clone)]
    struct DpapiSecretStore {
        directory: PathBuf,
    }

    impl DpapiSecretStore {
        fn record_path(&self, key: &str) -> Result<PathBuf, LauncherError> {
            validate_key(key)?;
            Ok(self.directory.join(format!(
                "{}.bin",
                hex::encode(Sha256::digest(key.as_bytes()))
            )))
        }

        fn protect(key: &str, plaintext: &[u8]) -> Result<Vec<u8>, LauncherError> {
            let input = blob(plaintext)?;
            let entropy_bytes = entropy(key);
            let entropy = blob(&entropy_bytes)?;
            let mut output = CRYPT_INTEGER_BLOB::default();
            // SAFETY: the input and entropy slices outlive the call, all optional pointers are
            // null, and the returned LocalAlloc buffer is copied before LocalFree.
            let succeeded = unsafe {
                CryptProtectData(
                    &input,
                    ptr::null(),
                    &entropy,
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            if succeeded == 0 {
                return Err(LauncherError::SecretStore(format!(
                    "Windows DPAPI failed to protect a credential: {}",
                    std::io::Error::last_os_error()
                )));
            }
            copy_and_free(output)
        }

        fn unprotect(key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, LauncherError> {
            let input = blob(ciphertext)?;
            let entropy_bytes = entropy(key);
            let entropy = blob(&entropy_bytes)?;
            let mut output = CRYPT_INTEGER_BLOB::default();
            // SAFETY: the input and entropy slices outlive the call, all optional pointers are
            // null, and the returned LocalAlloc buffer is copied before LocalFree.
            let succeeded = unsafe {
                CryptUnprotectData(
                    &input,
                    ptr::null_mut(),
                    &entropy,
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            if succeeded == 0 {
                return Err(LauncherError::SecretStore(format!(
                    "Windows DPAPI could not decrypt this user's credential: {}",
                    std::io::Error::last_os_error()
                )));
            }
            copy_and_free(output)
        }
    }

    #[async_trait]
    impl SecretStore for DpapiSecretStore {
        fn backend_name(&self) -> &'static str {
            "windows-dpapi-current-user"
        }

        async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, LauncherError> {
            let path = self.record_path(key)?;
            if !path.exists() {
                return Ok(None);
            }
            let bytes = std::fs::read(path)?;
            let ciphertext = bytes.strip_prefix(FILE_MAGIC).ok_or_else(|| {
                LauncherError::SecretStore("credential envelope has an unknown format".to_string())
            })?;
            if ciphertext.is_empty() {
                return Err(LauncherError::SecretStore(
                    "credential envelope is empty".to_string(),
                ));
            }
            Self::unprotect(key, ciphertext).map(Some)
        }

        async fn replace(&self, key: &str, secret: &[u8]) -> Result<(), LauncherError> {
            if secret.is_empty() {
                return Err(LauncherError::SecretStore(
                    "refusing to persist an empty secret".to_string(),
                ));
            }
            let path = self.record_path(key)?;
            let protected = Self::protect(key, secret)?;
            let mut envelope = Vec::with_capacity(FILE_MAGIC.len() + protected.len());
            envelope.extend_from_slice(FILE_MAGIC);
            envelope.extend_from_slice(&protected);
            write_atomic(&path, &envelope)
        }

        async fn delete(&self, key: &str) -> Result<(), LauncherError> {
            let path = self.record_path(key)?;
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
    }

    fn entropy(key: &str) -> [u8; 32] {
        Sha256::digest(format!("{SERVICE_NAME}:v1:{key}").as_bytes()).into()
    }

    fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, LauncherError> {
        let length = u32::try_from(bytes.len()).map_err(|_| {
            LauncherError::SecretStore("credential is too large for Windows DPAPI".to_string())
        })?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: bytes.as_ptr().cast_mut(),
        })
    }

    fn copy_and_free(output: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, LauncherError> {
        if output.pbData.is_null() || output.cbData == 0 {
            return Err(LauncherError::SecretStore(
                "Windows DPAPI returned an empty credential".to_string(),
            ));
        }
        // SAFETY: CryptProtectData/CryptUnprotectData returned a valid buffer with cbData bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        // SAFETY: DPAPI allocates this exact pointer with LocalAlloc.
        let result = unsafe { LocalFree(output.pbData.cast()) };
        if !result.is_null() {
            return Err(LauncherError::SecretStore(
                "Windows failed to release a DPAPI buffer".to_string(),
            ));
        }
        Ok(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn dpapi_records_roundtrip_and_are_not_plaintext() {
            let directory = tempfile::tempdir().unwrap();
            let store = DpapiSecretStore {
                directory: directory.path().join("credentials"),
            };
            let key = "account:00000000-0000-0000-0000-000000000001";
            store.replace(key, b"refresh-token-value").await.unwrap();
            assert_eq!(
                store.load(key).await.unwrap().as_deref(),
                Some(b"refresh-token-value".as_slice())
            );
            let stored = std::fs::read(store.record_path(key).unwrap()).unwrap();
            assert!(
                !stored
                    .windows(b"refresh-token-value".len())
                    .any(|window| window == b"refresh-token-value")
            );
            store.delete(key).await.unwrap();
            assert!(store.load(key).await.unwrap().is_none());
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use secret_service::{EncryptionType, SecretService};

    use super::{SERVICE_NAME, SecretStore, validate_key};
    use crate::error::LauncherError;
    use crate::runtime::RuntimePaths;

    pub(super) fn native_secret_store(
        _paths: &RuntimePaths,
    ) -> Result<Box<dyn SecretStore>, LauncherError> {
        Ok(Box::new(SecretServiceStore))
    }

    #[derive(Debug, Clone, Copy)]
    struct SecretServiceStore;

    impl SecretServiceStore {
        fn attributes(key: &str) -> HashMap<&str, &str> {
            HashMap::from([("application", SERVICE_NAME), ("record", key)])
        }

        fn map_error(error: secret_service::Error) -> LauncherError {
            LauncherError::SecretStore(format!(
                "Secret Service is unavailable or rejected the operation: {error}"
            ))
        }
    }

    #[async_trait]
    impl SecretStore for SecretServiceStore {
        fn backend_name(&self) -> &'static str {
            "freedesktop-secret-service"
        }

        async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, LauncherError> {
            validate_key(key)?;
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(Self::map_error)?;
            let result = service
                .search_items(Self::attributes(key))
                .await
                .map_err(Self::map_error)?;
            if result.unlocked.len() + result.locked.len() > 1 {
                return Err(LauncherError::SecretStore(format!(
                    "Secret Service contains duplicate records for '{key}'"
                )));
            }
            if let Some(item) = result.unlocked.first() {
                return item.get_secret().await.map(Some).map_err(Self::map_error);
            }
            let Some(item) = result.locked.first() else {
                return Ok(None);
            };
            item.unlock().await.map_err(Self::map_error)?;
            item.get_secret().await.map(Some).map_err(Self::map_error)
        }

        async fn replace(&self, key: &str, secret: &[u8]) -> Result<(), LauncherError> {
            validate_key(key)?;
            if secret.is_empty() {
                return Err(LauncherError::SecretStore(
                    "refusing to persist an empty secret".to_string(),
                ));
            }
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(Self::map_error)?;
            let collection = service
                .get_default_collection()
                .await
                .map_err(Self::map_error)?;
            collection
                .ensure_unlocked()
                .await
                .map_err(Self::map_error)?;
            collection
                .create_item(
                    "Orbit Launcher account session",
                    Self::attributes(key),
                    secret,
                    true,
                    "application/json",
                )
                .await
                .map_err(Self::map_error)?;
            Ok(())
        }

        async fn delete(&self, key: &str) -> Result<(), LauncherError> {
            validate_key(key)?;
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(Self::map_error)?;
            let result = service
                .search_items(Self::attributes(key))
                .await
                .map_err(Self::map_error)?;
            for item in result.unlocked.iter().chain(result.locked.iter()) {
                item.delete().await.map_err(Self::map_error)?;
            }
            Ok(())
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::SecretStore;
    use crate::error::LauncherError;
    use crate::runtime::RuntimePaths;

    pub(super) fn native_secret_store(
        _paths: &RuntimePaths,
    ) -> Result<Box<dyn SecretStore>, LauncherError> {
        Err(LauncherError::UnsupportedPlatform)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{SecretStore, validate_key};
    use crate::error::LauncherError;

    #[derive(Debug, Default)]
    pub struct MemorySecretStore {
        records: Mutex<HashMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl SecretStore for MemorySecretStore {
        fn backend_name(&self) -> &'static str {
            "memory-test"
        }

        async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, LauncherError> {
            validate_key(key)?;
            Ok(self.records.lock().unwrap().get(key).cloned())
        }

        async fn replace(&self, key: &str, secret: &[u8]) -> Result<(), LauncherError> {
            validate_key(key)?;
            self.records
                .lock()
                .unwrap()
                .insert(key.to_string(), secret.to_vec());
            Ok(())
        }

        async fn delete(&self, key: &str) -> Result<(), LauncherError> {
            validate_key(key)?;
            self.records.lock().unwrap().remove(key);
            Ok(())
        }
    }
}
