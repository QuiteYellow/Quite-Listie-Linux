use reqwest::Client;
use url::Url;

use crate::model::Coordinate;

/// iPhone Mobile Safari UA. Swift uses this so Google + Apple consistently
/// redirect short URLs the same way they do for the iOS app — without it,
/// `maps.app.goo.gl` often refuses to resolve at all (see Swift
/// `LocationParser.resolveRedirect`, LocationParser.swift:95-118).
const IPHONE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
                        AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 \
                        Mobile/15E148 Safari/604.1";

/// Resolve a URL to a Coordinate by inspecting the URL itself and, if needed,
/// following redirects (for short URLs like goo.gl / maps.app.goo.gl).
///
/// Mirrors Swift `LocationParser.parseCoordinateWithSource`
/// (Services/LocationParser.swift). Strategies in order:
///   1. Plain `lat,lng` paste
///   2. Direct extraction from URL (data=!3d!4d, @lat,lng, ?q=, ll=, …)
///   3. `/place/<NAME>/` geocode with comma-suffix fallback (no redirect)
///   4. Follow redirect → unwrap consent page → retry 2, 3
///   5. Treat the original input as a place name and geocode
pub async fn resolve_location(input: &str, client: &Client) -> Option<Coordinate> {
    let input = input.trim();

    if let Some(coord) = parse_bare_coordinate(input) {
        return Some(coord);
    }

    let url = Url::parse(input).ok()?;

    if let Some(coord) = extract_from_url(&url) {
        return Some(coord);
    }

    // Google Maps URLs without embedded coords (CID-only `@0,0,22z`) carry the
    // location only in the `/place/<NAME>/` path segment. Try geocoding it
    // before paying the network cost of a redirect resolve.
    if let Some(name) = place_name_from_google_maps_path(&url) {
        if let Some(coord) = geocode_with_fallback(&name, client).await {
            return Some(coord);
        }
    }

    if needs_redirect_resolution(&url) {
        if let Some(resolved) = follow_redirect(input, client).await {
            if let Ok(mut resolved_url) = Url::parse(&resolved) {
                // Google sometimes redirects through `consent.google.com`
                // before reaching Maps — the real destination is the
                // `continue` query param. Mirrors LocationParser.swift:152-160.
                if let Some(real) = google_consent_continue(&resolved_url) {
                    resolved_url = real;
                }
                if let Some(coord) = extract_from_url(&resolved_url) {
                    return Some(coord);
                }
                if let Some(name) = place_name_from_google_maps_path(&resolved_url) {
                    if let Some(coord) = geocode_with_fallback(&name, client).await {
                        return Some(coord);
                    }
                }
            }
        }
    }

    geocode_nominatim(input, client).await
}

/// Convenience wrapper for GUI callers without their own reqwest client: builds a
/// default client, resolves `input`, and echoes the input back as the source URL only
/// when it parses as a URL (a bare `lat,lng` paste has no meaningful source). Mirrors the
/// KDE `parse_location_input` invokable.
pub async fn resolve_location_with_source(input: &str) -> Option<(Coordinate, String)> {
    let input = input.trim();
    let client = Client::builder()
        .user_agent("QuiteListie/0.1")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let coord = resolve_location(input, &client).await?;
    let source_url = if Url::parse(input).is_ok() {
        input.to_string()
    } else {
        String::new()
    };
    Some((coord, source_url))
}

/// Extract a human place name from a Google Maps URL's `/place/<NAME>/` segment, if any.
/// Mirrors Swift `LocationParser.parsePlaceName`; used to auto-fill an empty item name.
pub fn parse_place_name(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    place_name_from_google_maps_path(&url)
}

fn parse_bare_coordinate(s: &str) -> Option<Coordinate> {
    let s = s.replace(' ', "");
    let mut parts = s.splitn(2, ',');
    let lat: f64 = parts.next()?.parse().ok()?;
    let lng: f64 = parts.next()?.parse().ok()?;
    valid_coord(lat, lng)
}

