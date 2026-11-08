//! Limited progressive MP4 resolver. Does not execute player JS, decipher signatures,
//! merge DASH streams or bypass authentication. Signed URLs are never shown in UI.
use super::{script_object, valid_video_id, SearchClient};
use anyhow::{bail, Context, Result};
use reqwest::{header, StatusCode, Url};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedVideo {
    pub video_id: String,
    pub title: String,
    pub quality: String,
    pub mime: String,
}

/// Intentionally no Debug implementation: URL contains temporary access tokens.
#[derive(Clone)]
pub struct MediaStream {
    pub metadata: PreparedVideo,
    url: Url,
    expires: u64,
}

fn current_time() -> Result<u64> { Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()) }

fn media_url(value: &str, now: u64) -> Result<(Url, u64)> {
    if value.len() > 16384 { bail!("media URL too long"); }
    let url = Url::parse(value).context("invalid media URL")?;
    let host = url.host_str().context("media hostname missing")?;
    if url.scheme() != "https" || !host.ends_with(".googlevideo.com")
        || url.port_or_known_default() != Some(443) || !url.username().is_empty()
        || url.password().is_some() || url.fragment().is_some() || url.path() != "/videoplayback" {
        bail!("media URL is not an approved HTTPS googlevideo endpoint");
    }
    let mut expires = None;
    for (key, value) in url.query_pairs() {
        if key == "n" { bail!("player n-parameter transform is not supported"); }
        if key == "expire" {
            if expires.is_some() { bail!("ambiguous media expiry"); }
            expires = Some(value.parse::<u64>().context("invalid media expiry")?);
        }
    }
    let expires = expires.context("media expiry missing")?;
    if expires <= now.saturating_add(30) { bail!("media URL expired or expires too soon"); }
    Ok((url, expires))
}

fn parse_player(html: &str, id: &str, now: u64) -> Result<Vec<MediaStream>> {
    if !valid_video_id(id) { bail!("invalid video ID"); }
    let data = script_object(html, &["var ytInitialPlayerResponse =", "window[\"ytInitialPlayerResponse\"] =", "ytInitialPlayerResponse ="])?;
    if data.pointer("/playabilityStatus/status").and_then(|v| v.as_str()) != Some("OK") {
        bail!("video is unavailable or requires authentication/consent");
    }
    if data.pointer("/videoDetails/videoId").and_then(|v| v.as_str()) != Some(id) { bail!("player video ID mismatch"); }
    if data.pointer("/streamingData/licenseInfos").is_some()
        || data.pointer("/videoDetails/isLiveContent").and_then(|v| v.as_bool()) == Some(true) {
        bail!("DRM and live streams are not supported");
    }
    let title: String = data.pointer("/videoDetails/title").and_then(|v| v.as_str()).unwrap_or(id).chars().take(500).collect();
    let formats = data.pointer("/streamingData/formats").and_then(|v| v.as_array())
        .context("progressive formats missing; adaptive/DASH playback is not implemented")?;
    let mut streams = Vec::new();
    for format in formats.iter().take(100) {
        if format.get("signatureCipher").is_some() || format.get("cipher").is_some() || format.get("drmFamilies").is_some() { continue; }
        let mime = format.get("mimeType").and_then(|v| v.as_str()).unwrap_or_default();
        if mime.split(';').next() != Some("video/mp4") || !mime.contains("avc1") || !mime.contains("mp4a")
            || format.get("audioQuality").and_then(|v| v.as_str()).is_none() { continue; }
        let Some(value) = format.get("url").and_then(|v| v.as_str()) else { continue; };
        // An unsupported candidate must not suppress other usable formats.
        let Ok((url, expires)) = media_url(value, now) else { continue; };
        streams.push(MediaStream { metadata: PreparedVideo {
            video_id: id.into(), title: title.clone(), mime: mime.chars().take(200).collect(),
            quality: format.get("qualityLabel").and_then(|v| v.as_str()).unwrap_or("unknown").chars().take(40).collect(),
        }, url, expires });
    }
    if streams.is_empty() { bail!("no supported direct progressive MP4; cipher, n-transform, DRM and adaptive streams are not implemented"); }
    Ok(streams)
}

fn validate_range(status: StatusCode, headers: &header::HeaderMap) -> Result<()> {
    if status != StatusCode::PARTIAL_CONTENT { bail!("CDN did not honor the one-byte Range request"); }
    let range = headers.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).context("CDN Content-Range missing")?;
    let total = range.strip_prefix("bytes 0-0/").context("unexpected CDN Content-Range")?.parse::<u64>()?;
    if total == 0 { bail!("empty media object"); }
    let mime = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
    if mime.split(';').next() != Some("video/mp4") { bail!("CDN response is not video/mp4"); }
    if headers.get(header::CONTENT_LENGTH).is_some_and(|v| v.to_str().ok() != Some("1")) { bail!("unexpected CDN byte count"); }
    Ok(())
}

impl SearchClient {
    /// Fetch watch metadata only through the mandatory local proxy.
    pub async fn resolve_video(&self, id: &str) -> Result<Vec<MediaStream>> {
        if !valid_video_id(id) { bail!("invalid video ID"); }
        let mut url = Url::parse("https://www.youtube.com/watch")?;
        url.query_pairs_mut().append_pair("v", id).append_pair("hl", "en");
        parse_player(&self.fetch_html(url).await?, id, current_time()?)
    }

