//! Matrix media module - handles media download endpoints

use poem::{
    handler,
    http::{header, StatusCode},
    web::{Data, Path},
    Error, Result,
};
use poem_openapi::payload::Response;

use crate::api::resource::FileMeta;
use crate::state::State;

/// Matrix media download endpoint
/// Handles GET /_matrix/media/v3/download/{server}/{media_id}
#[handler]
pub async fn download(
    state: Data<&State>,
    Path((server, media_id)): Path<(String, String)>,
) -> Result<Response<Vec<u8>>> {
    download_media(&state, &server, &media_id).await
}

/// Matrix media download endpoint with optional filename
/// Handles GET /_matrix/media/v3/download/{server}/{media_id}/{filename}
#[handler]
pub async fn download_with_filename(
    state: Data<&State>,
    path: Path<(String, String, String)>,
) -> Result<Response<Vec<u8>>> {
    let (server, media_id, _filename) = path.0;
    download_media(&state, &server, &media_id).await
}

/// Core download logic
async fn download_media(state: &State, server: &str, media_id: &str) -> Result<Response<Vec<u8>>> {
    // Get Matrix domain from config
    let matrix_domain = super::auth::get_matrix_domain(state);

    // Validate server name matches our domain
    if server != matrix_domain {
        return Err(Error::from_status(StatusCode::NOT_FOUND));
    }

    // Convert media_id back to file_path
    // The media_id was created by replacing '/' with '_' in the original file_path
    // e.g., "2025/01/30/uuid" -> "2025_01_30_uuid"
    let file_path = media_id.replace('_', "/");

    if file_path.is_empty() || file_path.contains("..") {
        return Err(Error::from_status(StatusCode::BAD_REQUEST));
    }

    // Build the file path
    let path = state.config.system.file_dir().join(&file_path);
    let path_meta = state
        .config
        .system
        .file_dir()
        .join(&file_path)
        .with_extension("meta");

    if !path.exists() {
        tracing::warn!("Media file not found: {}", file_path);
        return Err(Error::from_status(StatusCode::NOT_FOUND));
    }

    // Read file content
    let content = tokio::fs::read(&path).await.map_err(|e| {
        tracing::error!("Failed to read media file {}: {}", file_path, e);
        Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    // Read file metadata
    let meta = tokio::fs::read(&path_meta)
        .await
        .ok()
        .and_then(|data| serde_json::from_slice::<FileMeta>(&data).ok())
        .unwrap_or_else(|| FileMeta {
            content_type: "application/octet-stream".to_string(),
            filename: None,
        });

    // Build response with proper headers
    let mut response = Response::new(content);
    response = response.header(header::CONTENT_TYPE, meta.content_type);
    response = response.header(header::CACHE_CONTROL, "public, max-age=31536000");

    if let Some(filename) = meta.filename {
        response = response.header(
            header::CONTENT_DISPOSITION,
            format!(r#"inline; filename="{}""#, filename),
        );
    }

    Ok(response)
}
