/// Gets the decimal point and the decimal places after it limited by the max_decimal_places
pub fn limit_decimal_places(amount: &str, max_decimal_places: usize) -> Option<&str> {
    let (dollars, cents_with_decimal_point) =
        seperate_and_limit_dollars_and_cents(amount, max_decimal_places);

    // if the dollars and cents are the same length, then there was no change
    if amount.len() == dollars.len() + cents_with_decimal_point.len() {
        return None;
    }

    Some(cents_with_decimal_point)
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

    const MAX_DECIMAL_PLACES: usize = 2;

    #[test]
    fn test_limit_decimal_places() {
        let amount = "0.00";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "0.01";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "0.1";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "0.12";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "0.123";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, Some(".12"));

        let amount = "0.1234";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, Some(".12"));

        let amount = "12.34";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "123.4";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "1234.0";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "1234.00";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, None);

        let amount = "1234.000";
        let result = limit_decimal_places(amount, MAX_DECIMAL_PLACES);
        assert_eq!(result, Some(".00"));
    }
}
