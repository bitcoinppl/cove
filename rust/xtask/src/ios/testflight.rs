use crate::common::{http_client_without_redirects, print_info, print_success, print_warning};
use color_eyre::eyre::{bail, ensure, eyre, Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, Url};
use serde::{
    de::{DeserializeOwned, IgnoredAny},
    Deserialize, Serialize,
};
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://api.appstoreconnect.apple.com/";
const GROUP_NAME: &str = "me-only";
const PROCESSING_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// The version and build number read from the uploaded archive
pub(crate) struct BuildIdentity {
    /// The app marketing version
    pub(crate) version: String,
    /// The app build number
    pub(crate) build_number: String,
}

/// A validated destination and a snapshot of the previous build's test notes
pub(crate) struct Distribution {
    api: Api,
    app_id: String,
    group: Resource<Group>,
    notes: Vec<TestDescription>,
}

impl Distribution {
    /// Validate the destination and save the latest valid iOS build's descriptions before upload
    pub(crate) fn prepare(
        key_path: &str,
        key_id: &str,
        issuer_id: &str,
        bundle_id: &str,
    ) -> Result<Self> {
        let key =
            std::fs::read(key_path).wrap_err("Failed to read App Store Connect signing key")?;

        let api = Api::new(&key, key_id, issuer_id)?;
        Self::prepare_with_api(api, bundle_id)
    }

    fn prepare_with_api(api: Api, bundle_id: &str) -> Result<Self> {
        let apps: Vec<Resource<App>> = api.list("v1/apps", &[("filter[bundleId]", bundle_id)])?;
        let mut apps = apps.into_iter().filter(|app| app.attributes.bundle_id == bundle_id);
        let app = apps.next().ok_or_else(|| eyre!("App not found: {bundle_id}"))?;

        ensure!(apps.next().is_none(), "Multiple apps match {bundle_id}");

        let groups: Vec<Resource<Group>> =
            api.list("v1/betaGroups", &[("filter[app]", &app.id), ("filter[name]", GROUP_NAME)])?;

        let group = exact_group(groups)?;
        let builds: Page<Build> = api
            .get(api.url(
                "v1/builds",
                &[
                    ("filter[app]", &app.id),
                    ("filter[preReleaseVersion.platform]", "IOS"),
                    ("filter[processingState]", "VALID"),
                    ("sort", "-uploadedDate"),
                    ("limit", "1"),
                ],
            )?)?
            .json()
            .wrap_err("Invalid previous-build response")?;

        let previous = builds.data.first().ok_or_else(|| {
            eyre!("No previous valid iOS build from which to copy test descriptions")
        })?;

        ensure!(
            previous.attributes.processing_state == ProcessingState::Valid,
            "Previous build is not valid"
        );

        let notes: Vec<Resource<Localization>> =
            api.list(&format!("v1/builds/{}/betaBuildLocalizations", previous.id), &[])?;

        let notes = notes
            .into_iter()
            .filter_map(|note| note.attributes.into_description())
            .collect::<Vec<_>>();

        // a build uploaded without notes must not block every later release
        if notes.is_empty() {
            print_warning(&format!(
                "Previous iOS build {} has no test description, the new build will have none",
                previous.attributes.version
            ));
        } else {
            print_info(&format!(
                "Will copy test descriptions from iOS build {} to {GROUP_NAME}",
                previous.attributes.version
            ));
        }

        Ok(Self { api, app_id: app.id, group, notes })
    }

    /// Wait for this exact build, copy all saved test descriptions, and add it to me-only
    pub(crate) fn distribute(&self, identity: &BuildIdentity) -> Result<()> {
        self.distribute_with_timeout(identity, PROCESSING_TIMEOUT, POLL_INTERVAL)
    }

