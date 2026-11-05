//! Anonymous search and limited progressive media metadata. YouTube's HTML schema is unofficial and may change.
//! Requests never fall back to a direct connection, and redirects are not followed.
mod media;
mod playback;
pub use playback::PlaybackSession;
pub use media::{PreparedVideo, MediaStream};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{collections::HashSet, net::SocketAddr, time::Duration};

const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: String,
}

#[derive(Clone)]
pub struct SearchClient(reqwest::Client, reqwest::Client);

impl SearchClient {
    /// An explicit loopback proxy is mandatory; system/environment proxies are ignored.
    pub fn new(proxy: SocketAddr) -> Result<Self> {
        if !proxy.ip().is_loopback() { bail!("search requires a loopback proxy"); }
        let builder = || -> Result<reqwest::ClientBuilder> {
            Ok(reqwest::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(format!("http://{proxy}"))?)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(8))
                .read_timeout(Duration::from_secs(20))
                .user_agent("Mozilla/5.0 LumenTube/0.1"))
        };
        Ok(Self(builder()?.timeout(Duration::from_secs(25)).build()?, builder()?.build()?))
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Video>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 200 { bail!("query must contain 1–200 characters"); }
        let mut url = reqwest::Url::parse("https://www.youtube.com/results")?;
        url.query_pairs_mut().append_pair("search_query", query).append_pair("hl", "en");
        let body = self.fetch_html(url).await?;
        parse_search(&body)
    }

    async fn fetch_html(&self, url: reqwest::Url) -> Result<String> {
        let mut response = self.0.get(url).send().await?.error_for_status()?;
        if response.status().is_redirection() { bail!("YouTube redirected the request; consent or login may be required"); }
        if response.content_length().is_some_and(|size| size > MAX_BODY as u64) { bail!("HTML response too large"); }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_BODY { bail!("HTML response too large"); }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body).context("HTML response is not UTF-8")
    }

}

/// Find a JSON value after a known script assignment. serde's streaming parser handles
/// braces and escaped quotes in strings; no brittle brace-counting or greedy regex.
fn initial_data(html: &str) -> Result<Value> {
    script_object(html, &["var ytInitialData =", "window[\"ytInitialData\"] =", "ytInitialData ="])
}

fn script_object(html: &str, markers: &[&str]) -> Result<Value> {
    if html.len() > MAX_BODY { bail!("search response too large"); }
    for marker in markers {
        for (offset, _) in html.match_indices(*marker) {
            let tail = html[offset + marker.len()..].trim_start();
            let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<Value>();
            if let Some(Ok(value)) = stream.next() {
                if value.is_object() { return Ok(value); }
            }
        }
    }
    bail!("YouTube script data missing or changed; consent/challenge pages are not bypassed")
}

pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn text(value: &Value) -> String {
    if let Some(simple) = value.get("simpleText").and_then(Value::as_str) { return simple.chars().take(500).collect(); }
    value.get("runs").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .flat_map(str::chars).take(500).collect()
}

pub fn parse_search(html: &str) -> Result<Vec<Video>> {
    let data = initial_data(html)?;
    // Deliberately ignore sidebars, promoted content and recommendation trees.
    let contents = data.pointer("/contents/twoColumnSearchResultsRenderer/primaryContents/sectionListRenderer/contents")
        .and_then(Value::as_array).context("unsupported YouTube search layout")?;
    let mut videos = Vec::new();
    let mut seen = HashSet::new();
    for section in contents {
        for item in section.pointer("/itemSectionRenderer/contents").and_then(Value::as_array).into_iter().flatten() {
            let Some(renderer) = item.get("videoRenderer") else { continue; };
            let Some(id) = renderer.get("videoId").and_then(Value::as_str) else { continue; };
            if !valid_video_id(id) { continue; }
            let title = text(&renderer["title"]);
            if title.is_empty() || !seen.insert(id.to_owned()) { continue; }
            videos.push(Video {
                id: id.to_owned(), title,
                channel: text(&renderer["ownerText"]),
                duration: text(&renderer["lengthText"]),
            });
            if videos.len() == MAX_RESULTS { return Ok(videos); }
        }
    }
    Ok(videos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(items: Vec<Value>) -> String {
        let data = json!({"contents":{"twoColumnSearchResultsRenderer":{"primaryContents":{
            "sectionListRenderer":{"contents":[{"itemSectionRenderer":{"contents":items}}]}
        }}}});
        format!("<script>var ytInitialData = {data};</script>")
    }
    fn video(id: &str) -> Value {
        json!({"videoRenderer":{"videoId":id,"title":{"runs":[{"text":"Rust {"},{"text":" & Android"}]},
            "ownerText":{"simpleText":"Channel"},"lengthText":{"simpleText":"12:34"}}})
    }
    #[test]
    fn parses_and_deduplicates_video_results() {
        let result = parse_search(&page(vec![video("abcdefghijk"), video("abcdefghijk"), json!({"adSlotRenderer":{}})])).unwrap();
        assert_eq!(result, vec![Video { id: "abcdefghijk".into(), title: "Rust { & Android".into(), channel: "Channel".into(), duration: "12:34".into() }]);
    }
    #[test]
    fn ignores_invalid_ids_and_empty_titles() {
        let mut empty = video("abcdefghijk");
        empty["videoRenderer"]["title"] = json!({});
        assert!(parse_search(&page(vec![video("../bad-id"), empty])).unwrap().is_empty());
    }
    #[test]
    fn distinguishes_empty_results_from_changed_schema() {
        assert!(parse_search(&page(vec![])).unwrap().is_empty());
        assert!(parse_search("<html>Consent required</html>").is_err());
        assert!(parse_search("var ytInitialData = {};").is_err());
    }
    #[test]
    fn rejects_oversize_pages_and_remote_proxy() {
        assert!(parse_search(&"x".repeat(MAX_BODY + 1)).is_err());
        assert!(SearchClient::new("192.168.1.2:8080".parse().unwrap()).is_err());
    }
    #[tokio::test]
    async fn closed_proxy_does_not_fall_back_to_direct_network() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        assert!(SearchClient::new(address).unwrap().search("rust").await.is_err());
    }
}
