use crate::fiat::FiatCurrency;
use strum::IntoEnumIterator as _;

fn fiat_tokens() -> Vec<&'static str> {
    let mut tokens: Vec<&'static str> = FiatCurrency::all_symbols().to_vec();
    for currency in FiatCurrency::iter() {
        tokens.push(currency.into());
    }
    tokens
}

const BTC_TOKENS: &[&str] = &["SATS", "SAT", "BTC"];

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

/// Sanitizes a raw amount string for the fiat field.
///
/// Strips recognized fiat tokens (symbols and codes). Returns `None` if
/// BTC/SAT/SATS or other unrecognized alphabetic characters are present —
/// those should not be accepted into a fiat amount field.
pub fn sanitize_fiat_amount(input: &str) -> Option<String> {
    if !input.chars().any(|c| c.is_alphabetic()) {
        return Some(input.to_string());
    }
    strip_tokens(input, &fiat_tokens())
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

// returns the dollars and the cents (with the decimal point) as a string
pub fn seperate_and_limit_dollars_and_cents(
    amount: &str,
    max_decimal_places: usize,
) -> (&str, &str) {
    let last_index = amount.len().saturating_sub(1);

    let decimal_index = match memchr::memchr(b'.', amount.as_bytes()) {
        Some(decimal_index) => decimal_index,
        None => return (amount, ""),
    };

    let current_decimal_places = last_index - decimal_index;
    let decimal_places = current_decimal_places.min(max_decimal_places);

    let dollars = &amount[..decimal_index];
    let cents_with_decimal_point = &amount[decimal_index..=decimal_index + decimal_places];
    (dollars, cents_with_decimal_point)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn sanitize_fiat_rejects_address() {
        assert!(sanitize_fiat_amount("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").is_none());
        assert!(sanitize_fiat_amount("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_none());
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

    #[test]
    fn sanitize_btc_rejects_address() {
        assert!(sanitize_btc_amount("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").is_none());
    }

    #[test]
    fn sanitize_btc_sats_prefix_not_mis_stripped_as_sat() {
        // "SATS" must be matched before "SAT" to avoid leaving a stray "S"
        assert_eq!(sanitize_btc_amount("SATS 0.1").unwrap(), "0.1");
        assert_eq!(sanitize_btc_amount("sats 1000").unwrap(), "1000");
    }
}
