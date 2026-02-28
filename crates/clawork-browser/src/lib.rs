use async_trait::async_trait;
use clawork_core::{BrowserAutomationService, BrowserRunRequest, BrowserRunResult};
use std::path::PathBuf;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use url::Url;

#[derive(Clone)]
pub struct PlaywrightSandbox {
    command: String,
    enable_exec: bool,
    work_dir: PathBuf,
}

impl Default for PlaywrightSandbox {
    fn default() -> Self {
        let command = std::env::var("CLAWORK_PLAYWRIGHT_CMD").unwrap_or_else(|_| "npx".into());
        let enable_exec = std::env::var("CLAWORK_ENABLE_BROWSER_EXEC")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            command,
            enable_exec,
            work_dir: PathBuf::from("data/browser"),
        }
    }
}

impl PlaywrightSandbox {
    fn validate_request(&self, req: &BrowserRunRequest) -> anyhow::Result<()> {
        let url = Url::parse(&req.url)?;
        let domain = url
            .domain()
            .ok_or_else(|| anyhow::anyhow!("url must include domain"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow::anyhow!("only http/https urls are allowed"));
        }

        if !req.allow_domains.is_empty()
            && !req
                .allow_domains
                .iter()
                .any(|allowed| domain.eq_ignore_ascii_case(allowed))
        {
            return Err(anyhow::anyhow!("domain '{domain}' is not in allow list"));
        }

        Ok(())
    }

    fn scripted_command(&self, req: &BrowserRunRequest) -> Vec<String> {
        let node_script = r#"const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch({ headless: process.argv[3] === 'true' });
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(process.argv[1], { waitUntil: 'domcontentloaded', timeout: Number(process.argv[2]) });
  const title = await page.title();
  console.log(JSON.stringify({ title, finalUrl: page.url() }));
  await browser.close();
})().catch((e) => { console.error(e.stack || String(e)); process.exit(1); });"#;

        vec![
            "-y".into(),
            "-p".into(),
            "playwright@latest".into(),
            "node".into(),
            "-e".into(),
            node_script.into(),
            req.url.clone(),
            ((req.timeout_seconds.max(1)) * 1000).to_string(),
            (!req.headed).to_string(),
        ]
    }
}

#[async_trait]
impl BrowserAutomationService for PlaywrightSandbox {
    async fn navigate(&self, req: BrowserRunRequest) -> anyhow::Result<BrowserRunResult> {
        self.validate_request(&req)?;
        tokio::fs::create_dir_all(&self.work_dir).await?;

        let args = self.scripted_command(&req);
        let cmd_line = format!("{} {}", self.command, args.join(" "));

        if !self.enable_exec {
            return Ok(BrowserRunResult {
                ok: true,
                command: cmd_line,
                stdout: "browser execution disabled; set CLAWORK_ENABLE_BROWSER_EXEC=1".into(),
                stderr: String::new(),
            });
        }

        let mut cmd = Command::new(&self.command);
        cmd.args(&args)
            .current_dir(&self.work_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd.spawn()?;
        let output = timeout(
            Duration::from_secs(req.timeout_seconds.max(10) + 10),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("browser command timed out"))??;

        Ok(BrowserRunResult {
            ok: output.status.success(),
            command: cmd_line,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}
