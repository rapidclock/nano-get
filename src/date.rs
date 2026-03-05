use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::errors::NanoGetError;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) fn format_http_date(time: SystemTime) -> Result<String, NanoGetError> {
    let seconds = time.duration_since(UNIX_EPOCH).map_err(|_| {
        NanoGetError::InvalidHeaderValue("HTTP dates before the Unix epoch are unsupported".into())
    })?;
    let seconds = seconds.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];

    Ok(format!(
        "{weekday}, {day:02} {} {year:04} {hour:02}:{minute:02}:{second:02} GMT",
        MONTHS[month as usize - 1]
    ))
}

pub(crate) fn parse_http_date(value: &str) -> Option<SystemTime> {
    parse_imf_fixdate(value)
        .or_else(|| parse_rfc850_date(value))
        .or_else(|| parse_asctime_date(value))
}

fn parse_imf_fixdate(value: &str) -> Option<SystemTime> {
    let (weekday, remainder) = value.split_once(", ")?;
    if !WEEKDAYS.contains(&weekday) {
        return None;
    }

    let mut pieces = remainder.split_whitespace();
    let day = parse_u32(pieces.next()?)?;
    let month = parse_month(pieces.next()?)?;
    let year = parse_i32(pieces.next()?)?;
    let (hour, minute, second) = parse_time_of_day(pieces.next()?)?;

    if pieces.next()? != "GMT" || pieces.next().is_some() {
        return None;
    }

    build_system_time(year, month, day, hour, minute, second)
}

fn parse_rfc850_date(value: &str) -> Option<SystemTime> {
    let (_weekday, remainder) = value.split_once(", ")?;
    let mut pieces = remainder.split_whitespace();
    let date = pieces.next()?;
    let time = pieces.next()?;
    if pieces.next()? != "GMT" || pieces.next().is_some() {
        return None;
    }

    let mut date_pieces = date.split('-');
    let day = parse_u32(date_pieces.next()?)?;
    let month = parse_month(date_pieces.next()?)?;
    let year = parse_rfc850_year(date_pieces.next()?)?;
    if date_pieces.next().is_some() {
        return None;
    }

    let (hour, minute, second) = parse_time_of_day(time)?;
    build_system_time(year, month, day, hour, minute, second)
}

fn parse_asctime_date(value: &str) -> Option<SystemTime> {
    let mut pieces = value.split_whitespace();
    let weekday = pieces.next()?;
    if !WEEKDAYS.contains(&weekday) {
        return None;
    }

    let month = parse_month(pieces.next()?)?;
    let day = parse_u32(pieces.next()?)?;
    let (hour, minute, second) = parse_time_of_day(pieces.next()?)?;
    let year = parse_i32(pieces.next()?)?;
    if pieces.next().is_some() {
        return None;
    }

    build_system_time(year, month, day, hour, minute, second)
}

fn parse_rfc850_year(value: &str) -> Option<i32> {
    let year = parse_i32(value)?;
    Some(if year >= 70 { 1900 + year } else { 2000 + year })
}

fn parse_month(value: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(value))
        .map(|index| index as u32 + 1)
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse().ok()
}

fn parse_i32(value: &str) -> Option<i32> {
    value.parse().ok()
}

fn parse_time_of_day(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split(':');
    let hour = parse_u32(parts.next()?)?;
    let minute = parse_u32(parts.next()?)?;
    let second = parse_u32(parts.next()?)?;
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

fn build_system_time(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<SystemTime> {
    if !(1..=12).contains(&month) || day == 0 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let month_length = days_in_month(year, month);
    if day > month_length {
        return None;
    }

    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }

    let seconds = days as u64 * 86_400 + hour as u64 * 3_600 + minute as u64 * 60 + second as u64;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{format_http_date, parse_http_date};

    #[test]
    fn formats_http_dates() {
        let formatted = format_http_date(UNIX_EPOCH + Duration::from_secs(784_111_777)).unwrap();
        assert_eq!(formatted, "Sun, 06 Nov 1994 08:49:37 GMT");
    }

    #[test]
    fn parses_imf_fixdates() {
        let parsed = parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        assert_eq!(
            format_http_date(parsed).unwrap(),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
    }

    #[test]
    fn parses_rfc850_dates() {
        let parsed = parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").unwrap();
        assert_eq!(
            format_http_date(parsed).unwrap(),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
    }

    #[test]
    fn parses_asctime_dates() {
        let parsed = parse_http_date("Sun Nov  6 08:49:37 1994").unwrap();
        assert_eq!(
            format_http_date(parsed).unwrap(),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
    }

    #[test]
    fn rejects_invalid_dates() {
        assert!(parse_http_date("bogus").is_none());
    }
}
