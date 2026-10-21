//! Confirmation times shown in the device's time zone

use cove_device::device::Device;
use cove_types::utxo::Utxo;
use jiff::{Timestamp, Zoned};

/// Date format shared by the transaction list and transaction details
const DATE_FORMAT: &str = "%B %-d, %Y";

/// Converts a timestamp to the device's time zone, or UTC when the zone is unknown
pub(crate) fn in_device_time_zone(timestamp: Timestamp) -> Zoned {
    let time_zone = Device::global().timezone();

    match timestamp.in_tz(&time_zone) {
        Ok(local) => local,
        Err(error) => {
            tracing::warn!("unable to convert timestamp to {time_zone}: {error}");
            timestamp.to_zoned(jiff::tz::TimeZone::UTC)
        }
    }
}

/// The calendar date of `timestamp` where the user is, such as "January 20, 2026"
pub(crate) fn local_date(timestamp: Timestamp) -> String {
    in_device_time_zone(timestamp).strftime(DATE_FORMAT).to_string()
}

/// The date and time of `timestamp` where the user is, such as "January 20, 2026 at 7:57 PM"
pub(crate) fn local_date_time(timestamp: Timestamp) -> String {
    let date_time_format = format!("{DATE_FORMAT} at %-I:%M %p");
    in_device_time_zone(timestamp).strftime(&date_time_format).to_string()
}

/// Short date format for coin lists, such as "Jan 5, 2026"
const SHORT_DATE_FORMAT: &str = "%b %-d, %Y";

/// The calendar date a UTXO was confirmed where the user is, such as "Jan 20, 2026"
#[uniffi::export]
pub fn utxo_date(utxo: Utxo) -> String {
    let Ok(timestamp) = Timestamp::from_second(utxo.datetime.cast_signed()) else {
        return String::new();
    };

    in_device_time_zone(timestamp).strftime(SHORT_DATE_FORMAT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_types::utxo::ffi_preview::preview_new_utxo_list;

    // regression for Manage UTXOs dates: 05:30 UTC on Jan 21 is still Jan 20 in America/Chicago
    #[test]
    fn utxo_date_uses_the_device_time_zone() {
        crate::test_support::init_test_device();

        let mut utxo = preview_new_utxo_list(1, 0).remove(0);
        utxo.datetime = 1_768_973_400;

        assert_eq!(utxo_date(utxo), "Jan 20, 2026");
    }
}