    fn distribute_with_timeout(
        &self,
        identity: &BuildIdentity,
        timeout: Duration,
        interval: Duration,
    ) -> Result<()> {
        print_info("Waiting for App Store Connect to process the uploaded build...");
        let deadline = Instant::now() + timeout;

        loop {
            let builds: Vec<Resource<Build>> = self.api.list(
                "v1/builds",
                &[
                    ("filter[app]", &self.app_id),
                    ("filter[preReleaseVersion.platform]", "IOS"),
                    ("filter[preReleaseVersion.version]", &identity.version),
                    ("filter[version]", &identity.build_number),
                ],
            )?;

            ensure!(builds.len() <= 1, "Multiple builds match the uploaded archive");

            if let Some(build) = builds.first() {
                ensure!(
                    build.attributes.version == identity.build_number,
                    "App Store Connect returned a different build number"
                );

                match build.attributes.processing_state {
                    ProcessingState::Valid => return self.assign(build),
                    ProcessingState::Processing => {}
                    ProcessingState::Failed | ProcessingState::Invalid => {
                        bail!(
                            "App Store Connect rejected build {}: {:?}",
                            identity.build_number,
                            build.attributes.processing_state
                        );
                    }
                }
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(
                !remaining.is_zero(),
                "Timed out waiting for TestFlight build {} ({})",
                identity.version,
                identity.build_number
            );

            std::thread::sleep(interval.min(remaining));
        }
    }

    fn assign(&self, build: &Resource<Build>) -> Result<()> {
        if build.attributes.uses_non_exempt_encryption != Some(false) {
            let path = format!("v1/builds/{}", build.id);
            let body = Document {
                data: Update {
                    kind: BUILDS,
                    id: &build.id,
                    attributes: EncryptionAttributes { uses_non_exempt_encryption: false },
                },
            };

            self.api.send(Method::PATCH, self.api.url(&path, &[])?, &body)?;
            print_info("Set Uses Non-Exempt Encryption to No");
        }

        let existing: Vec<Resource<Localization>> =
            self.api.list(&format!("v1/builds/{}/betaBuildLocalizations", build.id), &[])?;

        // write descriptions before group membership so testers receive the intended notes
        for note in &self.notes {
            let current = existing.iter().find(|entry| entry.attributes.locale == note.locale);
            self.write_localization(&build.id, note, current)?;
        }

        // the API permits only one relationship filter; build IDs already identify the app
        let groups: Vec<Resource<IgnoredAny>> =
            self.api.list("v1/betaGroups", &[("filter[builds]", &build.id)])?;

        if !groups.iter().any(|group| group.id == self.group.id) {
            let path = format!("v1/betaGroups/{}/relationships/builds", self.group.id);
            let body = Document { data: [Identifier::build(&build.id)] };
            self.api.send(Method::POST, self.api.url(&path, &[])?, &body)?;
        }

        if !self.group.attributes.is_internal_group {
            self.submit_for_external_testing(build)?;
        }

        print_success(&format!(
            "Copied test descriptions and added build {} to {GROUP_NAME}",
            build.attributes.version
        ));
        Ok(())
    }

    fn submit_for_external_testing(&self, build: &Resource<Build>) -> Result<()> {
        let path = format!("v1/builds/{}/buildBetaDetail", build.id);
        let detail: BuildBetaDetailResponse = self
            .api
            .get(self.api.url(&path, &[])?)?
            .json()
            .wrap_err("Invalid TestFlight beta status response")?;

        match detail.data.attributes.external_build_state {
            ExternalBetaState::ReadyForBetaSubmission => {
                let body = Document {
                    data: Create::<()> {
                        kind: "betaAppReviewSubmissions",
                        attributes: None,
                        relationships: BuildRelationship::new(&build.id),
                    },
                };

                self.api.send(
                    Method::POST,
                    self.api.url("v1/betaAppReviewSubmissions", &[])?,
                    &body,
                )?;

                print_info("Submitted the build for TestFlight beta review");
            }

            ExternalBetaState::WaitingForBetaReview | ExternalBetaState::InBetaReview => {
                print_info("The build is waiting for TestFlight beta review");
            }

            ExternalBetaState::BetaApproved
            | ExternalBetaState::ReadyForBetaTesting
            | ExternalBetaState::InBetaTesting => {}
            state => {
                bail!("TestFlight cannot start external testing in state {state:?}");
            }
        }

        Ok(())
    }

    fn write_localization(
        &self,
        build_id: &str,
        note: &TestDescription,
        current: Option<&Resource<Localization>>,
    ) -> Result<()> {
        if let Some(current) = current {
            let path = format!("v1/betaBuildLocalizations/{}", current.id);
            let body = Document {
                data: Update {
                    kind: BETA_BUILD_LOCALIZATIONS,
                    id: &current.id,
                    attributes: WhatsNew { whats_new: &note.whats_new },
                },
            };

            self.api.send(Method::PATCH, self.api.url(&path, &[])?, &body)?;
            return Ok(());
        }

        let body = Document {
            data: Create {
                kind: BETA_BUILD_LOCALIZATIONS,
                attributes: Some(note),
                relationships: BuildRelationship::new(build_id),
            },
        };

        self.api.send(Method::POST, self.api.url("v1/betaBuildLocalizations", &[])?, &body)?;
        Ok(())
    }
}

fn exact_group(groups: Vec<Resource<Group>>) -> Result<Resource<Group>> {
    let mut matches = groups.into_iter().filter(|group| group.attributes.name == GROUP_NAME);
    let group =
        matches.next().ok_or_else(|| eyre!("TestFlight group {GROUP_NAME} was not found"))?;

    ensure!(matches.next().is_none(), "Multiple TestFlight groups are named {GROUP_NAME}");
    Ok(group)
}

struct Api {
    client: Client,
    base_url: Url,
    key: EncodingKey,
    key_id: String,
    issuer_id: String,
}

impl Api {
    fn new(key: &[u8], key_id: &str, issuer_id: &str) -> Result<Self> {
        let client = http_client_without_redirects()?;

        Ok(Self {
            client,
            base_url: Url::parse(API_URL)?,
            key: EncodingKey::from_ec_pem(key).wrap_err("Invalid App Store Connect ES256 key")?,
            key_id: key_id.to_owned(),
            issuer_id: issuer_id.to_owned(),
        })
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url> {
        let mut url = self.base_url.join(path)?;
        ensure!(
            url.origin() == self.base_url.origin()
                && url.username().is_empty()
                && url.password().is_none(),
            "App Store Connect returned an unsafe pagination URL"
        );

        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }

        Ok(url)
    }

