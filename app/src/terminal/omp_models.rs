use std::process::Output;
use std::time::Instant;

/// Represents a model available through the `omp` binary.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OmpModel {
    pub provider: String,
    pub id: String,
    pub selector: String,
    pub name: String,
    #[serde(rename = "contextWindow")]
    pub context_window: Option<u64>,
}

/// Outer response wrapper for `omp models --json`.
#[derive(serde::Deserialize)]
struct ModelsResponse {
    models: Vec<OmpModel>,
}

/// Cached registry of omp models, backed by the `omp` CLI binary.
///
/// All subprocess calls use a 10-second timeout.
pub struct OmpModelRegistry {
    models: Vec<OmpModel>,
    last_refresh: Option<Instant>,
    omp_binary: String,
}

/// Run a command with a timeout. Returns its output on success, or an error string.
async fn run_with_timeout(
    cmd: &mut command::r#async::Command,
    timeout_secs: u64,
) -> Result<Output, String> {
    use futures_lite::future::race;

    cmd.kill_on_drop(true);
    let output_fut = cmd.output();
    let timeout_fut =
        warpui::r#async::Timer::after(std::time::Duration::from_secs(timeout_secs));

    let result = race(
        async { output_fut.await },
        async {
            timeout_fut.await;
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "command timed out"))
        },
    )
    .await;

    let output = result.map_err(|e| format!("command failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("exited with {}: {stderr}", output.status));
    }

    Ok(output)
}

impl OmpModelRegistry {
    pub fn new(omp_binary: &str) -> Self {
        Self {
            models: Vec::new(),
            last_refresh: None,
            omp_binary: omp_binary.to_owned(),
        }
    }

    /// Runs `<omp_binary> models --json` and parses the output.
    ///
    /// On any failure (binary not found, non-zero exit, malformed JSON, empty list)
    /// the existing model list is preserved and `Err` is returned.
    pub async fn refresh(&mut self) -> Result<(), String> {
        let binary = self.resolve_binary().await?;
        let mut cmd = command::r#async::Command::new(&binary);
        cmd.args(["models", "--json"]);

        let output = run_with_timeout(&mut cmd, 10).await?;

        let response: ModelsResponse = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("failed to parse models --json: {e}"))?;

        let models = response.models;

        if models.is_empty() {
            return Err("models --json returned empty list".into());
        }

        self.models = models;
        self.last_refresh = Some(Instant::now());
        self.omp_binary = binary;
        Ok(())
    }

    /// Resolve the `omp` binary path.
    /// When launched as a GUI app (e.g. from Finder), PATH may not include
    /// Homebrew paths like `/opt/homebrew/bin`. This method tries the binary
    /// name directly first, then falls back to common installation paths.
    async fn resolve_binary(&self) -> Result<String, String> {
        let candidates = if self.omp_binary.contains('/') {
            vec![self.omp_binary.clone()]
        } else {
            let mut c = vec![self.omp_binary.clone()];
            c.push("/opt/homebrew/bin/omp".into());
            c.push("/usr/local/bin/omp".into());
            c.push("/home/linuxbrew/.linuxbrew/bin/omp".into());
            c
        };
        for path in &candidates {
            if command::r#async::Command::new(path)
                .arg("--version")
                .stdout(command::Stdio::null())
                .stderr(command::Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(path.clone());
            }
        }
        Err(format!(
            "omp binary not found (tried: {})",
            candidates.join(", ")
        ))
    }

    /// Returns the cached model slice.
    /// Returns the cached model slice.
    pub fn models(&self) -> &[OmpModel] {
        &self.models
    }

    /// Runs `<omp_binary> config set modelRoles '{"default":"<selector>"}'`.
    pub async fn set_model(&self, selector: &str) -> Result<(), String> {
        use serde_json::json;
        let records = json!({"default": selector});
        let record_str = serde_json::to_string(&records)
            .map_err(|e| format!("serialize error: {e}"))?;

        let mut cmd = command::r#async::Command::new(&self.omp_binary);
        cmd.args(["config", "set", "modelRoles", &record_str]);

        run_with_timeout(&mut cmd, 10).await?;
        Ok(())
    }

    /// Runs `<omp_binary> config get modelRoles` and extracts the `default` field.
    pub async fn read_current_model(&self) -> Result<String, String> {
        let mut cmd = command::r#async::Command::new(&self.omp_binary);
        cmd.args(["config", "get", "modelRoles"]);

        let output = run_with_timeout(&mut cmd, 10).await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse the JSON record and extract the "default" field.
        let records: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("parse modelRoles error: {e}"))?;
        let default = records.get("default")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        Ok(default)
    }

}
