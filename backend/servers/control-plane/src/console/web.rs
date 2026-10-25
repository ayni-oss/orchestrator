//! Optional prebuilt browser assets share the contributor gateway's authority boundary.
use super::Console;
use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse as _, Redirect, Response},
};
use std::{collections::BTreeMap, fs, io, io::Read as _, path::Path as FilePath};

const MAX_BUILD_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct WebAssets(BTreeMap<String, Asset>);

#[derive(Clone)]
struct Asset {
    media_type: &'static str,
    bytes: Vec<u8>,
}

impl WebAssets {
    pub(super) fn load(directory: &FilePath) -> io::Result<Self> {
        let mut assets = Self(BTreeMap::new());
        let mut remaining = MAX_BUILD_BYTES;
        assets.read(directory, "", 0, &mut remaining)?;
        if !assets.0.contains_key("index.html") {
            return Err(io::Error::other(
                "web build needs index.html; run pnpm web:build",
            ));
        }
        Ok(assets)
    }

    fn read(
        &mut self,
        directory: &FilePath,
        prefix: &str,
        depth: usize,
        remaining: &mut u64,
    ) -> io::Result<()> {
        if depth > 4 {
            return Err(io::Error::other(
                "web build directory nesting exceeds limit",
            ));
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| io::Error::other("web asset name must be UTF-8"))?;
            let key = format!("{prefix}{name}");
            if kind.is_symlink() {
                return Err(io::Error::other("web build must not contain symlinks"));
            }
            if kind.is_dir() {
                self.read(&entry.path(), &format!("{key}/"), depth + 1, remaining)?;
            } else if kind.is_file() {
                let media_type = match entry.path().extension().and_then(|value| value.to_str()) {
                    Some("html") => "text/html; charset=utf-8",
                    Some("js" | "mjs") => "text/javascript; charset=utf-8",
                    Some("css") => "text/css; charset=utf-8",
                    Some("svg") => "image/svg+xml",
                    _ => return Err(io::Error::other("unsupported web build asset type")),
                };
                let mut bytes = Vec::new();
                fs::File::open(entry.path())?
                    .take(*remaining + 1)
                    .read_to_end(&mut bytes)?;
                let length = u64::try_from(bytes.len()).map_err(io::Error::other)?;
                *remaining = remaining
                    .checked_sub(length)
                    .ok_or_else(|| io::Error::other("web build exceeds 16 MiB"))?;
                self.0.insert(key, Asset { media_type, bytes });
            } else {
                return Err(io::Error::other("web build must contain regular files"));
            }
        }
        Ok(())
    }

    fn response(&self, path: &str) -> Response {
        self.0.get(path).map_or_else(
            || StatusCode::NOT_FOUND.into_response(),
            |asset| {
                (
                    [(header::CONTENT_TYPE, asset.media_type)],
                    asset.bytes.clone(),
                )
                    .into_response()
            },
        )
    }
}

pub(super) async fn redirect() -> Redirect {
    Redirect::temporary("/workspace/")
}

pub(super) async fn index(State(console): State<Console>) -> Response {
    respond(&console, "index.html")
}

pub(super) async fn asset(State(console): State<Console>, Path(path): Path<String>) -> Response {
    respond(&console, &path)
}

fn respond(console: &Console, path: &str) -> Response {
    console.web_assets.as_ref().map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, "Workspace assets are not loaded. Use dev/ctl web, or run pnpm web:build and pass apps/web/build as the gateway's third argument.").into_response(),
        |assets| assets.response(path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    type Result = std::result::Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn built_workspace_assets_use_the_gateway_security_boundary() -> Result {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("index.html"), "<h1>Workspace</h1>")?;
        fs::create_dir(directory.path().join("assets"))?;
        fs::write(directory.path().join("assets/app.js"), "export {};")?;
        let console = Console::new("127.0.0.1:3000".parse()?, "127.0.0.1:50051".parse()?)?
            .with_web_assets(directory.path())?;
        let app = super::super::router(console);
        for (path, status, media) in [
            ("/workspace/", StatusCode::OK, "text/html; charset=utf-8"),
            (
                "/workspace/assets/app.js",
                StatusCode::OK,
                "text/javascript; charset=utf-8",
            ),
            ("/workspace/missing.js", StatusCode::NOT_FOUND, ""),
            ("/workspace/%2e%2e/Cargo.toml", StatusCode::NOT_FOUND, ""),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", "127.0.0.1:3000")
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(
                response.headers()[header::CONTENT_SECURITY_POLICY]
                    .to_str()?
                    .contains("script-src 'self'")
            );
            if !media.is_empty() {
                assert_eq!(response.headers()[header::CONTENT_TYPE], media);
            }
        }
        let forbidden = app
            .oneshot(
                Request::builder()
                    .uri("/workspace/")
                    .header("host", "foreign.example")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[tokio::test]
    async fn missing_assets_explain_the_build_prerequisite() -> Result {
        let console = Console::new("127.0.0.1:3000".parse()?, "127.0.0.1:50051".parse()?)?;
        let response = index(State(console)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await?.to_bytes();
        assert!(std::str::from_utf8(&body)?.contains("dev/ctl web"));
        Ok(())
    }

    #[test]
    fn build_requires_an_index_and_rejects_external_symlinks() -> Result {
        let directory = tempfile::tempdir()?;
        assert!(WebAssets::load(directory.path()).is_err());
        fs::write(directory.path().join("index.html"), "Workspace")?;
        assert!(WebAssets::load(directory.path()).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", directory.path().join("outside.html"))?;
            assert!(WebAssets::load(directory.path()).is_err());
        }
        Ok(())
    }
}
