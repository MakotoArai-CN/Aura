use serde::Serialize;
use tauri::AppHandle;

const GITHUB_REPO: &str = "MakotoArai-CN/Aura";

#[derive(Debug, Serialize, Clone)]
pub struct UpdateAsset {
    pub label: String,
    pub url: String,
    pub filename: String,
}

fn update_assets(version: &str) -> Vec<UpdateAsset> {
    let base = format!("https://github.com/{GITHUB_REPO}/releases/download/v{version}");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match os {
        "windows" => {
            let arch_tag = match arch {
                "x86_64" => "x64",
                "x86" => "x86",
                "aarch64" => "arm64",
                _ => "x64",
            };
            vec![
                UpdateAsset {
                    label: format!("Windows {arch_tag} 安装包 (exe)"),
                    filename: format!("Aura_{version}_{arch_tag}-setup.exe"),
                    url: format!("{base}/Aura_{version}_{arch_tag}-setup.exe"),
                },
                UpdateAsset {
                    label: format!("Windows {arch_tag} 安装包 (msi)"),
                    filename: format!("Aura_{version}_{arch_tag}_en-US.msi"),
                    url: format!("{base}/Aura_{version}_{arch_tag}_en-US.msi"),
                },
            ]
        }
        "macos" => {
            let arch_tag = match arch {
                "x86_64" => "x64",
                "aarch64" => "aarch64",
                _ => "aarch64",
            };
            let label = if arch_tag == "aarch64" {
                "macOS Apple Silicon"
            } else {
                "macOS Intel"
            };
            vec![UpdateAsset {
                label: format!("{label} (dmg)"),
                filename: format!("Aura_{version}_{arch_tag}.dmg"),
                url: format!("{base}/Aura_{version}_{arch_tag}.dmg"),
            }]
        }
        "linux" => vec![
            UpdateAsset {
                label: "Linux (AppImage)".into(),
                filename: format!("Aura_{version}_amd64.AppImage"),
                url: format!("{base}/Aura_{version}_amd64.AppImage"),
            },
            UpdateAsset {
                label: "Linux (deb)".into(),
                filename: format!("Aura_{version}_amd64.deb"),
                url: format!("{base}/Aura_{version}_amd64.deb"),
            },
            UpdateAsset {
                label: "Linux (rpm)".into(),
                filename: format!("Aura_{version}-1.x86_64.rpm"),
                url: format!("{base}/Aura_{version}-1.x86_64.rpm"),
            },
        ],
        "android" => {
            let arch_tag = match arch {
                "aarch64" => "arm64",
                "arm" => "arm",
                "x86_64" => "x86_64",
                "x86" => "x86",
                _ => "universal",
            };
            vec![
                UpdateAsset {
                    label: format!("Android {arch_tag}"),
                    filename: format!("aura-android-{arch_tag}.apk"),
                    url: format!("{base}/aura-android-{arch_tag}.apk"),
                },
                UpdateAsset {
                    label: "Android 通用".into(),
                    filename: "aura-android-universal.apk".into(),
                    url: format!("{base}/aura-android-universal.apk"),
                },
            ]
        }
        _ => vec![],
    }
}

#[tauri::command]
pub fn get_update_assets(version: String) -> Vec<UpdateAsset> {
    update_assets(&version)
}

#[tauri::command]
#[cfg(desktop)]
pub async fn download_and_run_update(app: AppHandle, url: String, filename: String) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[aura] downloading update: {url}");

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| format!("读取失败: {e}"))?;

    let path = std::env::temp_dir().join(&filename);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入失败: {e}"))?;

    #[cfg(debug_assertions)]
    eprintln!("[aura] saved to {}, launching...", path.display());

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(&path)
            .spawn()
            .map_err(|e| format!("启动安装程序失败: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开安装包失败: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "AppImage" => {
                #[allow(clippy::permissions_set_readonly_false)]
                {
                    let mut perms = std::fs::metadata(&path)
                        .map_err(|e| format!("获取文件权限失败: {e}"))?
                        .permissions();
                    use std::os::unix::fs::PermissionsExt;
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&path, perms)
                        .map_err(|e| format!("设置执行权限失败: {e}"))?;
                }
                std::process::Command::new(&path)
                    .spawn()
                    .map_err(|e| format!("启动 AppImage 失败: {e}"))?;
            }
            "deb" => {
                std::process::Command::new("xdg-open")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("pkexec")
                            .args(["dpkg", "-i"])
                            .arg(&path)
                            .spawn()
                    })
                    .map_err(|e| format!("安装 deb 包失败: {e}"))?;
            }
            "rpm" => {
                std::process::Command::new("xdg-open")
                    .arg(&path)
                    .spawn()
                    .or_else(|_| {
                        std::process::Command::new("pkexec")
                            .args(["rpm", "-U"])
                            .arg(&path)
                            .spawn()
                    })
                    .map_err(|e| format!("安装 rpm 包失败: {e}"))?;
            }
            _ => {
                std::process::Command::new("xdg-open")
                    .arg(&path)
                    .spawn()
                    .map_err(|e| format!("打开文件失败: {e}"))?;
            }
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    app.exit(0);
    Ok(())
}

#[tauri::command]
#[cfg(not(desktop))]
pub async fn download_and_run_update(
    _app: AppHandle,
    _url: String,
    _filename: String,
) -> Result<(), String> {
    Err("MOBILE_OPEN_URL".to_string())
}
