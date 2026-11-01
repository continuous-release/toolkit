use anyhow::{Context, Result};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "GitHub API failed: {} {}",
            self.status, self.message
        )
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug)]
pub struct TagConflictError {
    pub tag: String,
    pub expected: String,
    pub actual: String,
}

impl fmt::Display for TagConflictError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "tag {} already points at {}, not {}",
            self.tag, self.actual, self.expected
        )
    }
}

impl std::error::Error for TagConflictError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseAsset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Release {
    pub id: u64,
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub html_url: String,
    pub upload_url: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitObject {
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub object: GitObject,
}

#[derive(Debug, Clone, Deserialize)]
struct AnnotatedTag {
    object: GitObject,
}

#[derive(Debug, Clone)]
pub struct ReleaseClientOptions {
    pub token: String,
    pub repository: String,
    pub api_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseClient {
    token: String,
    repository: String,
    api_url: String,
    http: Client,
}

#[derive(Debug, Clone)]
pub struct CreateRelease<'a> {
    pub tag: &'a str,
    pub target: Option<&'a str>,
    pub name: Option<&'a str>,
    pub body: &'a str,
    pub prerelease: bool,
    pub draft: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateRelease<'a> {
    pub name: Option<&'a str>,
    pub body: Option<&'a str>,
    pub prerelease: Option<bool>,
    pub draft: Option<bool>,
}

impl ReleaseClient {
    pub fn new(options: ReleaseClientOptions) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("building the HTTP client")?;

        Ok(Self {
            token: options.token,
            repository: options.repository,
            api_url: options
                .api_url
                .unwrap_or_else(|| "https://api.github.com".to_string())
                .trim_end_matches('/')
                .to_string(),
            http,
        })
    }

    fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.api_url, path))
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "continuous-release-toolkit");

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().context("calling the GitHub API")?;
        let status = response.status();

        if !status.is_success() {
            let message = response.text().unwrap_or_default();
            return Err(ApiError {
                status: status.as_u16(),
                message,
            }
            .into());
        }

        if status == StatusCode::NO_CONTENT {
            return serde_json::from_value(serde_json::Value::Null)
                .context("decoding an empty GitHub response");
        }

        response
            .json::<T>()
            .context("decoding the GitHub API response")
    }

    fn try_get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<Option<T>> {
        match self.request::<T>(Method::GET, path, None) {
            Ok(value) => Ok(Some(value)),
            Err(error) => match error.downcast_ref::<ApiError>() {
                Some(api) if api.status == 404 => Ok(None),
                _ => Err(error),
            },
        }
    }

    pub fn get_release_by_tag(&self, tag: &str) -> Result<Option<Release>> {
        self.try_get(&format!(
            "/repos/{}/releases/tags/{}",
            self.repository,
            urlencode(tag)
        ))
    }

    pub fn list_releases(&self) -> Result<Vec<Release>> {
        self.request(
            Method::GET,
            &format!("/repos/{}/releases?per_page=100", self.repository),
            None,
        )
    }

    pub fn create_release(&self, input: &CreateRelease<'_>) -> Result<Release> {
        self.request(
            Method::POST,
            &format!("/repos/{}/releases", self.repository),
            Some(json!({
                "tag_name": input.tag,
                "target_commitish": input.target,
                "name": input.name.unwrap_or(input.tag),
                "body": input.body,
                "draft": input.draft,
                "prerelease": input.prerelease,
            })),
        )
    }

    pub fn update_release(&self, id: u64, input: &UpdateRelease<'_>) -> Result<Release> {
        let mut body = serde_json::Map::new();
        if let Some(name) = input.name {
            body.insert("name".to_string(), json!(name));
        }
        if let Some(text) = input.body {
            body.insert("body".to_string(), json!(text));
        }
        if let Some(prerelease) = input.prerelease {
            body.insert("prerelease".to_string(), json!(prerelease));
        }
        if let Some(draft) = input.draft {
            body.insert("draft".to_string(), json!(draft));
        }

        self.request(
            Method::PATCH,
            &format!("/repos/{}/releases/{}", self.repository, id),
            Some(serde_json::Value::Object(body)),
        )
    }

    pub fn list_assets(&self, release_id: u64) -> Result<Vec<ReleaseAsset>> {
        self.request(
            Method::GET,
            &format!(
                "/repos/{}/releases/{}/assets?per_page=100",
                self.repository, release_id
            ),
            None,
        )
    }

    pub fn delete_asset(&self, asset_id: u64) -> Result<()> {
        self.request::<()>(
            Method::DELETE,
            &format!("/repos/{}/releases/assets/{}", self.repository, asset_id),
            None,
        )
    }

    pub fn upload_asset(
        &self,
        release: &Release,
        file: &Path,
        name: Option<&str>,
    ) -> Result<ReleaseAsset> {
        let name = name
            .map(str::to_string)
            .or_else(|| {
                file.file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .context("asset path has no file name")?;

        for asset in self
            .list_assets(release.id)?
            .into_iter()
            .filter(|asset| asset.name == name)
        {
            self.delete_asset(asset.id)?;
        }

        let base = release
            .upload_url
            .split('{')
            .next()
            .unwrap_or(&release.upload_url);
        let mut url = Url::parse(base).context("parsing the asset upload URL")?;
        url.query_pairs_mut().append_pair("name", &name);

        let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let response = self
            .http
            .post(url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "continuous-release-toolkit")
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .context("uploading the release asset")?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().unwrap_or_default();
            return Err(ApiError {
                status: status.as_u16(),
                message,
            }
            .into());
        }

        response.json().context("decoding the uploaded asset")
    }

    pub fn delete_release(&self, id: u64) -> Result<()> {
        self.request::<()>(
            Method::DELETE,
            &format!("/repos/{}/releases/{}", self.repository, id),
            None,
        )
    }

    pub fn delete_tag(&self, tag: &str) -> Result<()> {
        self.request::<()>(
            Method::DELETE,
            &format!(
                "/repos/{}/git/refs/tags/{}",
                self.repository,
                urlencode(tag)
            ),
            None,
        )
    }

    pub fn get_tag_ref(&self, tag: &str) -> Result<Option<GitRef>> {
        self.try_get(&format!(
            "/repos/{}/git/ref/tags/{}",
            self.repository,
            urlencode(tag)
        ))
    }

    pub fn resolve_tag_commit(&self, tag: &str) -> Result<Option<String>> {
        let Some(reference) = self.get_tag_ref(tag)? else {
            return Ok(None);
        };
        if reference.object.kind == "commit" {
            return Ok(Some(reference.object.sha));
        }
        let annotated: AnnotatedTag = self.request(
            Method::GET,
            &format!(
                "/repos/{}/git/tags/{}",
                self.repository, reference.object.sha
            ),
            None,
        )?;
        Ok(Some(annotated.object.sha))
    }

    pub fn create_tag_ref(&self, tag: &str, sha: &str) -> Result<GitRef> {
        self.request(
            Method::POST,
            &format!("/repos/{}/git/refs", self.repository),
            Some(json!({ "ref": format!("refs/tags/{tag}"), "sha": sha })),
        )
    }

    /// Reserve a version by creating its tag. Tag creation is atomic on the
    /// server, so the first run to reach it wins and every later run sees
    /// either the same commit (a re-run) or a different one (a lost race).
    pub fn reserve_tag(&self, tag: &str, sha: &str) -> Result<bool> {
        match self.create_tag_ref(tag, sha) {
            Ok(_) => return Ok(true),
            Err(error) => match error.downcast_ref::<ApiError>() {
                Some(api) if api.status == 422 => {}
                _ => return Err(error),
            },
        }

        let current = self
            .resolve_tag_commit(tag)?
            .with_context(|| format!("tag {tag} exists but could not be read"))?;
        if current != sha {
            return Err(TagConflictError {
                tag: tag.to_string(),
                expected: sha.to_string(),
                actual: current,
            }
            .into());
        }
        Ok(false)
    }
}

const TAG_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn urlencode(value: &str) -> String {
    utf8_percent_encode(value, TAG_ENCODE).to_string()
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn encodes_tag_names_like_encode_uri_component() {
        assert_eq!(urlencode("release/1.0.0"), "release%2F1.0.0");
        assert_eq!(urlencode("v1.2.3-rc.1"), "v1.2.3-rc.1");
        assert_eq!(urlencode("a b"), "a%20b");
    }
}
