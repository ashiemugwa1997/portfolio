use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use sha2::{Digest, Sha256};

const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;

/// One file of the built site, held in memory with its precompressed copies.
#[derive(Debug, Clone)]
struct Asset {
    raw: Bytes,
    br: Option<Bytes>,
    gzip: Option<Bytes>,
    mime: &'static str,
    etag: HeaderValue,
    cache: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Brotli,
    Gzip,
    Identity,
}

impl Encoding {
    /// the best encoding the client accepts and we have
    fn pick(headers: &HeaderMap, asset: &Asset) -> Self {
        let accept = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("");
        let offers = |name: &str| {
            accept.split(',').any(|part| {
                let mut it = part.trim().split(';');
                it.next().is_some_and(|n| n.trim().eq_ignore_ascii_case(name)) && !it.any(|q| q.trim().replace(' ', "") == "q=0")
            })
        };
        if asset.br.is_some() && offers("br") {
            Encoding::Brotli
        } else if asset.gzip.is_some() && offers("gzip") {
            Encoding::Gzip
        } else {
            Encoding::Identity
        }
    }
}

/// The built React site, loaded once at start-up. Requests are answered from memory by
/// exact path, so nothing a client sends ever becomes a filesystem path.
#[derive(Debug, Default)]
pub struct StaticSite {
    files: HashMap<String, Asset>,
    index: Option<Asset>,
}

impl StaticSite {
    pub fn load(dir: &Path) -> Self {
        let mut site = StaticSite::default();
        let mut total = 0;
        let mut stack = vec![(dir.to_path_buf(), String::new())];
        while let Some((path, prefix)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let url = format!("{prefix}/{name}");
                if meta.is_dir() {
                    stack.push((entry.path(), url));
                    continue;
                }
                let ext = Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
                let precompressed = matches!(ext.as_deref(), Some("br" | "gz"));
                if precompressed || !meta.is_file() || meta.len() > MAX_FILE || total > MAX_TOTAL {
                    continue;
                }
                let Ok(raw) = std::fs::read(entry.path()) else { continue };
                total += raw.len();
                let sibling = |ext: &str| std::fs::read(entry.path().with_file_name(format!("{name}.{ext}"))).ok().map(Bytes::from);
                let digest = Sha256::digest(&raw);
                let etag = HeaderValue::from_str(&format!("\"{}\"", crate::store::hex(&digest[..8]))).expect("hex etag");
                let asset = Asset {
                    br: sibling("br"),
                    gzip: sibling("gz"),
                    mime: mime_for(&name),
                    etag,
                    cache: if url.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" },
                    raw: Bytes::from(raw),
                };
                site.files.insert(url, asset);
            }
        }
        site.index = site.files.get("/index.html").cloned();
        site
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    fn find(&self, path: &str) -> Option<&Asset> {
        if let Some(a) = self.files.get(path) {
            return Some(a);
        }
        // unknown paths under /assets/ are real 404s; anything else is a page of the app
        if path.starts_with("/assets/") || path.rsplit('/').next().is_some_and(|last| last.contains('.')) {
            return None;
        }
        self.index.as_ref()
    }
}

pub async fn serve(State(site): State<Arc<StaticSite>>, method: Method, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "GET, HEAD")]).into_response();
    }
    let path = if uri.path() == "/" { "/index.html" } else { uri.path() };
    let Some(asset) = site.find(path) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, asset.mime)
        .header(header::CACHE_CONTROL, asset.cache)
        .header(header::ETAG, asset.etag.clone())
        .header(header::VARY, "accept-encoding");
    if headers.get(header::IF_NONE_MATCH) == Some(&asset.etag) {
        return res.status(StatusCode::NOT_MODIFIED).body(Body::empty()).expect("valid response");
    }
    let body = match Encoding::pick(&headers, asset) {
        Encoding::Brotli => {
            res = res.header(header::CONTENT_ENCODING, "br");
            asset.br.clone().expect("picked only when present")
        }
        Encoding::Gzip => {
            res = res.header(header::CONTENT_ENCODING, "gzip");
            asset.gzip.clone().expect("picked only when present")
        }
        Encoding::Identity => asset.raw.clone(),
    };
    res = res.header(header::CONTENT_LENGTH, body.len());
    let body = if method == Method::HEAD { Body::empty() } else { Body::from(body) };
    res.body(body).expect("valid response")
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(br: bool, gzip: bool) -> Asset {
        Asset {
            raw: Bytes::from_static(b"x"),
            br: br.then(|| Bytes::from_static(b"b")),
            gzip: gzip.then(|| Bytes::from_static(b"g")),
            mime: "text/plain",
            etag: HeaderValue::from_static("\"e\""),
            cache: "no-cache",
        }
    }

    fn accept(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ACCEPT_ENCODING, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn negotiates_the_best_available_encoding() {
        assert_eq!(Encoding::pick(&accept("gzip, deflate, br"), &asset(true, true)), Encoding::Brotli);
        assert_eq!(Encoding::pick(&accept("gzip, br;q=0"), &asset(true, true)), Encoding::Gzip);
        assert_eq!(Encoding::pick(&accept("br"), &asset(false, true)), Encoding::Identity);
        assert_eq!(Encoding::pick(&HeaderMap::new(), &asset(true, true)), Encoding::Identity);
        assert_eq!(Encoding::pick(&accept("BR"), &asset(true, false)), Encoding::Brotli);
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for("engine.wasm"), "application/wasm");
        assert_eq!(mime_for("A.JS"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }
}
