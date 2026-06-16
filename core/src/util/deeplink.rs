//! `quitelistie://` URL parsing + generation.
//!
//! Wire-format parity with Swift (`DeeplinkCoordinator.swift`,
//! `DeeplinkCompression.swift`, `ShareLinkSheet.swift:187-219`). The payload is
//! the GFM markdown text the list exports — not the JSON document — so that
//! shared links round-trip with iOS.
//!
//! Important: Apple's `COMPRESSION_ZLIB` produces a raw DEFLATE stream (RFC
//! 1951) with no zlib header or trailer, despite the name. The Rust side must
//! therefore use [`flate2::write::DeflateEncoder`] /
//! [`flate2::read::DeflateDecoder`], **not** the `Zlib*` variants.
//!
//! Supported URLs:
//!   - `quitelistie://import?list=<uuid>&markdown=<encoded>&enc={zlib|b64|lzma}&preview={true|false}`
//!   - `quitelistie://item?id=<uuid>`
//!   - Legacy `listie://` scheme is accepted for both hosts (Swift bw-compat).

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use std::io::{Read, Write};
use url::Url;

/// Parsed action carried by a `quitelistie://` URL.
#[derive(Debug, Clone, PartialEq)]
pub enum DeeplinkAction {
    /// `quitelistie://import?...` — paste markdown into a (possibly missing) list.
    Import {
        /// Target list UUID hint. If `None` the caller must show a picker.
        list_id: Option<String>,
        /// Decoded markdown text.
        markdown: String,
        /// Whether the importer should show the preview workflow rather than
        /// committing immediately.
        preview: bool,
    },
    /// `quitelistie://item?id=<uuid>` — open the editor for a specific item.
    /// The caller is responsible for scanning open lists to locate it.
    Item { item_id: String },
}

/// Parse a `quitelistie://` (or legacy `listie://`) URL.
pub fn parse_url(url: &str) -> anyhow::Result<DeeplinkAction> {
    let parsed = Url::parse(url)?;
    let scheme = parsed.scheme();
    if scheme != "quitelistie" && scheme != "listie" {
        anyhow::bail!("unsupported scheme: {scheme}");
    }
    let host = parsed.host_str().unwrap_or("");
    match host {
        "import" => parse_import(&parsed),
        "item" => parse_item(&parsed),
        other => anyhow::bail!("unknown host: {other}"),
    }
}

fn parse_import(parsed: &Url) -> anyhow::Result<DeeplinkAction> {
    let mut list_id: Option<String> = None;
    let mut markdown_raw: Option<String> = None;
    let mut preview = false;
    let mut enc = "b64".to_string();
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "list" => list_id = Some(v.into_owned()),
            "markdown" => markdown_raw = Some(v.into_owned()),
            "preview" => preview = v == "true",
            "enc" => enc = v.into_owned(),
            _ => {}
        }
    }
    let raw = markdown_raw.ok_or_else(|| anyhow::anyhow!("missing markdown parameter"))?;
    let markdown = match enc.as_str() {
        "zlib" => deflate_b64url_decode(&raw)?,
        "b64" => {
            // Swift uses `Data(base64Encoded:)` (standard, with padding).
            // Accept both padded and unpadded for tolerance.
            let bytes = STANDARD
                .decode(raw.trim_end_matches('='))
                .or_else(|_| STANDARD.decode(&raw))
                .map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;
            String::from_utf8(bytes)?
        }
        "lzma" => anyhow::bail!("LZMA-compressed deeplinks are not supported on KDE yet"),
        other => anyhow::bail!("unknown enc parameter: {other}"),
    };
    Ok(DeeplinkAction::Import { list_id, markdown, preview })
}