fn valid_coord(lat: f64, lng: f64) -> Option<Coordinate> {
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) {
        Some(Coordinate { latitude: lat, longitude: lng, extra: Default::default() })
    } else {
        None
    }
}

fn extract_from_url(url: &Url) -> Option<Coordinate> {
    let host = url.host_str().unwrap_or("");
    let path = url.path();

    if host.contains("google.com") {
        for (k, v) in url.query_pairs() {
            if k == "q" {
                if let Some(coord) = parse_bare_coordinate(&v) {
                    return Some(coord);
                }
            }
        }
        // `data=!3d<lat>!4d<lng>` — actual pin coordinates, more accurate than
        // the @-viewport. Check first so it wins over @ when both are present.
        if let Some(lat) = extract_data_param(path, "3d") {
            if let Some(lng) = extract_data_param(path, "4d") {
                // Don't let `!3d0!4d0` placeholders sneak through.
                if lat != 0.0 || lng != 0.0 {
                    if let Some(c) = valid_coord(lat, lng) {
                        return Some(c);
                    }
                }
            }
        }
        // `@lat,lng,zoom` in path — viewport center fallback.
        if let Some(at_idx) = path.find('@') {
            let rest = &path[at_idx + 1..];
            let parts: Vec<&str> = rest.splitn(3, ',').collect();
            if parts.len() >= 2 {
                if let (Ok(lat), Ok(lng)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                    // Skip `@0,0,…` — Google uses this when the URL embeds
                    // only a CID and no real coordinates. Without this guard
                    // every CID share lands on Null Island.
                    if lat != 0.0 || lng != 0.0 {
                        if let Some(c) = valid_coord(lat, lng) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    }

    if host.contains("maps.apple.com") || host.contains("link.maps.apple.com") {
        for (k, v) in url.query_pairs() {
            if k == "ll" {
                if let Some(coord) = parse_bare_coordinate(&v) {
                    return Some(coord);
                }
            }
        }
    }

    if host.contains("openstreetmap.org") {
        let mut lat_opt = None;
        let mut lng_opt = None;
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "mlat" => lat_opt = v.parse::<f64>().ok(),
                "mlon" => lng_opt = v.parse::<f64>().ok(),
                _ => {}
            }
        }
        if let (Some(lat), Some(lng)) = (lat_opt, lng_opt) {
            return valid_coord(lat, lng);
        }
    }

    None
}

fn extract_data_param(path: &str, marker: &str) -> Option<f64> {
    let needle = format!("!{}", marker);
    let idx = path.find(&needle)?;
    let rest = &path[idx + needle.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Hosts whose URLs need to be resolved through a redirect before we can
/// extract a coordinate. Mirrors Swift `needsRedirectResolution`
/// (LocationParser.swift:85-91).
fn needs_redirect_resolution(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    host.contains("goo.gl")
        || host.contains("link.maps.apple.com")
        || host == "maps.apple"
        || (host.contains("maps.apple.com") && url.query().map_or(true, |q| q.is_empty()))
}

/// If `url` is a `consent.google.com` page, return the real destination
/// from its `continue` parameter. Mirrors `LocationParser.swift:152-160`.
fn google_consent_continue(url: &Url) -> Option<Url> {
    if !url.host_str().unwrap_or("").contains("consent.google.com") {
        return None;
    }
    let cont = url.query_pairs().find(|(k, _)| k == "continue")?.1;
    Url::parse(&cont).ok()
}

/// Extracts the place name from a Google Maps `/place/<NAME>/…` URL path.
/// Returns the percent-decoded name with `+` swapped for space, or `None`
/// if there is no such segment.
fn place_name_from_google_maps_path(url: &Url) -> Option<String> {
    if !url.host_str().unwrap_or("").contains("google.com") {
        return None;
    }
    let mut segs = url.path_segments()?;
    while let Some(seg) = segs.next() {
        if seg == "place" {
            let raw = segs.next()?;
            let decoded = urlencoding::decode(raw).ok()?;
            let cleaned = decoded.replace('+', " ");
            if cleaned.trim().is_empty() || cleaned == "@" {
                return None;
            }
            return Some(cleaned);
        }
    }
    None
}

/// Geocode a place name, trying progressively shorter comma-delimited
/// suffixes so e.g. "Short Stay Car Pk, Bristol BS48 3DY" falls back to
/// "Bristol BS48 3DY". Mirrors `LocationParser.swift:260-273`.
async fn geocode_with_fallback(name: &str, client: &Client) -> Option<Coordinate> {
    let parts: Vec<&str> = name.split(',').collect();
    let mut candidates: Vec<String> = vec![name.to_string()];
    for i in 1..parts.len() {
        let suffix = parts[i..].join(",").trim().to_string();
        if !suffix.is_empty() {
            candidates.push(suffix);
        }
    }
    for candidate in candidates {
        if let Some(coord) = geocode_nominatim(&candidate, client).await {
            return Some(coord);
        }
    }
    None
}

/// Follow a short URL's redirect chain and return the final URL.
/// GET (not HEAD) — `maps.app.goo.gl` doesn't always 30x on HEAD — with the
/// iPhone Safari UA Swift uses, so we get the same redirect target it does.
async fn follow_redirect(url: &str, client: &Client) -> Option<String> {
    let resp = client
        .get(url)
        .header("User-Agent", IPHONE_UA)
        .send()
        .await
        .ok()?;
    Some(resp.url().to_string())
}

async fn geocode_nominatim(query: &str, client: &Client) -> Option<Coordinate> {
    #[derive(serde::Deserialize)]
    struct NominatimResult {
        lat: String,
        lon: String,
    }

    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1",
        urlencoding::encode(query)
    );
    let results: Vec<NominatimResult> = client
        .get(&url)
        .header("User-Agent", "QuiteListie-KDE/0.1")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let first = results.into_iter().next()?;
    let lat: f64 = first.lat.parse().ok()?;
    let lng: f64 = first.lon.parse().ok()?;
    valid_coord(lat, lng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_coordinate_paste() {
        let c = parse_bare_coordinate("48.8584, 2.2945").unwrap();
        assert!((c.latitude - 48.8584).abs() < 1e-6);
        assert!((c.longitude - 2.2945).abs() < 1e-6);
    }

    #[test]
    fn bare_coordinate_rejects_out_of_range() {
        assert!(parse_bare_coordinate("100, 200").is_none());
        assert!(parse_bare_coordinate("-91, 0").is_none());
    }

    #[test]
    fn google_at_zero_zero_is_skipped() {
        // CID-only share URL — the `@0,0,22z` is a placeholder and must not
        // come back as Null Island. Swift skips it (parseGoogleMapsAt with
        // `lat != 0 || lng != 0`); we should match.
        let url = Url::parse(
            "https://www.google.com/maps/place/Some+Cafe/@0,0,22z/data=!4m2!3m1!1s0x:0x123"
        )
        .unwrap();
        assert!(extract_from_url(&url).is_none());
    }

    #[test]
    fn google_at_nonzero_is_kept() {
        let url = Url::parse(
            "https://www.google.com/maps/place/Some+Cafe/@51.5074,-0.1278,15z"
        )
        .unwrap();
        let c = extract_from_url(&url).unwrap();
        assert!((c.latitude - 51.5074).abs() < 1e-6);
        assert!((c.longitude + 0.1278).abs() < 1e-6);
    }

    #[test]
    fn data_param_wins_over_at() {
        // The pin lives at !3d!4d; @ is just the camera viewport. When both
        // are present we want the pin, not the viewport.
        let url = Url::parse(
            "https://www.google.com/maps/place/X/@51.5,-0.1,15z/data=!3d40.7128!4d-74.0060"
        )
        .unwrap();
        let c = extract_from_url(&url).unwrap();
        assert!((c.latitude - 40.7128).abs() < 1e-6);
        assert!((c.longitude + 74.0060).abs() < 1e-6);
    }

    #[test]
    fn data_zero_zero_skipped() {
        let url = Url::parse(
            "https://www.google.com/maps/place/X/data=!3d0!4d0"
        )
        .unwrap();
        assert!(extract_from_url(&url).is_none());
    }

    #[test]
    fn needs_redirect_resolution_matches_short_hosts() {
        assert!(needs_redirect_resolution(&Url::parse("https://maps.app.goo.gl/abc").unwrap()));
        assert!(needs_redirect_resolution(&Url::parse("https://goo.gl/maps/xyz").unwrap()));
        assert!(needs_redirect_resolution(&Url::parse("https://link.maps.apple.com/x").unwrap()));
        // Apple Maps with no query → needs resolution (carries no inline coord)
        assert!(needs_redirect_resolution(&Url::parse("https://maps.apple.com/?").unwrap()));
        // Apple Maps with ll= — already extractable, no redirect needed
        assert!(!needs_redirect_resolution(&Url::parse("https://maps.apple.com/?ll=1,2").unwrap()));
        assert!(!needs_redirect_resolution(&Url::parse("https://www.google.com/maps").unwrap()));
    }

    #[test]
    fn google_consent_continue_unwraps() {
        let consent = Url::parse(
            "https://consent.google.com/m?continue=https%3A%2F%2Fwww.google.com%2Fmaps%2Fplace%2FX%2F%4051.5%2C-0.1%2C15z&gl=GB"
        )
        .unwrap();
        let real = google_consent_continue(&consent).unwrap();
        assert_eq!(real.host_str(), Some("www.google.com"));
        // And the unwrapped URL should now be extractable.
        let c = extract_from_url(&real).unwrap();
        assert!((c.latitude - 51.5).abs() < 1e-6);
    }

    #[test]
    fn google_consent_ignores_non_consent_hosts() {
        let url = Url::parse("https://www.google.com/maps?continue=foo").unwrap();
        assert!(google_consent_continue(&url).is_none());
    }

    #[test]
    fn place_name_from_path() {
        let url = Url::parse(
            "https://www.google.com/maps/place/Short+Stay+Car+Pk%2C+Bristol+BS48+3DY/@0,0,22z"
        )
        .unwrap();
        assert_eq!(
            place_name_from_google_maps_path(&url).as_deref(),
            Some("Short Stay Car Pk, Bristol BS48 3DY")
        );
    }

    #[test]
    fn place_name_skips_at_placeholder() {
        // Some Google URLs have `/place/@/data=…` with no name segment.
        let url = Url::parse("https://www.google.com/maps/place/@/data=!3d1!4d2").unwrap();
        assert!(place_name_from_google_maps_path(&url).is_none());
    }

    #[test]
    fn place_name_only_google_hosts() {
        let url = Url::parse("https://maps.apple.com/place/X").unwrap();
        assert!(place_name_from_google_maps_path(&url).is_none());
    }

    #[test]
    fn geocode_fallback_candidates_shrink_left_to_right() {
        // Pure unit check of the candidate-generation logic (no network).
        // We can't easily test geocode_with_fallback without a stub, but we
        // can verify the suffix sequence directly using the same algorithm.
        let name = "A, B, C, D";
        let parts: Vec<&str> = name.split(',').collect();
        let mut candidates: Vec<String> = vec![name.to_string()];
        for i in 1..parts.len() {
            let suffix = parts[i..].join(",").trim().to_string();
            if !suffix.is_empty() {
                candidates.push(suffix);
            }
        }
        assert_eq!(
            candidates,
            vec![
                "A, B, C, D".to_string(),
                "B, C, D".to_string(),
                "C, D".to_string(),
                "D".to_string(),
            ]
        );
    }
}
