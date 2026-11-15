//! 便携版自动更新：检查 GitHub Release、下载校验 ZIP、暂存新程序并重启替换。
//!
//! 更新只替换 exe 与 README，不覆盖用户 data 中的配置与缓存。
//! 参考 ddl-desktop 的实现，改为与项目一致的阻塞式 reqwest。

use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/zhanghu-god/push-wp/releases/latest";
const USER_AGENT: &str = "push-wp-updater";
const PORTABLE_SUFFIX: &str = "_x64-portable.zip";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_url: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub asset_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedUpdate {
    pub version: String,
    pub asset_name: String,
    pub already_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedMeta {
    version: String,
    asset_name: String,
    asset_digest: String,
    staged_exe: String,
    staged_readme: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 检查 GitHub 最新 Release 与便携 ZIP 资产
pub fn check_update() -> Result<UpdateInfo, String> {
    let client = http_client(Duration::from_secs(30))?;
    let text = client
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("检查更新失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("检查更新失败：{e}"))?
        .text()
        .map_err(|e| format!("读取更新信息失败：{e}"))?;
    let release: GithubRelease =
        serde_json::from_str(&text).map_err(|e| format!("解析更新信息失败：{e}"))?;

    let current = parse_version(current_version())?;
    let latest = parse_version(&release.tag_name)?;
    let asset = select_asset(&release.assets)?;
    let digest = asset
        .digest
        .clone()
        .ok_or_else(|| "Release 附件缺少 SHA-256 校验值".to_string())?;

    Ok(UpdateInfo {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        has_update: latest > current,
        release_url: release.html_url,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        asset_size: asset.size,
        asset_digest: digest,
    })
}

/// 下载更新包，校验大小与 SHA-256，解压暂存 exe 与 README，并写入元数据
pub fn prepare_update(data_dir: &Path, info: &UpdateInfo) -> Result<PreparedUpdate, String> {
    if !info.has_update {
        return Err("当前已是最新版本".into());
    }

    let update_dir = data_dir.join("update");
    let staged_dir = update_dir.join("staged");
    let meta_path = update_dir.join("update.json");

    // 同一版本已暂存则直接复用，不重复下载
    if let Ok(text) = fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<StagedMeta>(&text) {
            if meta.version == info.latest_version
                && meta.asset_digest == info.asset_digest
                && PathBuf::from(&meta.staged_exe).exists()
            {
                return Ok(PreparedUpdate {
                    version: meta.version,
                    asset_name: meta.asset_name,
                    already_staged: true,
                });
            }
        }
    }

    let client = http_client(Duration::from_secs(300))?;
    let bytes = client
        .get(&info.asset_url)
        .send()
        .map_err(|e| format!("下载更新失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("下载更新失败：{e}"))?
        .bytes()
        .map_err(|e| format!("下载更新失败：{e}"))?;

    if info.asset_size > 0 && bytes.len() as u64 != info.asset_size {
        return Err("更新包大小与 Release 不一致".into());
    }
    verify_digest(&bytes, &info.asset_digest)?;

    if staged_dir.exists() {
        fs::remove_dir_all(&staged_dir).map_err(|e| format!("清理旧更新文件失败：{e}"))?;
    }
    fs::create_dir_all(&staged_dir).map_err(|e| format!("创建更新目录失败：{e}"))?;

    let staged_readme = extract_update_files(&bytes, &staged_dir)?;
    let staged_exe = staged_dir.join("push-wp.exe");
    if !staged_exe.exists() {
        return Err("更新包中未找到 push-wp.exe".into());
    }

    let meta = StagedMeta {
        version: info.latest_version.clone(),
        asset_name: info.asset_name.clone(),
        asset_digest: info.asset_digest.clone(),
        staged_exe: staged_exe.to_string_lossy().into_owned(),
        staged_readme: staged_readme.map(|p| p.to_string_lossy().into_owned()),
    };
    let text = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, text).map_err(|e| format!("写入更新元数据失败：{e}"))?;

    Ok(PreparedUpdate {
        version: meta.version,
        asset_name: meta.asset_name,
        already_staged: false,
    })
}

/// 生成并启动 PowerShell 更新脚本：等待本进程退出后替换 exe/README 并重新启动
pub fn apply_update_and_restart(data_dir: &Path) -> Result<(), String> {
    let update_dir = data_dir.join("update");
    let meta_path = update_dir.join("update.json");
    let text = fs::read_to_string(&meta_path).map_err(|_| "没有已准备好的更新".to_string())?;
    let meta: StagedMeta = serde_json::from_str(&text).map_err(|e| format!("更新元数据损坏：{e}"))?;
    let staged_exe = PathBuf::from(&meta.staged_exe);
    if !staged_exe.exists() {
        return Err("暂存的新程序不存在，请重新下载更新".into());
    }

    let target_exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序：{e}"))?;
    let target_dir = target_exe
        .parent()
        .ok_or_else(|| "程序路径没有父目录".to_string())?;
    let target_readme = target_dir.join("README.md");
    let staged_readme = meta.staged_readme.clone().unwrap_or_default();
    let script_path = update_dir.join("apply-update.ps1");
    fs::write(&script_path, UPDATE_HELPER_SCRIPT).map_err(|e| format!("写入更新脚本失败：{e}"))?;

    spawn_helper(
        &script_path,
        std::process::id(),
        &staged_exe,
        &target_exe,
        &staged_readme,
        &target_readme,
        &update_dir,
    )?;
    Ok(())
}

/// 查询已暂存待安装的更新（用户之前选择了"稍后"）
pub fn pending_update(data_dir: &Path) -> Option<PreparedUpdate> {
    let text = fs::read_to_string(data_dir.join("update").join("update.json")).ok()?;
    let meta: StagedMeta = serde_json::from_str(&text).ok()?;
    if PathBuf::from(&meta.staged_exe).exists() {
        Some(PreparedUpdate {
            version: meta.version,
            asset_name: meta.asset_name,
            already_staged: true,
        })
    } else {
        None
    }
}

/// 启动时清理上次更新遗留的脚本目录；仍有 update.json 时表示用户选择了稍后更新
pub fn cleanup_leftover(data_dir: &Path) {
    let update_dir = data_dir.join("update");
    if update_dir.exists() && !update_dir.join("update.json").exists() {
        let _ = fs::remove_dir_all(update_dir);
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))
}

fn parse_version(tag: &str) -> Result<Version, String> {
    let normalized = tag.trim().trim_start_matches('v');
    Version::parse(normalized).map_err(|e| format!("无法解析版本号 {tag}：{e}"))
}

fn select_asset(assets: &[GithubAsset]) -> Result<&GithubAsset, String> {
    assets
        .iter()
        .find(|asset| asset.name.ends_with(PORTABLE_SUFFIX))
        .or_else(|| {
            assets
                .iter()
                .find(|asset| asset.name.contains("portable") && asset.name.ends_with(".zip"))
        })
        .ok_or_else(|| "最新 Release 中没有便携 ZIP 附件".to_string())
}

fn verify_digest(bytes: &[u8], digest: &str) -> Result<(), String> {
    let expected = digest
        .trim()
        .strip_prefix("sha256:")
        .unwrap_or(digest.trim())
        .to_ascii_lowercase();
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err("更新包 SHA-256 校验失败".into());
    }
    Ok(())
}

fn extract_update_files(bytes: &[u8], staged_dir: &Path) -> Result<Option<PathBuf>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("打开更新包失败：{e}"))?;
    let mut exe_index = None;
    let mut readme_index = None;

    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|e| format!("读取更新包失败：{e}"))?;
        let name = file.name().replace('\\', "/");
        if name.ends_with("/push-wp.exe") || name == "push-wp.exe" {
            exe_index = Some(index);
        } else if name.ends_with("/README.md") || name == "README.md" {
            readme_index = Some(index);
        }
    }

    let exe_index = exe_index.ok_or_else(|| "更新包中未找到 push-wp.exe".to_string())?;
    extract_one(&mut archive, exe_index, &staged_dir.join("push-wp.exe"))?;

    let readme_path = match readme_index {
        Some(index) => {
            let path = staged_dir.join("README.md");
            extract_one(&mut archive, index, &path)?;
            Some(path)
        }
        None => None,
    };
    Ok(readme_path)
}

