//! Confirmation times shown in the device's time zone

use cove_device::device::Device;
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
