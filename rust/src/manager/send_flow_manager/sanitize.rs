const FIAT_TOKENS: &[&str] = &[
    // fiat symbols
    "$", "€", "£", "¥", "₹", // fiat codes
    "USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF", "INR", "MXN",
];

const BTC_TOKENS: &[&str] = &["BTC", "SAT", "SATS"];

const CURRENCY_TOKENS: &[&str] = &[
    // fiat symbols
    "$", "€", "£", "¥", "₹", // fiat codes
    "USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF", "INR", "MXN", // bitcoin units
    "BTC", "SAT", "SATS",
];

fn strip_prefix_ignore_ascii_case<'a>(input: &'a str, token: &str) -> Option<&'a str> {
    input
        .get(..token.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(token))
        .map(|_| input[token.len()..].trim())
}

fn strip_suffix_ignore_ascii_case<'a>(input: &'a str, token: &str) -> Option<&'a str> {
    let start = input.len().checked_sub(token.len())?;
    input
        .get(start..)
        .filter(|suffix| suffix.eq_ignore_ascii_case(token))
        .map(|_| input[..start].trim())
}

fn strip_tokens(input: &str, tokens: &[&str]) -> Option<String> {
    let mut work = input.trim();

    loop {
        let next = tokens.iter().find_map(|token| {
            strip_prefix_ignore_ascii_case(work, token)
                .or_else(|| strip_suffix_ignore_ascii_case(work, token))
        });

        match next {
            Some(stripped) => work = stripped,
            None => break,
        }
    }

    if work.chars().any(|c| c.is_alphabetic()) {
        return None;
    }

    Some(work.to_string())
}

/// Strip leading/trailing currency tokens from `input` (case-insensitive).
///
/// Returns `Some(cleaned)` if no alphabetic characters remain after stripping,
/// or `None` if letters survive (e.g. a bech32 address or free-form text).
pub fn strip_currency_suffix(input: &str) -> Option<String> {
    strip_tokens(input, CURRENCY_TOKENS)
}

/// Sanitizes a raw amount string for the fiat field.
///
/// Strips recognized fiat tokens (symbols and codes). Returns `None` if
/// BTC/SAT/SATS or other unrecognized alphabetic characters are present —
/// those should not be accepted into a fiat amount field.
pub fn sanitize_fiat_amount(input: &str) -> Option<String> {
    if !input.chars().any(|c| c.is_alphabetic()) {
        return Some(input.to_string());
    }
    strip_tokens(input, FIAT_TOKENS)
}

/// Sanitizes a raw amount string for the BTC/sats field.
///
/// Strips recognized BTC tokens (BTC, SAT, SATS). Returns `None` if fiat
/// symbols/codes or other unrecognized alphabetic characters are present —
/// those should not be accepted into a BTC amount field.
pub fn sanitize_btc_amount(input: &str) -> Option<String> {
    if !input.chars().any(|c| c.is_alphabetic()) {
        return Some(input.to_string());
    }
    strip_tokens(input, BTC_TOKENS)
}

/// Sanitizes a raw amount string by stripping recognized currency tokens.
///
/// Returns `Some(sanitized)` when the input is usable — either no alphabetic
/// characters were present (pass-through), or all alphabetic characters were
/// recognized currency tokens and successfully stripped.
///
/// Returns `None` when the input contains alphabetic characters that are not a
/// recognized currency token. The caller should reject the input and revert to
/// the previous value.
pub fn sanitize_amount(input: &str) -> Option<String> {
    if !input.chars().any(|c| c.is_alphabetic()) {
        return Some(input.to_string());
    }
    strip_currency_suffix(input)
}

// returns the dollars and the cents (with the decimal point) as a string
pub fn seperate_and_limit_dollars_and_cents(
    amount: &str,
    max_decimal_places: usize,
) -> (&str, &str) {
    // get how many decimals there are after the decimal point
    let last_index = amount.len().saturating_sub(1);

    let decimal_index = match memchr::memchr(b'.', amount.as_bytes()) {
        Some(decimal_index) => decimal_index,
        None => return (amount, ""),
    };

    let current_decimal_places = last_index - decimal_index;

    // get the number of decimals after the decimal point
    let decimal_places = current_decimal_places.min(max_decimal_places);

    let dollars = &amount[..decimal_index];
    let cents_with_decimal_point = &amount[decimal_index..=decimal_index + decimal_places];
    (dollars, cents_with_decimal_point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_chf() {
        assert_eq!(strip_currency_suffix("100 CHF").unwrap(), "100");
    }

    #[test]
    fn strips_leading_btc() {
        assert_eq!(strip_currency_suffix("BTC 0.5").unwrap(), "0.5");
    }

    #[test]
    fn strips_sats_suffix_case_insensitive() {
        assert_eq!(strip_currency_suffix("1000 sats").unwrap(), "1000");
    }

    #[test]
    fn strips_dollar_prefix() {
        assert_eq!(strip_currency_suffix("$12.50").unwrap(), "12.50");
    }

    #[test]
    fn plain_number_passes_through() {
        assert_eq!(strip_currency_suffix("123.45").unwrap(), "123.45");
    }

    #[test]
    fn bech32_address_returns_none() {
        assert!(strip_currency_suffix("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").is_none());
    }

    #[test]
    fn legacy_address_returns_none() {
        assert!(strip_currency_suffix("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_none());
    }

    #[test]
    fn sanitize_amount_plain_number_passes_through() {
        assert_eq!(sanitize_amount("100").unwrap(), "100");
    }

    #[test]
    fn sanitize_amount_strips_currency() {
        assert_eq!(sanitize_amount("100 sats").unwrap(), "100");
    }

    #[test]
    fn sanitize_amount_rejects_address() {
        assert!(sanitize_amount("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").is_none());
    }

    #[test]
    fn sanitize_fiat_strips_fiat_tokens() {
        assert_eq!(sanitize_fiat_amount("100 CHF").unwrap(), "100");
        assert_eq!(sanitize_fiat_amount("EUR 50").unwrap(), "50");
        // $ is not alphabetic — no stripping needed, passes through
        assert_eq!(sanitize_fiat_amount("$12.50").unwrap(), "$12.50");
    }

    #[test]
    fn sanitize_fiat_rejects_btc_tokens() {
        assert!(sanitize_fiat_amount("0.5 BTC").is_none());
        assert!(sanitize_fiat_amount("1000 SATS").is_none());
        assert!(sanitize_fiat_amount("500 sat").is_none());
    }

    #[test]
    fn sanitize_fiat_plain_number_passes_through() {
        assert_eq!(sanitize_fiat_amount("100").unwrap(), "100");
    }

    #[test]
    fn sanitize_btc_strips_btc_tokens() {
        assert_eq!(sanitize_btc_amount("0.5 BTC").unwrap(), "0.5");
        assert_eq!(sanitize_btc_amount("1000 SATS").unwrap(), "1000");
        assert_eq!(sanitize_btc_amount("BTC 0.1").unwrap(), "0.1");
    }

    #[test]
    fn sanitize_btc_rejects_fiat_tokens() {
        assert!(sanitize_btc_amount("100 USD").is_none());
        assert!(sanitize_btc_amount("100 CHF").is_none());
        // $ is not alphabetic — passes through; downstream parser rejects it
        assert_eq!(sanitize_btc_amount("$100").unwrap(), "$100");
    }

    #[test]
    fn sanitize_btc_plain_number_passes_through() {
        assert_eq!(sanitize_btc_amount("1000").unwrap(), "1000");
    }
}
