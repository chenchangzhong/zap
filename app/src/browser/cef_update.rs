//! CEF 内核版本检查(设置页「CEF 内核更新」用)。
//!
//! 只做"已链接版本 vs 上游最新版本"的比较与展示:**不参与启动流程、不自动下载、不改任何设置**。
//! 上游来源是 CEF 官方构建索引(与 `download-cef` 用的同一个 CDN),与
//! `specs/cef-webview-minimal/CEF-UPGRADE.md` 的升级流程配套。

use std::time::Duration;

/// 已链接的 CEF 分发版版本。
///
/// 由 `app/build.rs` 从 `Cargo.lock` 里 `cef` crate 的版本推出(`154.0.0+154.0.23` → `154.0.23`)。
/// 之所以可信:`cef-dll-sys` 构建时用 `check_archive_json` 强制 CEF 目录与该 crate 版本匹配,
/// 所以这就是运行时实际加载的那份 CEF。
pub(crate) const INSTALLED_VERSION: &str = env!("ZAP_CEF_VERSION");

/// CEF 官方构建索引(与 `download-cef` 的 `DEFAULT_CDN_URL` 同一个 CDN)。
const CEF_INDEX_URL: &str = "https://cef-builds.spotifycdn.com/index.json";

/// 手动检查的等待上限。
///
/// 实测 index.json 约 **10MB**(2026-09-23),远大于 dsh 渠道检查那种几 KB 的响应,
/// 故取 30s(仍是手动触发,不存在自动轮询)。
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// 一次检查的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CefUpdateCheck {
    /// 上游有比已链接版本更新的版本。
    UpdateAvailable { latest: String },
    /// 已是最新(或索引里找不到可比较的版本)。
    UpToDate { latest: Option<String> },
    /// 检查失败(离线 / 超时 / 响应异常)。文案由调用方决定,这里只带原因。
    Failed { error: String },
}

/// 查上游最新版本并与 [`INSTALLED_VERSION`] 比较(设置页按钮触发;async,不阻塞 UI 线程)。
///
/// 不返回 `Result`:设置页只需要展示一行结果,调用方不该为网络问题写错误分支。
pub(crate) async fn check_cef_update_future() -> CefUpdateCheck {
    let platform = current_platform_key();
    match tokio::time::timeout(CHECK_TIMEOUT, fetch_latest(platform)).await {
        Ok(Ok(latest)) => match compare(&latest, INSTALLED_VERSION) {
            Some(true) => CefUpdateCheck::UpdateAvailable { latest },
            // `None` = 版本号无法比较(上游格式变化):按"无法判定"处理,不谎报有新版本。
            _ => CefUpdateCheck::UpToDate {
                latest: Some(latest),
            },
        },
        Ok(Err(error)) => CefUpdateCheck::Failed { error },
        Err(_) => CefUpdateCheck::Failed {
            error: format!("检查超时({CHECK_TIMEOUT:?})"),
        },
    }
}

/// 索引里本机架构对应的键(与 `download_cef::CefIndex` 的字段同名)。
fn current_platform_key() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "macosarm64"
    } else {
        "macosx64"
    }
}

/// 拉 `index.json` 并取该平台最新的 CEF 版本(形如 `154.0.23+g062ebe4+chromium-…`)。
async fn fetch_latest(platform: &str) -> Result<String, String> {
    let client = http_client::Client::new();
    let resp = client
        .get(CEF_INDEX_URL)
        .send()
        .await
        .map_err(|err| format!("请求失败:{err}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|err| format!("解析失败:{err}"))?;
    latest_from_index(&body, platform)
}

/// 从索引 JSON 里取"该平台最新版本"(纯函数,便于单测;不触网)。
///
/// 索引形状(与 `download-cef` 的 `CefIndex` 一致):
/// `{ "macosarm64": { "versions": [ { "cef_version": "154.0.23+g…", … }, … ] }, … }`。
fn latest_from_index(body: &serde_json::Value, platform: &str) -> Result<String, String> {
    let versions = body
        .get(platform)
        .and_then(|platform| platform.get("versions"))
        .and_then(|versions| versions.as_array())
        .ok_or_else(|| format!("索引里没有平台 {platform}"))?;
    versions
        .iter()
        .filter_map(|entry| entry.get("cef_version").and_then(|raw| raw.as_str()))
        .filter_map(|raw| parse_version(raw).map(|parsed| (parsed, raw.to_string())))
        .max_by_key(|(parsed, _)| *parsed)
        .map(|(_, raw)| raw)
        .ok_or_else(|| "索引里没有可解析的版本号".to_string())
}

/// 解析 `152.0.8+g1ce985c+chromium-…` 的版本段为可比较的元组。
///
/// `+` 之后是 CEF 自己的构建标识(commit/chromium 版本),不参与比较;缺 patch 时按 0 处理。
fn parse_version(raw: &str) -> Option<(u32, u32, u32)> {
    let core = raw.split('+').next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// `latest > installed`?任一侧不可解析时返回 `None`(调用方按"无法比较"处理)。
fn compare(latest: &str, installed: &str) -> Option<bool> {
    Some(parse_version(latest)? > parse_version(installed)?)
}

#[cfg(test)]
#[path = "cef_update_tests.rs"]
mod tests;
