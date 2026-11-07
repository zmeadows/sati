// Task creation times, stored in tasks.json as RFC 3339 UTC strings such as
// "2026-11-07T19:42:05Z" and kept in memory as Unix seconds.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

const SECONDS_PER_DAY: i64 = 86_400;
// Days from 0000-03-01 to 1970-01-01, for the proleptic Gregorian calendar below.
const EPOCH_SHIFT: i64 = 719_468;
const DAYS_PER_ERA: i64 = 146_097;
const FORMAT_HINT: &str = "expected an RFC 3339 UTC time like 2026-11-07T19:42:05Z";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());

        Self(i64::try_from(seconds).unwrap_or(i64::MAX))
    }

    #[cfg(test)]
    pub fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let days = self.0.div_euclid(SECONDS_PER_DAY);
        let second_of_day = self.0.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);

        write!(
            formatter,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            second_of_day / 3600,
            second_of_day / 60 % 60,
            second_of_day % 60
        )
    }
}

impl FromStr for Timestamp {
    type Err = String;

    // Accept exactly "YYYY-MM-DDTHH:MM:SSZ"; offsets and fractional seconds are rejected.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        let separators = [
            (4, b'-'),
            (7, b'-'),
            (10, b'T'),
            (13, b':'),
            (16, b':'),
            (19, b'Z'),
        ];

        let well_formed = bytes.len() == 20
            && separators
                .iter()
                .all(|&(index, separator)| bytes[index] == separator)
            && bytes
                .iter()
                .enumerate()
                .filter(|(index, _)| !separators.iter().any(|(at, _)| at == index))
                .all(|(_, byte)| byte.is_ascii_digit());

        if !well_formed {
            return Err(FORMAT_HINT.to_owned());
        }

        let number = |range: std::ops::Range<usize>| text[range].parse::<i64>().unwrap();
        let (year, month, day) = (number(0..4), number(5..7), number(8..10));
        let (hour, minute, second) = (number(11..13), number(14..16), number(17..19));

        let valid = (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day)
            && hour < 24
            && minute < 60
            && second < 60;

        if !valid {
            return Err(FORMAT_HINT.to_owned());
        }

        let days = days_from_civil(year, month, day);

        Ok(Self(
            days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second,
        ))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);

    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's civil-date algorithms: years start in March so leap days fall last.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + EPOCH_SHIFT;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;

    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * DAYS_PER_ERA + day_of_era - EPOCH_SHIFT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_format_and_parse_as_rfc_3339_utc() {
        for (seconds, text) in [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (946_684_799, "1999-12-31T23:59:59Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
            (1_794_079_445, "2026-11-07T19:24:05Z"),
        ] {
            let timestamp = Timestamp::from_unix_seconds(seconds);

            assert_eq!(timestamp.to_string(), text);
            assert_eq!(text.parse::<Timestamp>(), Ok(timestamp));
        }
    }

    #[test]
    fn every_day_over_several_centuries_round_trips() {
        for days in (-200_000..200_000).step_by(37) {
            let timestamp = Timestamp::from_unix_seconds(days * SECONDS_PER_DAY + 45_296);

            assert_eq!(timestamp.to_string().parse::<Timestamp>(), Ok(timestamp));
        }
    }

    #[test]
    fn malformed_or_impossible_times_are_rejected() {
        for text in [
            "",
            "2026-11-07",
            "2026-11-07T19:24:05",
            "2026-11-07T19:24:05.123Z",
            "2026-11-07T19:24:05+00:00",
            "2026-11-07 19:24:05Z",
            "2026-13-07T19:24:05Z",
            "2026-02-29T00:00:00Z",
            "2100-02-29T00:00:00Z",
            "2026-11-31T00:00:00Z",
            "2026-11-07T24:00:00Z",
            "2026-11-07T23:60:00Z",
            "2026-11-07T23:59:60Z",
            "２026-11-07T19:24:05Z",
        ] {
            assert!(text.parse::<Timestamp>().is_err(), "{text:?}");
        }

        assert!("2024-02-29T00:00:00Z".parse::<Timestamp>().is_ok());
    }

    #[test]
    fn now_is_after_this_code_was_written() {
        assert!(Timestamp::now() > "2026-01-01T00:00:00Z".parse().unwrap());
    }
}
