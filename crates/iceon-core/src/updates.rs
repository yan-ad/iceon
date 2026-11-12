//! Explicit advisory update checks. Never downloads or executes an installer.
use serde::Deserialize;
use std::time::Duration;

pub const FEED_URL: &str = "https://github.com/yan-ad/iceon/releases/latest/download/latest.json";
pub const RELEASES_URL: &str = "https://github.com/yan-ad/iceon/releases/latest";
const LIMIT: usize = 256 * 1024;
#[derive(Debug, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    pub available: bool,
}
#[derive(Deserialize)]
struct Feed {
    version: String,
}
fn parse(bytes: &[u8], current: &str) -> Result<UpdateCheck, String> {
    let feed: Feed =
        serde_json::from_slice(bytes).map_err(|_| "The update feed is invalid.".to_owned())?;
    let latest = semver::Version::parse(&feed.version)
        .map_err(|_| "The release version is invalid.".to_owned())?;
    let current = semver::Version::parse(current)
        .map_err(|_| "The installed version is invalid.".to_owned())?;
    if !latest.pre.is_empty() {
        return Err("The stable update feed contains a prerelease.".into());
    }
    Ok(UpdateCheck {
        available: latest > current,
        latest: latest.to_string(),
        current: current.to_string(),
    })
}
pub async fn check() -> Result<UpdateCheck, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Iceon/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not initialize update checking.".to_owned())?;
    let mut response = client
        .get(FEED_URL)
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your connection and try again.".to_owned())?
        .error_for_status()
        .map_err(|_| "The update feed is unavailable. Please try again later.".to_owned())?;
    if response
        .content_length()
        .is_some_and(|size| size > LIMIT as u64)
    {
        return Err("The update feed exceeds the size limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the update feed.".to_owned())?
    {
        if bytes.len() + chunk.len() > LIMIT {
            return Err("The update feed exceeds the size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes, env!("CARGO_PKG_VERSION"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_versions_not_strings() {
        assert!(
            parse(br#"{"version":"0.1.10"}"#, "0.1.9")
                .unwrap()
                .available
        );
        assert!(!parse(br#"{"version":"0.1.3"}"#, "0.1.3").unwrap().available);
        assert!(!parse(br#"{"version":"0.1.2"}"#, "0.1.3").unwrap().available);
    }
    #[test]
    fn rejects_invalid_and_prerelease_feeds() {
        for feed in [
            br#"{}"#.as_slice(),
            br#"{"version":"bad"}"#,
            br#"{"version":"0.2.0-beta.1"}"#,
        ] {
            assert!(parse(feed, "0.1.3").is_err());
        }
    }
}
