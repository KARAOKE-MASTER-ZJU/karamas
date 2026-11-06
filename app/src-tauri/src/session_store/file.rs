use crate::error::{CommandError, CommandResult};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

const AAD: &[u8] = b"karamas.bilibili.session.v1";

#[derive(Serialize, Deserialize)]
struct EncryptedSession {
    version: u8,
    nonce: String,
    ciphertext: String,
}

pub(super) struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            directory: data_dir.join("credentials"),
        }
    }

    async fn run<T: Send + 'static>(
        &self,
        action: impl FnOnce(PathBuf) -> CommandResult<T> + Send + 'static,
    ) -> CommandResult<T> {
        let directory = self.directory.clone();
        tauri::async_runtime::spawn_blocking(move || action(directory))
            .await
            .map_err(|_| storage_error("会话存储任务执行失败，请重试。"))?
    }

    pub async fn read(&self) -> CommandResult<Option<String>> {
        self.run(|directory| {
            let bytes = match fs::read(directory.join("session.json")) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(_) => {
                    return Err(storage_error(
                        "无法读取登录会话，请检查应用数据目录的权限。",
                    ));
                }
            };
            let envelope: EncryptedSession =
                serde_json::from_slice(&bytes).map_err(|_| invalid_session())?;
            if envelope.version != 1 {
                return Err(invalid_session());
            }
            let key = read_key(&directory)?
                .ok_or_else(|| storage_error("登录密钥丢失，请重新扫码登录。"))?;
            let nonce = STANDARD
                .decode(&envelope.nonce)
                .map_err(|_| invalid_session())?;
            let nonce =
                aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| invalid_session())?;
            let mut ciphertext = STANDARD
                .decode(&envelope.ciphertext)
                .map_err(|_| invalid_session())?;
            let plaintext = cipher(&key)?
                .open_in_place(nonce, aead::Aad::from(AAD), &mut ciphertext)
                .map_err(|_| invalid_session())?;
            let session = String::from_utf8(plaintext.to_vec()).map_err(|_| invalid_session())?;
            Ok(Some(session))
        })
        .await
    }

    pub async fn write(&self, value: String) -> CommandResult<()> {
        self.run(move |directory| {
            prepare_directory(&directory)?;
            let key = load_or_create_key(&directory)?;
            let mut nonce = [0; aead::NONCE_LEN];
            SystemRandom::new()
                .fill(&mut nonce)
                .map_err(|_| storage_error("无法生成会话随机数，请重试。"))?;
            let mut ciphertext = value.into_bytes();
            cipher(&key)?
                .seal_in_place_append_tag(
                    aead::Nonce::assume_unique_for_key(nonce),
                    aead::Aad::from(AAD),
                    &mut ciphertext,
                )
                .map_err(|_| storage_error("登录会话加密失败，请重试。"))?;
            let envelope = EncryptedSession {
                version: 1,
                nonce: STANDARD.encode(nonce),
                ciphertext: STANDARD.encode(ciphertext),
            };
            let bytes =
                serde_json::to_vec(&envelope).map_err(|_| storage_error("无法编码加密会话。"))?;
            private_file(&directory, &bytes)?
                .persist(directory.join("session.json"))
                .map_err(|_| storage_error("无法保存登录会话，请检查应用数据目录的权限。"))?;
            Ok(())
        })
        .await
    }

    pub async fn delete(&self) -> CommandResult<()> {
        self.run(
            |directory| match fs::remove_file(directory.join("session.json")) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                Err(_) => Err(storage_error(
                    "无法删除登录会话，请检查应用数据目录的权限。",
                )),
            },
        )
        .await
    }
}

fn storage_error(message: &str) -> CommandError {
    CommandError::new("STORAGE_ERROR", message)
}

fn invalid_session() -> CommandError {
    CommandError::new("SESSION_FORMAT", "保存的登录会话无法解密，请重新扫码登录。")
}

fn cipher(key: &[u8; 32]) -> CommandResult<aead::LessSafeKey> {
    aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map(aead::LessSafeKey::new)
        .map_err(|_| storage_error("无法初始化会话加密。"))
}

fn prepare_directory(directory: &Path) -> CommandResult<()> {
    fs::create_dir_all(directory)
        .map_err(|_| storage_error("无法创建会话目录，请检查应用数据目录的权限。"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| storage_error("无法设置会话目录权限。"))?;
    }
    Ok(())
}

fn private_file(directory: &Path, bytes: &[u8]) -> CommandResult<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| storage_error("无法创建会话存储文件。"))?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| storage_error("无法写入会话存储文件。"))?;
    Ok(file)
}

fn read_key(directory: &Path) -> CommandResult<Option<[u8; 32]>> {
    match fs::read(directory.join("key.bin")) {
        Ok(bytes) => bytes
            .try_into()
            .map(Some)
            .map_err(|_| storage_error("登录密钥格式无效。")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(storage_error(
            "无法读取登录密钥，请检查应用数据目录的权限。",
        )),
    }
}

fn load_or_create_key(directory: &Path) -> CommandResult<[u8; 32]> {
    if let Some(key) = read_key(directory)? {
        return Ok(key);
    }
    let mut key = [0; 32];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| storage_error("无法生成登录密钥，请重试。"))?;
    match private_file(directory, &key)?.persist_noclobber(directory.join("key.bin")) {
        Ok(_) => Ok(key),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            read_key(directory)?.ok_or_else(|| storage_error("登录密钥保存失败，请重试。"))
        }
        Err(_) => Err(storage_error(
            "无法保存登录密钥，请检查应用数据目录的权限。",
        )),
    }
}
