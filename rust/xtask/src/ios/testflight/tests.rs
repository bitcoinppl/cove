use super::{exact_group, Api, BuildIdentity, Distribution, Group, Localization, Resource};
use reqwest::Url;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// disposable test key, unrelated to App Store Connect credentials
const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgiEMKw30PfmMEwjn3
j0ygowH8+9FglyHR6PVLIHVq41ahRANCAAQ7vkWoz3MlWICekj51h5HziO7hSgeW
sje31mu8zpQ44ib5q2hHTcTG/T8A4GKUvCILYf92wwyPkMtQn+GYSHhZ
-----END PRIVATE KEY-----";

struct Request {
    method: String,
    url: Url,
    body: Value,
}

struct Server {
    url: Url,
    handle: JoinHandle<Vec<Request>>,
}

impl Server {
    fn new(responses: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();

            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Mock API request did not arrive");
                            std::thread::sleep(Duration::from_millis(5));
                        }

                        Err(error) => panic!("{error}"),
                    }
                };

                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 1024];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);

                    if let Some(index) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);

                while bytes.len() < header_end + length {
                    let mut buffer = [0; 1024];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }

                assert!(headers.to_lowercase().contains("authorization: bearer "));
                let mut line = headers.lines().next().unwrap().split_whitespace();
                let method = line.next().unwrap().to_owned();
                let url = Url::parse(&format!("http://localhost{}", line.next().unwrap())).unwrap();
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                };
                requests.push(Request { method, url, body });

                let body = response.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }

            requests
        });

        Self { url, handle }
    }

    fn api(&self) -> Api {
        let mut api = Api::new(TEST_KEY.as_bytes(), "TESTKEY", "test-issuer").unwrap();
        api.base_url = self.url.clone();
        api.client = reqwest::blocking::Client::builder().no_proxy().build().unwrap();
        api
    }
}

fn page(data: Value) -> Value {
    json!({"data": data, "links": {"next": null}})
}

fn build(state: &str) -> Value {
    page(json!([{"id": "new-build", "attributes": {
        "version": "203", "processingState": state, "usesNonExemptEncryption": false
    }}]))
}

fn identity() -> BuildIdentity {
    BuildIdentity { version: "1.4.0".to_owned(), build_number: "203".to_owned() }
}

fn target_group(id: &str, internal: bool) -> Resource<Group> {
    Resource {
        id: id.to_owned(),
        attributes: Group { name: "me-only".to_owned(), is_internal_group: internal },
    }
}

fn beta_detail(state: &str) -> Value {
    json!({"data": {"id": "detail", "attributes": {"externalBuildState": state}}})
}

#[test]
fn copies_snapshot_for_each_locale_before_adding_exact_build_to_group() {
    let description = "Fix backup\nKeep this exact text 🪙";
    let server = Server::new(vec![
        page(json!([{"id": "app", "attributes": {"bundleId": "org.bitcoinppl.cove"}}])),
        page(
            json!([{"id": "private-group", "attributes": {"name": "me-only", "isInternalGroup": false}}]),
        ),
        page(
            json!([{"id": "old-build", "attributes": {"version": "202", "processingState": "VALID"}}]),
        ),
        page(json!([
            {"id": "old-en", "attributes": {"locale": "en-US", "whatsNew": description}},
            {"id": "old-fr", "attributes": {"locale": "fr-FR", "whatsNew": "Correction"}},
            {"id": "old-de", "attributes": {"locale": "de-DE", "whatsNew": null}},
            {"id": "old-es", "attributes": {"locale": "es-ES", "whatsNew": ""}}
        ])),
        build("PROCESSING"),
        build("VALID"),
        page(json!([{"id": "new-en", "attributes": {"locale": "en-US", "whatsNew": "stale"}}])),
        json!({}),
        json!({}),
        page(json!([])),
        json!({}),
        beta_detail("READY_FOR_BETA_SUBMISSION"),
        json!({}),
    ]);
    let distribution = Distribution::prepare_with_api(server.api(), "org.bitcoinppl.cove").unwrap();
    distribution
        .distribute_with_timeout(&identity(), Duration::from_secs(2), Duration::ZERO)
        .unwrap();
    let requests = server.handle.join().unwrap();
    let previous_query = requests[2].url.query_pairs().collect::<std::collections::HashMap<_, _>>();
    assert_eq!(previous_query["filter[preReleaseVersion.platform]"], "IOS");
    assert_eq!(previous_query["sort"], "-uploadedDate");
    assert_eq!(previous_query["filter[processingState]"], "VALID");
    assert_eq!(requests[3].url.path(), "/v1/builds/old-build/betaBuildLocalizations");

    for request in &requests[4..6] {
        let query = request.url.query_pairs().collect::<std::collections::HashMap<_, _>>();
        assert_eq!(query["filter[app]"], "app");
        assert_eq!(query["filter[preReleaseVersion.platform]"], "IOS");
        assert_eq!(query["filter[preReleaseVersion.version]"], "1.4.0");
        assert_eq!(query["filter[version]"], "203");
    }

    assert_eq!(requests[7].method, "PATCH");
    assert_eq!(requests[7].url.path(), "/v1/betaBuildLocalizations/new-en");
    assert_eq!(requests[7].body["data"]["attributes"]["whatsNew"], description);
    assert_eq!(requests[8].method, "POST");
    assert_eq!(requests[8].body["data"]["attributes"]["locale"], "fr-FR");
    assert_eq!(requests[8].body["data"]["relationships"]["build"]["data"]["id"], "new-build");
    assert_eq!(requests[9].url.path(), "/v1/betaGroups");
    let group_query = requests[9].url.query_pairs().collect::<std::collections::HashMap<_, _>>();
    assert_eq!(group_query.len(), 1);
    assert_eq!(group_query["filter[builds]"], "new-build");
    assert_eq!(requests[10].url.path(), "/v1/betaGroups/private-group/relationships/builds");
    assert_eq!(requests[10].body, json!({"data": [{"type": "builds", "id": "new-build"}]}));
    assert_eq!(requests[11].url.path(), "/v1/builds/new-build/buildBetaDetail");
    assert_eq!(requests[12].method, "POST");
    assert_eq!(requests[12].url.path(), "/v1/betaAppReviewSubmissions");
    assert_eq!(requests[12].body["data"]["relationships"]["build"]["data"]["id"], "new-build");
}