    fn list<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<Resource<T>>> {
        let mut url = self.url(path, query)?;
        let mut seen = HashSet::new();
        let mut data = Vec::new();

        loop {
            ensure!(seen.insert(url.to_string()), "App Store Connect repeated a pagination URL");
            let page: Page<T> =
                self.get(url)?.json().wrap_err("Invalid App Store Connect response")?;

            data.extend(page.data);
            let Some(next) = page.links.next else { return Ok(data) };
            url = self.url(&next, &[])?;
        }
    }

    fn get(&self, url: Url) -> Result<Response> {
        self.request::<()>(Method::GET, url, None)
    }

    fn send<B: Serialize>(&self, method: Method, url: Url, body: &B) -> Result<Response> {
        self.request(method, url, Some(body))
    }

    fn request<B: Serialize>(
        &self,
        method: Method,
        url: Url,
        body: Option<&B>,
    ) -> Result<Response> {
        for attempt in 0..3 {
            let issued_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(self.key_id.clone());
            let claims = Claims {
                iss: &self.issuer_id,
                iat: issued_at,
                exp: issued_at + 600,
                aud: "appstoreconnect-v1",
            };
            let token = jsonwebtoken::encode(&header, &claims, &self.key)
                .wrap_err("Failed to sign App Store Connect request")?;

            let mut request = self.client.request(method.clone(), url.clone()).bearer_auth(token);

            if let Some(body) = body {
                request = request.json(body);
            }

            let response = request.send().wrap_err("App Store Connect request failed")?;
            let status = response.status();

            if status.is_success() {
                return Ok(response);
            }

            if method == Method::GET
                && attempt < 2
                && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
            {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }

            let errors = response.text().unwrap_or_default();
            bail!("App Store Connect {method} {} failed ({status}): {errors}", url.path());
        }

        unreachable!("the last request attempt always returns")
    }
}

const BUILDS: &str = "builds";
const BETA_BUILD_LOCALIZATIONS: &str = "betaBuildLocalizations";

/// App Store Connect API token claims
#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    iat: u64,
    exp: u64,
    aud: &'static str,
}

/// JSON:API request document
#[derive(Serialize)]
struct Document<T> {
    data: T,
}

/// JSON:API resource identifier
#[derive(Serialize)]
struct Identifier<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
}

impl<'a> Identifier<'a> {
    fn build(id: &'a str) -> Self {
        Self { kind: BUILDS, id }
    }
}

/// JSON:API update of an existing resource's attributes
#[derive(Serialize)]
struct Update<'a, A> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
    attributes: A,
}

/// JSON:API create of a resource related to a build
#[derive(Serialize)]
struct Create<'a, A> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<A>,
    relationships: BuildRelationship<'a>,
}

#[derive(Serialize)]
struct BuildRelationship<'a> {
    build: Document<Identifier<'a>>,
}

impl<'a> BuildRelationship<'a> {
    fn new(build_id: &'a str) -> Self {
        Self { build: Document { data: Identifier::build(build_id) } }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptionAttributes {
    uses_non_exempt_encryption: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WhatsNew<'a> {
    whats_new: &'a str,
}

#[derive(Deserialize)]
struct Page<T> {
    data: Vec<Resource<T>>,
    #[serde(default)]
    links: Links,
}

#[derive(Default, Deserialize)]
struct Links {
    next: Option<String>,
}

#[derive(Deserialize)]
struct Resource<T> {
    id: String,
    attributes: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct App {
    bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    name: String,
    is_internal_group: bool,
}

#[derive(Deserialize)]
struct BuildBetaDetailResponse {
    data: Resource<BuildBetaDetail>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildBetaDetail {
    external_build_state: ExternalBetaState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ExternalBetaState {
    Processing,
    ProcessingException,
    MissingExportCompliance,
    ReadyForBetaTesting,
    InBetaTesting,
    Expired,
    ReadyForBetaSubmission,
    InExportComplianceReview,
    WaitingForBetaReview,
    InBetaReview,
    BetaRejected,
    BetaApproved,
    NotApplicable,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Build {
    version: String,
    processing_state: ProcessingState,
    uses_non_exempt_encryption: Option<bool>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProcessingState {
    Processing,
    Valid,
    Failed,
    Invalid,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Localization {
    locale: String,
    whats_new: Option<String>,
}

impl Localization {
    fn into_description(self) -> Option<TestDescription> {
        let whats_new = self.whats_new.filter(|text| !text.trim().is_empty())?;
        Some(TestDescription { locale: self.locale, whats_new })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestDescription {
    locale: String,
    whats_new: String,
}

#[cfg(test)]
mod tests;
