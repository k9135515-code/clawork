use anyhow::Context;
use base64::Engine;
use clawork_core::{OfficeExcelRequest, OfficeGraphUploadRequest, OfficeUploadResult};
use reqwest::Client;
use rust_xlsxwriter::Workbook;
use std::path::Path;

#[derive(Clone)]
pub struct OfficeService {
    client: Client,
}

impl Default for OfficeService {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl OfficeService {
    pub async fn create_excel_report(&self, req: OfficeExcelRequest) -> anyhow::Result<String> {
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name(&req.sheet_name)?;

        for (col, header) in req.headers.iter().enumerate() {
            worksheet.write_string(0, col as u16, header)?;
        }

        for (row_idx, row) in req.rows.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                worksheet.write_string((row_idx + 1) as u32, col_idx as u16, cell)?;
            }
        }

        if let Some(parent) = Path::new(&req.output_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        workbook
            .save(&req.output_path)
            .with_context(|| format!("failed to save xlsx: {}", req.output_path))?;

        Ok(req.output_path)
    }

    pub async fn upload_file_to_graph(
        &self,
        req: OfficeGraphUploadRequest,
    ) -> anyhow::Result<OfficeUploadResult> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(req.content_base64.as_bytes())
            .context("invalid base64 content")?;

        let endpoint = format!(
            "https://graph.microsoft.com/v1.0/me/drive/root:{}:/content",
            req.remote_path
        );

        let resp = self
            .client
            .put(endpoint)
            .bearer_auth(req.graph_token)
            .header("Content-Type", req.mime_type)
            .body(bytes)
            .send()
            .await
            .context("graph upload request failed")?;

        let status = resp.status();
        let response_body = resp.text().await.unwrap_or_default();

        Ok(OfficeUploadResult {
            ok: status.is_success(),
            status: status.as_u16(),
            response_body,
        })
    }
}