fn extract_one(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
    target: &Path,
) -> Result<(), String> {
    let mut file = archive
        .by_index(index)
        .map_err(|e| format!("读取更新包失败：{e}"))?;
    let mut out = fs::File::create(target).map_err(|e| format!("创建暂存文件失败：{e}"))?;
    io::copy(&mut file, &mut out).map_err(|e| format!("解压更新文件失败：{e}"))?;
    out.flush().map_err(|e| format!("写入暂存文件失败：{e}"))?;
    Ok(())
}

#[cfg(windows)]
fn spawn_helper(
    script: &Path,
    pid: u32,
    staged_exe: &Path,
    target_exe: &Path,
    staged_readme: &str,
    target_readme: &Path,
    update_dir: &Path,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script)
        .args(["-ProcessId", &pid.to_string()])
        .arg("-StagedExe")
        .arg(staged_exe)
        .arg("-TargetExe")
        .arg(target_exe)
        .arg("-StagedReadme")
        .arg(staged_readme)
        .arg("-TargetReadme")
        .arg(target_readme)
        .arg("-UpdateDir")
        .arg(update_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("启动更新脚本失败：{e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_helper(
    _script: &Path,
    _pid: u32,
    _staged_exe: &Path,
    _target_exe: &Path,
    _staged_readme: &str,
    _target_readme: &Path,
    _update_dir: &Path,
) -> Result<(), String> {
    Err("自动更新目前仅支持 Windows".into())
}

const UPDATE_HELPER_SCRIPT: &str = r#"param(
  [int]$ProcessId,
  [string]$StagedExe,
  [string]$TargetExe,
  [string]$StagedReadme = '',
  [string]$TargetReadme,
  [string]$UpdateDir
)
$ErrorActionPreference = 'Stop'
while (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue) {
  Start-Sleep -Milliseconds 200
}
Copy-Item -LiteralPath $StagedExe -Destination $TargetExe -Force
if ($StagedReadme -and (Test-Path -LiteralPath $StagedReadme)) {
  Copy-Item -LiteralPath $StagedReadme -Destination $TargetReadme -Force
}
Start-Process -FilePath $TargetExe -WorkingDirectory (Split-Path -Parent $TargetExe)
$staged = Join-Path $UpdateDir 'staged'
if (Test-Path -LiteralPath $staged) {
  Remove-Item -LiteralPath $staged -Recurse -Force -ErrorAction SilentlyContinue
}
$meta = Join-Path $UpdateDir 'update.json'
if (Test-Path -LiteralPath $meta) {
  Remove-Item -LiteralPath $meta -Force -ErrorAction SilentlyContinue
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v_prefixed_versions() {
        assert_eq!(parse_version("v0.2.0").unwrap(), Version::new(0, 2, 0));
        assert!(parse_version("v0.2.0").unwrap() > parse_version("0.1.0").unwrap());
        assert!(parse_version("not-a-version").is_err());
    }

    #[test]
    fn selects_portable_zip_asset() {
        let assets = vec![
            GithubAsset {
                name: "source.zip".into(),
                size: 1,
                browser_download_url: "https://example.com/source.zip".into(),
                digest: Some("sha256:abc".into()),
            },
            GithubAsset {
                name: "push-wp_0.2.0_x64-portable.zip".into(),
                size: 2,
                browser_download_url: "https://example.com/portable.zip".into(),
                digest: Some("sha256:def".into()),
            },
        ];
        let asset = select_asset(&assets).unwrap();
        assert!(asset.name.ends_with(PORTABLE_SUFFIX));
    }

    #[test]
    fn verifies_sha256_digest() {
        let bytes = b"push-wp update";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        assert!(verify_digest(bytes, &digest).is_ok());
        assert!(verify_digest(bytes, "sha256:0000").is_err());
    }

    #[test]
    fn extracts_exe_and_readme_from_zip() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("push-wp/push-wp.exe", options).unwrap();
            writer.write_all(b"exe").unwrap();
            writer.start_file("push-wp/README.md", options).unwrap();
            writer.write_all(b"readme").unwrap();
            writer.start_file("push-wp/data/settings.json", options).unwrap();
            writer.write_all(b"{}").unwrap();
            writer.finish().unwrap();
        }

        let dir = std::env::temp_dir().join(format!("push-wp-update-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let readme = extract_update_files(&buffer.into_inner(), &dir).unwrap();
        assert!(dir.join("push-wp.exe").exists());
        assert!(readme.unwrap().exists());
        assert!(!dir.join("settings.json").exists());
        let _ = fs::remove_dir_all(dir);
    }
}