fn parse_item(parsed: &Url) -> anyhow::Result<DeeplinkAction> {
    let item_id = parsed
        .query_pairs()
        .find_map(|(k, v)| (k == "id").then(|| v.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("item URL missing id"))?;
    Ok(DeeplinkAction::Item { item_id })
}

/// Build an `import` share URL. Mirrors Swift's `ShareLinkSheet.generateShareURL`.
/// When `compress` is true the markdown is raw-DEFLATE + Base64URL encoded
/// (`enc=zlib`); otherwise standard Base64 (`enc=b64`).
pub fn build_import_url(
    list_id: &str,
    markdown: &str,
    preview: bool,
    compress: bool,
) -> anyhow::Result<String> {
    let (encoded, enc) = if compress {
        (deflate_b64url_encode(markdown.as_bytes())?, "zlib")
    } else {
        (STANDARD.encode(markdown.as_bytes()), "b64")
    };
    Ok(format!(
        "quitelistie://import?list={list_id}&markdown={encoded}&enc={enc}&preview={preview}"
    ))
}

/// Build a `quitelistie://item?id=<uuid>` URL.
pub fn build_item_url(item_id: &str) -> String {
    format!("quitelistie://item?id={item_id}")
}

fn deflate_b64url_encode(data: &[u8]) -> anyhow::Result<String> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data)?;
    let compressed = enc.finish()?;
    Ok(URL_SAFE_NO_PAD.encode(&compressed))
}

fn deflate_b64url_decode(encoded: &str) -> anyhow::Result<String> {
    let compressed = URL_SAFE_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
        .map_err(|e| anyhow::anyhow!("base64url decode failed: {e}"))?;
    let mut dec = DeflateDecoder::new(&compressed[..]);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(String::from_utf8(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_import_zlib() {
        let md = "# Test\n\n- [ ] item one\n- [x] item two\n";
        let url = build_import_url("abc-123", md, true, true).unwrap();
        let parsed = parse_url(&url).unwrap();
        match parsed {
            DeeplinkAction::Import { list_id, markdown, preview } => {
                assert_eq!(list_id.as_deref(), Some("abc-123"));
                assert_eq!(markdown, md);
                assert!(preview);
            }
            other => panic!("expected Import, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_import_b64() {
        let md = "# Hi\n\n- [ ] x";
        let url = build_import_url("L", md, false, false).unwrap();
        let parsed = parse_url(&url).unwrap();
        match parsed {
            DeeplinkAction::Import { markdown, preview, .. } => {
                assert_eq!(markdown, md);
                assert!(!preview);
            }
            other => panic!("expected Import, got {other:?}"),
        }
    }

    #[test]
    fn parse_item_url() {
        let url = build_item_url("11111111-2222-3333-4444-555555555555");
        let parsed = parse_url(&url).unwrap();
        assert_eq!(
            parsed,
            DeeplinkAction::Item {
                item_id: "11111111-2222-3333-4444-555555555555".to_string()
            }
        );
    }

    #[test]
    fn legacy_listie_scheme_accepted() {
        let parsed = parse_url("listie://item?id=abc").unwrap();
        assert_eq!(parsed, DeeplinkAction::Item { item_id: "abc".to_string() });
    }

    #[test]
    fn rejects_zlib_wrapped_payload() {
        // A real raw-DEFLATE blob succeeds; one with a zlib header (0x78 0x9c …)
        // must fail because the DeflateDecoder doesn't strip wrappers. This
        // protects against a regression where someone re-introduces ZlibEncoder.
        use flate2::{write::ZlibEncoder, Compression};
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(b"hello").unwrap();
        let wrapped = enc.finish().unwrap();
        let url = format!(
            "quitelistie://import?list=x&markdown={}&enc=zlib",
            URL_SAFE_NO_PAD.encode(&wrapped)
        );
        // DeflateDecoder treats the zlib header as a corrupt stream — either
        // it errors or returns wrong bytes; in both cases we must not yield
        // "hello".
        match parse_url(&url) {
            Ok(DeeplinkAction::Import { markdown, .. }) => assert_ne!(markdown, "hello"),
            Err(_) => {}
            Ok(other) => panic!("unexpected: {other:?}"),
        }
    }
}
