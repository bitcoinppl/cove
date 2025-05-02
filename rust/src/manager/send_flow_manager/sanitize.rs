/// Gets the decimal point and the decimal places after it limited by the max_decimal_places
pub fn limit_decimal_places(amount: &str, max_decimal_places: usize) -> Option<&str> {
    // get how many decimals there are after the decimal point
    let last_index = amount.len().saturating_sub(1);

    let decimal_index = memchr::memchr(b'.', amount.as_bytes())?;
    let current_decimal_places = last_index - decimal_index;

    // if there are more than the max decimals, then no change is needed
    if current_decimal_places <= max_decimal_places {
        return None;
    }

    // get the number of decimals after the decimal point
    let decimal_places = current_decimal_places.min(max_decimal_places);

    let decimal_places_with_decimal_point = &amount[decimal_index..=decimal_index + decimal_places];
    Some(decimal_places_with_decimal_point)
}
