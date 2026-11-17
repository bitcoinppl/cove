use crate::common::http_client_without_redirects;
use crate::version::StoreBuildNumber;
use color_eyre::eyre::{bail, Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

const EDITS_URL: &str =
    "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{package}/edits";
const SCOPE: &str = "https://www.googleapis.com/auth/androidpublisher";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// Google Play Developer API access for one package, authorized by a service account
pub(crate) struct PlayApi {
    client: Client,
    edits_url: String,
    token: String,
}

impl PlayApi {
    /// Exchange the service account key for an access token scoped to the Android Publisher API
    pub(crate) fn new(json_key_path: &str, package: &str) -> Result<Self> {
        let key: ServiceAccountKey = serde_json::from_slice(
            &std::fs::read(json_key_path).wrap_err("Failed to read Google Play service account")?,
        )
        .wrap_err("Invalid Google Play service account JSON")?;

        let client = http_client_without_redirects()?;
        let token = access_token(&client, &key)?;
        let edits_url = EDITS_URL.replace("{package}", package);

        Ok(Self { client, edits_url, token })
    }

    /// Every versionCode Google Play has accepted for the package, on any track or none
    ///
    /// Play rejects an upload that reuses any of these, including bundles that never reached a
    /// track, so the list comes from an edit's bundles and APKs rather than track releases
    pub(crate) fn used_version_codes(&self) -> Result<BTreeSet<u32>> {
        let edit: Edit = self.send(Method::POST, &self.edits_url)?.json()?;
        let edit_url = format!("{}/{}", self.edits_url, edit.id);
        let codes = self.version_codes_in(&edit_url);

        // the edit is only a read snapshot, so discard it instead of committing
        let deleted = self.send(Method::DELETE, &edit_url);
        let codes = codes?;
        deleted.wrap_err("Failed to discard the Google Play edit")?;

        Ok(codes)
    }

    /// The highest versionCode Google Play has accepted, or none for a package with no uploads
    pub(crate) fn highest_version_code(&self) -> Result<Option<StoreBuildNumber>> {
        Ok(self.used_version_codes()?.last().copied().map(StoreBuildNumber))
    }

    fn version_codes_in(&self, edit_url: &str) -> Result<BTreeSet<u32>> {
        let bundles: Bundles = self.send(Method::GET, &format!("{edit_url}/bundles"))?.json()?;
        let apks: Apks = self.send(Method::GET, &format!("{edit_url}/apks"))?.json()?;

        let bundle_codes = bundles.bundles.into_iter().map(|bundle| bundle.version_code);
        let apk_codes = apks.apks.into_iter().map(|apk| apk.version_code);

        Ok(bundle_codes.chain(apk_codes).collect())
    }

    fn send(&self, method: Method, url: &str) -> Result<Response> {
        let mut request =
            self.client.request(method.clone(), Url::parse(url)?).bearer_auth(&self.token);

        // Google answers a body-less POST with 411 Length Required
        if method == Method::POST {
            request = request.header(CONTENT_TYPE, "application/json").body("{}");
        }

        let response = request.send().wrap_err("Google Play request failed")?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let body = response.text().unwrap_or_default();
        bail!("Google Play {method} {url} failed ({status}): {body}");
    }
}

fn access_token(client: &Client, key: &ServiceAccountKey) -> Result<String> {
    let issued_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = Claims {
        iss: &key.client_email,
        scope: SCOPE,
        aud: &key.token_uri,
        iat: issued_at,
        exp: issued_at + 600,
    };

    let signing_key = EncodingKey::from_rsa_pem(key.private_key.as_bytes())
        .wrap_err("Invalid Google Play service account private key")?;
    let assertion = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &signing_key)
        .wrap_err("Failed to sign the Google Play token request")?;

    // reqwest's form feature is not enabled, so let Url do the form encoding
    let mut form = Url::parse("form:")?;
    form.query_pairs_mut()
        .append_pair("grant_type", JWT_BEARER_GRANT)
        .append_pair("assertion", &assertion);
    let body = form.query().unwrap_or_default().to_owned();

    let response = client
        .post(Url::parse(&key.token_uri)?)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .wrap_err("Google OAuth token request failed")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("Google OAuth token request failed ({status}): {body}");
    }

    let token: AccessToken = response.json().wrap_err("Invalid Google OAuth token response")?;
    Ok(token.access_token)
}

/// The fields of a Google service account key file that token signing needs
#[derive(Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

/// Google OAuth JWT bearer assertion claims
#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'static str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: String,
}

#[derive(Deserialize)]
struct Edit {
    id: String,
}

/// An edit's bundle list, which Play omits entirely when there are none
#[derive(Deserialize)]
struct Bundles {
    #[serde(default)]
    bundles: Vec<VersionCode>,
}

/// An edit's APK list, which Play omits entirely when there are none
#[derive(Deserialize)]
struct Apks {
    #[serde(default)]
    apks: Vec<VersionCode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionCode {
    version_code: u32,
}

#[cfg(test)]
mod tests {
    use super::{Apks, Bundles, PlayApi};
    use crate::version::StoreBuildNumber;

    #[test]
    fn reads_version_codes_from_play_lists_that_may_be_missing() {
        let bundles: Bundles =
            serde_json::from_str(r#"{"kind":"x","bundles":[{"versionCode":51,"sha1":"a"}]}"#)
                .unwrap();
        let empty: Apks = serde_json::from_str(r#"{"kind":"x"}"#).unwrap();

        assert_eq!(bundles.bundles[0].version_code, 51);
        assert!(empty.apks.is_empty());
    }

    // hits the live Play API: GOOGLE_PLAY_JSON_KEY_PATH=... cargo test -p xtask live_play -- --ignored
    #[test]
    #[ignore = "needs Google Play credentials and network"]
    fn live_play_reports_the_highest_version_code() {
        let key = std::env::var("GOOGLE_PLAY_JSON_KEY_PATH").unwrap();
        let api = PlayApi::new(&key, "org.bitcoinppl.cove").unwrap();

        let highest = api.highest_version_code().unwrap();
        println!("highest Play versionCode: {highest:?}");

        assert!(highest >= Some(StoreBuildNumber(51)));
    }
}