#[test]
fn external_review_is_not_submitted_again() {
    for state in ["WAITING_FOR_BETA_REVIEW", "IN_BETA_REVIEW", "BETA_APPROVED", "IN_BETA_TESTING"] {
        let server = Server::new(vec![
            build("VALID"),
            page(json!([])),
            page(json!([
                {"id": "group", "attributes": {}}
            ])),
            beta_detail(state),
        ]);
        let distribution = Distribution {
            api: server.api(),
            app_id: "app".to_owned(),
            group: target_group("group", false),
            notes: vec![],
        };

        distribution.distribute_with_timeout(&identity(), Duration::ZERO, Duration::ZERO).unwrap();
        assert!(server.handle.join().unwrap().iter().all(|request| request.method == "GET"));
    }
}

#[test]
fn rejects_failed_invalid_unknown_and_timed_out_builds_without_writes() {
    for state in ["FAILED", "INVALID", "NEW_UNKNOWN_STATE", "PROCESSING"] {
        let server = Server::new(vec![build(state)]);
        let distribution = Distribution {
            api: server.api(),
            app_id: "app".to_owned(),
            group: target_group("group", true),
            notes: vec![],
        };
        assert!(distribution
            .distribute_with_timeout(&identity(), Duration::ZERO, Duration::ZERO)
            .is_err());
        let requests = server.handle.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }
}

#[test]
fn existing_membership_is_not_added_again() {
    let server = Server::new(vec![
        build("VALID"),
        page(json!([])),
        page(json!([
            {"id": "group", "attributes": {}}
        ])),
    ]);
    let distribution = Distribution {
        api: server.api(),
        app_id: "app".to_owned(),
        group: target_group("group", true),
        notes: vec![],
    };
    distribution.distribute_with_timeout(&identity(), Duration::ZERO, Duration::ZERO).unwrap();
    assert!(server.handle.join().unwrap().iter().all(|request| request.method == "GET"));
}

#[test]
fn sets_missing_compliance_before_group_assignment() {
    let mut pending_compliance = build("VALID");
    pending_compliance["data"][0]["attributes"]["usesNonExemptEncryption"] = Value::Null;
    let server = Server::new(vec![
        pending_compliance,
        json!({}),
        page(json!([])),
        page(json!([])),
        json!({}),
    ]);
    let distribution = Distribution {
        api: server.api(),
        app_id: "app".to_owned(),
        group: target_group("group", true),
        notes: vec![],
    };

    distribution.distribute_with_timeout(&identity(), Duration::ZERO, Duration::ZERO).unwrap();
    let requests = server.handle.join().unwrap();
    assert_eq!(requests[1].method, "PATCH");
    assert_eq!(requests[1].url.path(), "/v1/builds/new-build");
    assert_eq!(
        requests[1].body,
        json!({"data": {
            "type": "builds", "id": "new-build", "attributes": {"usesNonExemptEncryption": false}
        }})
    );
    assert_eq!(requests[4].url.path(), "/v1/betaGroups/group/relationships/builds");
}

#[test]
fn requires_one_exact_group() {
    let group = |name: &str| Resource {
        id: "id".to_owned(),
        attributes: Group { name: name.to_owned(), is_internal_group: true },
    };
    assert!(exact_group(vec![]).is_err());
    assert!(exact_group(vec![group("other")]).is_err());
    assert!(exact_group(vec![group("me-only"), group("me-only")]).is_err());
    assert_eq!(exact_group(vec![group("me-only")]).unwrap().id, "id");
}

#[test]
fn pagination_cannot_send_credentials_to_another_origin() {
    let api = Api::new(TEST_KEY.as_bytes(), "TESTKEY", "issuer").unwrap();
    assert!(api.url("https://example.com/v1/apps", &[]).is_err());
    assert!(api.url("http://api.appstoreconnect.apple.com/v1/apps", &[]).is_err());
    assert!(api.url("https://user@api.appstoreconnect.apple.com/v1/apps", &[]).is_err());
    assert!(api.url("https://api.appstoreconnect.apple.com/v1/apps?cursor=next", &[]).is_ok());
}

#[test]
fn empty_localizations_do_not_create_invalid_description_requests() {
    for whats_new in [None, Some(String::new()), Some(" \n".to_owned())] {
        let note = Localization { locale: "en-US".to_owned(), whats_new };
        assert!(note.into_description().is_none());
    }

    let note =
        Localization { locale: "en-US".to_owned(), whats_new: Some(" Exact text\n".to_owned()) };
    assert_eq!(note.into_description().unwrap().whats_new, " Exact text\n");
}