    /// Validate an actual media byte, not just a successful metadata request.
    /// Does not start playback or hand a signed URL to another networking stack.
    pub async fn inspect_stream(&self, stream: &MediaStream) -> Result<PreparedVideo> {
        if stream.expires <= current_time()?.saturating_add(30) { bail!("media URL expired"); }
        let mut response = self.0.get(stream.url.clone()).header(header::RANGE, "bytes=0-0")
            .send().await.map_err(|error| { self.note_transport_error(&error); error.without_url() })?;
        validate_range(response.status(), response.headers())?;
        let mut received = 0;
        while let Some(bytes) = response.chunk().await.map_err(|error| { self.note_transport_error(&error); error.without_url() })? {
            received += bytes.len();
            if received > 1 { bail!("CDN sent more than one requested byte"); }
        }
        if received != 1 { bail!("CDN returned an empty body"); }
        Ok(stream.metadata.clone())
    }

    pub async fn prepare_stream(&self, id: &str) -> Result<MediaStream> {
        // Bound CDN checks and stop at the first working progressive format.
        for stream in self.resolve_video(id).await?.iter().take(3) {
            if self.inspect_stream(stream).await.is_ok() { return Ok(stream.clone()); }
        }
        bail!("supported metadata found, but CDN byte checks failed through the proxy")
    }

    pub async fn prepare_video(&self, id: &str) -> Result<PreparedVideo> {
        Ok(self.prepare_stream(id).await?.metadata)
    }

    pub(crate) async fn open_media(&self, stream: &MediaStream, range: Option<&str>, head: bool) -> Result<reqwest::Response> {
        if stream.expires <= current_time()?.saturating_add(5) { bail!("media URL expired"); }
        let mut request = if head { self.1.head(stream.url.clone()) } else { self.1.get(stream.url.clone()) };
        if let Some(range) = range { request = request.header(header::RANGE, range); }
        request.send().await.map_err(|error| { self.note_transport_error(&error); error.without_url() }).map_err(Into::into)
    }

}

#[cfg(test)]
pub(crate) fn fixture_stream() -> MediaStream {
    MediaStream {
        metadata: PreparedVideo { video_id: "abcdefghijk".into(), title: "Fixture".into(), quality: "360p".into(), mime: "video/mp4".into() },
        url: Url::parse("https://rr1.googlevideo.com/videoplayback?expire=9999999999&sig=must-not-leak").unwrap(),
        expires: 9999999999,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    const ID: &str = "abcdefghijk";
    fn format(url: &str) -> Value {
        json!({"url":url,"mimeType":"video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", "audioQuality":"AUDIO_QUALITY_MEDIUM", "qualityLabel":"360p"})
    }
    fn page(formats: Vec<Value>) -> String {
        let data = json!({"playabilityStatus":{"status":"OK"}, "videoDetails":{"videoId":ID,"title":"Fixture"},"streamingData":{"formats":formats}});
        format!("<script>var ytInitialPlayerResponse = {data};</script>")
    }
    #[test]
    fn parses_only_usable_progressive_formats() {
        let good = "https://rr1.googlevideo.com/videoplayback?expire=2000&sig=secret";
        let mut cipher = format(good); cipher["signatureCipher"] = json!("secret");
        let result = parse_player(&page(vec![cipher,format(good)]), ID, 1000).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].metadata.quality, "360p");
    }
    #[test]
    fn rejects_untrusted_or_unsupported_urls() {
        for value in ["http://rr1.googlevideo.com/videoplayback?expire=2000", "https://googlevideo.com.evil.test/videoplayback?expire=2000", "https://127.0.0.1/videoplayback?expire=2000", "https://user:pass@rr1.googlevideo.com/videoplayback?expire=2000", "https://rr1.googlevideo.com:444/videoplayback?expire=2000", "https://rr1.googlevideo.com/videoplayback?expire=1000", "https://rr1.googlevideo.com/videoplayback?expire=2000&n=abc", "https://rr1.googlevideo.com/videoplayback?expire=2000&expire=3000"] {
            assert!(media_url(value, 1000).is_err());
        }
    }
    #[test]
    fn rejects_wrong_video_and_challenge() {
        assert!(parse_player(&page(vec![]), "zyxwvutsrqp", 1000).is_err());
        assert!(parse_player("var ytInitialPlayerResponse = {\"playabilityStatus\":{\"status\":\"LOGIN_REQUIRED\"}};", ID, 1000).is_err());
        assert!(parse_player("consent", ID, 1000).is_err());
        assert!(parse_player(&page(vec![]), "bad", 1000).is_err());
    }
    #[test]
    fn validates_range_status_type_and_size() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 0-0/1024".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "1".parse().unwrap());
        assert!(validate_range(StatusCode::PARTIAL_CONTENT, &headers).is_ok());
        assert!(validate_range(StatusCode::OK, &headers).is_err());
        headers.insert(header::CONTENT_RANGE, "bytes 1-1/1024".parse().unwrap());
        assert!(validate_range(StatusCode::PARTIAL_CONTENT, &headers).is_err());
        headers.insert(header::CONTENT_RANGE, "bytes 0-0/1024".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "text/html".parse().unwrap());
        assert!(validate_range(StatusCode::PARTIAL_CONTENT, &headers).is_err());
    }
}
