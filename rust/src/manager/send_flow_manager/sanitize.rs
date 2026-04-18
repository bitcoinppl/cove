/// Known currency tokens that can appear as a prefix or suffix when pasting
/// formatted amounts (e.g. "100 CHF", "BTC 0.5", "100 SATS").
const CURRENCY_TOKENS: &[&str] =
    &["SATS", "SAT", "BTC", "CHF", "USD", "EUR", "GBP", "CAD", "AUD", "JPY"];

/// Strip leading/trailing currency tokens and whitespace from `s` (case-insensitive).
///
/// Returns `Some(cleaned)` if no alphabetic characters remain after stripping,
/// or `None` if letters survive (e.g. a bech32 address or free-form text).
pub fn strip_currency_suffix(s: &str) -> Option<String> {
    let mut work = s.trim();

    let mut changed = true;
    while changed {
        changed = false;
        for token in CURRENCY_TOKENS {
            let upper_work = work.to_ascii_uppercase();
            if let Some(rest) = upper_work.strip_prefix(token) {
                // advance past the token in the original (same byte length since ASCII)
                work = work[token.len()..].trim();
                let _ = rest;
                changed = true;
                continue;
            }
            if let Some(rest) = upper_work.strip_suffix(token) {
                work = work[..rest.len()].trim();
                changed = true;
            }
        }
    }

    if work.chars().any(|c| c.is_alphabetic()) {
        return None;
    }

    Some(work.to_string())
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
}
