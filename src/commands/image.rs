//! Inline images embedded in work item Description / Acceptance Criteria HTML
//! — the feature that lets agents see screenshots attached to bug reports.

use crate::cli::items::ImageCmd;
use crate::client::Clients;
use crate::context::Ctx;
use crate::domain::html;
use crate::error::CliError;
use crate::output::{Column, CommandOutput};
use serde_json::json;
use std::path::Path;

pub async fn run(cmd: &ImageCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        ImageCmd::List { id } => list(clients, project, *id).await,
        ImageCmd::Download { id, output_dir } => download(clients, project, *id, output_dir).await,
    }
}

async fn inline_images(
    clients: &Clients,
    project: &str,
    id: i32,
) -> Result<Vec<html::InlineImage>, CliError> {
    let item = clients
        .wit()
        .work_items_client()
        .get_work_item(&clients.org, id, project)
        .fields("System.Description,Microsoft.VSTS.Common.AcceptanceCriteria")
        .send()
        .await?
        .into_body()?;
    let value = serde_json::to_value(&item)?;
    let fields = value
        .get("fields")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    // Description first, then Acceptance Criteria (bash-script parity).
    let mut combined = String::new();
    for key in [
        "System.Description",
        "Microsoft.VSTS.Common.AcceptanceCriteria",
    ] {
        if let Some(text) = fields.get(key).and_then(|v| v.as_str()) {
            combined.push_str(text);
        }
    }
    Ok(html::extract_images(&combined))
}

async fn list(clients: &Clients, project: &str, id: i32) -> Result<CommandOutput, CliError> {
    let images = inline_images(clients, project, id).await?;
    Ok(CommandOutput::List {
        value: images
            .iter()
            .map(|i| serde_json::to_value(i).unwrap_or_default())
            .collect(),
        columns: vec![
            Column::new("Index", "index"),
            Column::new("URL", "url"),
            Column::new("Context", "context"),
        ],
    })
}

async fn download(
    clients: &Clients,
    project: &str,
    id: i32,
    output_dir: &Path,
) -> Result<CommandOutput, CliError> {
    let images = inline_images(clients, project, id).await?;
    if images.is_empty() {
        return Ok(CommandOutput::List {
            value: vec![],
            columns: vec![],
        });
    }
    std::fs::create_dir_all(output_dir)?;
    let raw = clients.raw();
    let mut results = Vec::new();
    for image in &images {
        // raw.download only sends credentials to dev.azure.com.
        let bytes = raw.download(&image.url).await?;
        let path = output_dir.join(format!(
            "wi{id}_img{}{}",
            image.index,
            extension_for(&image.url, &bytes)
        ));
        std::fs::write(&path, &bytes)?;
        results.push(json!({
            "index": image.index,
            "file": path.display().to_string(),
            "url": image.url,
            "context": image.context,
        }));
    }
    Ok(CommandOutput::List {
        value: results,
        columns: vec![
            Column::new("Index", "index"),
            Column::new("File", "file"),
            Column::new("Context", "context"),
        ],
    })
}

/// File extension from the URL's fileName= parameter, or content sniffing.
fn extension_for(url: &str, bytes: &[u8]) -> &'static str {
    let from_url = url
        .split("fileName=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .and_then(|name| name.rsplit('.').next())
        .map(|e| e.to_ascii_lowercase());
    match from_url.as_deref() {
        Some("png") => ".png",
        Some("jpg") | Some("jpeg") => ".jpg",
        Some("gif") => ".gif",
        Some("bmp") => ".bmp",
        Some("webp") => ".webp",
        Some("svg") => ".svg",
        _ => {
            if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                ".png"
            } else if bytes.starts_with(&[0xFF, 0xD8]) {
                ".jpg"
            } else if bytes.starts_with(b"GIF8") {
                ".gif"
            } else {
                ".png"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_from_url_or_magic_bytes() {
        assert_eq!(
            extension_for("https://x/_apis/wit/attachments/a?fileName=shot.PNG", &[]),
            ".png"
        );
        assert_eq!(extension_for("https://x/a", &[0xFF, 0xD8, 0xFF]), ".jpg");
        assert_eq!(extension_for("https://x/a", b"GIF89a"), ".gif");
        assert_eq!(extension_for("https://x/a", &[]), ".png");
    }
}
