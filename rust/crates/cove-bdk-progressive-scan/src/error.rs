use bdk_esplora::esplora_client;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("full scan request is missing")]
    MissingRequest,

    #[error("stop gap is missing")]
    MissingStopGap,

    #[error("event sender is missing")]
    MissingEvents,

    #[error("scan was cancelled")]
    Cancelled,

    #[error("scan event receiver closed")]
    ChannelClosed,

    /// Built through `From`, which drops the request URL
    #[error("esplora scan failed: {}", esplora_error_text(.0))]
    Esplora(#[source] Box<esplora_client::Error>),

    #[error("electrum scan failed: {0}")]
    Electrum(#[from] bdk_electrum::electrum_client::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<Box<esplora_client::Error>> for Error {
    fn from(error: Box<esplora_client::Error>) -> Self {
        Self::Esplora(Box::new(esplora_error_without_url(*error)))
    }
}

/// Drops the request URL from an esplora error
///
/// A node URL can carry an access token in its path or query, and scan errors reach logs
pub fn esplora_error_without_url(error: esplora_client::Error) -> esplora_client::Error {
    match error {
        esplora_client::Error::Reqwest(error) => {
            esplora_client::Error::Reqwest(error.without_url())
        }
        error => error,
    }
}

/// The longest part of an HTTP error body an error message quotes
const HTTP_BODY_EXCERPT_CHARS: usize = 120;

/// The first line of an HTTP error body, cut short, or nothing for an empty or markup body
///
/// A proxy can answer with a whole HTML error page, which would fill an alert with markup
fn http_body_excerpt(body: &str) -> Option<String> {
    // a byte order mark would hide the `<` an HTML page starts with
    let body = body.trim_start_matches('\u{feff}');
    let line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
    if line.starts_with('<') {
        return None;
    }

    if line.chars().count() <= HTTP_BODY_EXCERPT_CHARS {
        return Some(line.to_string());
    }

    let excerpt: String = line.chars().take(HTTP_BODY_EXCERPT_CHARS).collect();
    Some(format!("{excerpt}…"))
}

/// Describes an esplora error in words
///
/// esplora-client's own Display prints its Debug form, such as `Reqwest(reqwest::Error { .. })`,
/// and node errors reach the apps' alerts. A request error lists its causes, as in "error
/// sending request: client error (Connect): tcp connect error: Connection refused", and leaves
/// out header values, which can hold credentials
pub fn esplora_error_text(error: &esplora_client::Error) -> String {
    use esplora_client::Error;

    match error {
        Error::Reqwest(error) => {
            let mut text = error.to_string();
            let mut source = std::error::Error::source(error);
            while let Some(cause) = source {
                text.push_str(&format!(": {cause}"));
                source = cause.source();
            }

            text
        }

        Error::HttpResponse { status, message } => match http_body_excerpt(message) {
            Some(excerpt) => format!("the node answered with HTTP status {status}: {excerpt}"),
            None => format!("the node answered with HTTP status {status}"),
        },
        Error::Parsing(error) => format!("the node sent an invalid number: {error}"),
        Error::StatusCode(error) => format!("the node sent an invalid HTTP status: {error}"),
        Error::BitcoinEncoding(error) => format!("the node sent invalid Bitcoin data: {error}"),
        Error::HexToArray(error) => format!("the node sent invalid hex: {error}"),
        Error::HexToBytes(error) => format!("the node sent invalid hex: {error}"),
        Error::TransactionNotFound(txid) => format!("transaction {txid} not found"),
        Error::HeaderHeightNotFound(height) => format!("no block header at height {height}"),
        Error::HeaderHashNotFound(hash) => format!("block header {hash} not found"),
        Error::InvalidHttpHeaderName(name) => format!("invalid HTTP header name: {name}"),
        Error::InvalidHttpHeaderValue(_) => "invalid HTTP header value".to_string(),
        Error::InvalidResponse => "the node sent an invalid response".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use bdk_esplora::esplora_client;

    use super::esplora_error_text;

    fn http_error(status: u16, message: &str) -> String {
        esplora_error_text(&esplora_client::Error::HttpResponse {
            status,
            message: message.to_string(),
        })
    }

    #[test]
    fn http_errors_quote_one_short_line_of_the_body() {
        assert_eq!(
            http_error(400, "\n  invalid address\nmore detail\n"),
            "the node answered with HTTP status 400: invalid address"
        );
        assert_eq!(http_error(502, ""), "the node answered with HTTP status 502");
        assert_eq!(
            http_error(502, "<!DOCTYPE html>\n<html><body>Bad Gateway</body></html>"),
            "the node answered with HTTP status 502"
        );
        assert_eq!(
            http_error(502, "\u{feff}<!DOCTYPE html>\n<html></html>"),
            "the node answered with HTTP status 502"
        );

        let long = http_error(500, &"x".repeat(500));
        assert_eq!(long, format!("the node answered with HTTP status 500: {}…", "x".repeat(120)));
    }
}
