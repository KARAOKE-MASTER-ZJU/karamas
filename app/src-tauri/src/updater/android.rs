use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateConfig {
    endpoints: Vec<url::Url>,
    #[serde(default)]
    dangerous_insecure_transport_protocol: bool,
}

impl UpdateConfig {
    fn validate_url(&self, url: &url::Url) -> CommandResult<()> {
        if url.scheme() == "https"
            || (self.dangerous_insecure_transport_protocol && url.scheme() == "http")
        {
            Ok(())
        } else {
            Err(failure("更新地址必须使用 HTTPS"))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: semver::Version,
    version_code: u64,
    notes: String,
    platforms: HashMap<String, Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    url: url::Url,
    sha256: String,
    size: u64,
}

pub struct PendingUpdate {
    artifact: Artifact,
    version_code: u64,
    path: Option<PathBuf>,
}

fn abi() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else if cfg!(target_arch = "arm") {
        "armeabi-v7a"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x86_64"
    }
}

pub async fn cleanup_download<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(cache) = app.path().app_cache_dir() {
        let _ = tokio::fs::remove_file(cache.join("updates/pending.apk.part")).await;
    }
}

pub async fn check<R: Runtime>(
    app: &AppHandle<R>,
) -> CommandResult<Option<(UpdateInfo, PendingUpdate)>> {
    let config: UpdateConfig = serde_json::from_value(
        app.config()
            .plugins
            .0
            .get("updater")
            .cloned()
            .ok_or_else(|| failure("未找到更新地址配置"))?,
    )
    .map_err(failure)?;
    let endpoint = config
        .endpoints
        .first()
        .ok_or_else(|| failure("未找到更新地址配置"))?;
    config.validate_url(endpoint)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(failure)?;
    let mut manifest: Manifest = client
        .get(endpoint.clone())
        .send()
        .await
        .map_err(failure)?
        .error_for_status()
        .map_err(failure)?
        .json()
        .await
        .map_err(failure)?;
    let installed = crate::platform::android_update_version(app).await?;
    // beta 与正式版可能共用 versionCode，由 SemVer 判断是否有新版。
    if manifest.version <= app.package_info().version || manifest.version_code < installed {
        return Ok(None);
    }
    let artifact = manifest
        .platforms
        .remove(abi())
        .ok_or_else(|| failure("此版本没有适合当前设备的安装包"))?;
    config.validate_url(&artifact.url)?;
    if artifact.size == 0 || artifact.sha256.len() != 64 {
        return Err(failure("更新信息不完整，请稍后重试"));
    }
    Ok(Some((
        UpdateInfo {
            version: manifest.version.to_string(),
            notes: manifest.notes,
        },
        PendingUpdate {
            artifact,
            version_code: manifest.version_code,
            path: None,
        },
    )))
}

pub async fn download<R: Runtime>(
    app: &AppHandle<R>,
    pending: &mut PendingUpdate,
    progress: Channel<DownloadProgress>,
) -> CommandResult<()> {
    if pending.path.is_some() {
        return Ok(());
    }
    let directory = app.path().app_cache_dir().map_err(failure)?.join("updates");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(failure)?;
    let partial = directory.join("pending.apk.part");
    let path = directory.join("pending.apk");
    let result: CommandResult<()> = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15 * 60))
            .build()
            .map_err(failure)?;
        let mut response = client
            .get(pending.artifact.url.clone())
            .send()
            .await
            .map_err(failure)?
            .error_for_status()
            .map_err(failure)?;
        let mut file = tokio::fs::File::create(&partial).await.map_err(failure)?;
        let mut hash = Sha256::new();
        let mut downloaded = 0;
        while let Some(chunk) = response.chunk().await.map_err(failure)? {
            downloaded += chunk.len() as u64;
            if downloaded > pending.artifact.size {
                return Err(failure("安装包大小与更新信息不符"));
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(failure)?;
            let _ = progress.send(DownloadProgress {
                downloaded,
                total: Some(pending.artifact.size),
            });
        }
        let checksum = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if downloaded != pending.artifact.size || checksum != pending.artifact.sha256 {
            return Err(failure("安装包下载不完整，请重新下载"));
        }
        file.flush().await.map_err(failure)?;
        drop(file);
        tokio::fs::rename(&partial, &path).await.map_err(failure)?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    result?;
    pending.path = Some(path);
    Ok(())
}

pub async fn install<R: Runtime>(
    app: &AppHandle<R>,
    pending: &mut PendingUpdate,
) -> CommandResult<()> {
    let path = pending
        .path
        .as_ref()
        .ok_or_else(|| failure("请先下载更新"))?;
    crate::platform::install_android_update(app, path, pending.version_code).await
}
