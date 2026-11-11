//! When a routine runs next, and what a time that a person or the agent wrote means. Routines use
//! a 5-field cron expression (minute hour day month weekday) in a named IANA time zone, so 9:00
//! stays 9:00 across clock changes. All results are UTC milliseconds.

use std::str::FromStr;

use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;

/// The shortest time between two runs of a routine, so one cannot burn its creator's tokens.
pub const MIN_INTERVAL_MS: i64 = 15 * 60 * 1000;
/// How many coming runs the interval check looks at.
const CHECKED_RUNS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    Cron,
    TimeZone,
    TooOften,
    Never,
}

impl ScheduleError {
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::Cron => {
                "The schedule must be a cron expression with 5 fields: minute hour day month weekday, for example `0 9 * * 1-5`."
            }
            Self::TimeZone => "The time zone must be an IANA name such as Europe/Berlin.",
            Self::TooOften => "A routine can run at most every 15 minutes.",
            Self::Never => "This schedule never runs.",
        }
    }
}

/// A checked schedule: the cron expression and its zone.
#[derive(Clone, Debug)]
pub struct RoutineSchedule {
    schedule: Schedule,
    zone: Tz,
}

pub fn time_zone(name: &str) -> Result<Tz, ScheduleError> {
    Tz::from_str(name.trim()).map_err(|_| ScheduleError::TimeZone)
}

impl RoutineSchedule {
    /// Parses `expression` (5 fields) in `zone`. It does not check the interval: see [`Self::check`].
    pub fn parse(expression: &str, zone: &str) -> Result<Self, ScheduleError> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 || expression.len() > 120 {
            return Err(ScheduleError::Cron);
        }
        // The cron crate wants seconds first, and counts weekdays from Sunday = 1.
        let weekdays = standard_weekdays(fields[4]).ok_or(ScheduleError::Cron)?;
        let schedule = Schedule::from_str(&format!("0 {} {weekdays}", fields[..4].join(" ")))
            .map_err(|_| ScheduleError::Cron)?;
        Ok(Self {
            schedule,
            zone: time_zone(zone)?,
        })
    }

    /// Parses and checks that the schedule runs, and never twice within [`MIN_INTERVAL_MS`].
    pub fn check(expression: &str, zone: &str, now_ms: i64) -> Result<Self, ScheduleError> {
        let parsed = Self::parse(expression, zone)?;
        let mut previous: Option<i64> = None;
        let mut after = now_ms;
        for _ in 0..CHECKED_RUNS {
            let Some(next) = parsed.next_after(after) else {
                break;
            };
            if previous.is_some_and(|previous| next - previous < MIN_INTERVAL_MS) {
                return Err(ScheduleError::TooOften);
            }
            previous = Some(next);
            after = next;
        }
        if previous.is_none() {
            return Err(ScheduleError::Never);
        }
        Ok(parsed)
    }

    /// The first run strictly after `after_ms`. A local time that a clock change skips does not
    /// happen that day; a local time that happens twice (the clock goes back) runs once, at its
    /// first occurrence.
    #[must_use]
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        let after = DateTime::<Utc>::from_timestamp_millis(after_ms)?.with_timezone(&self.zone);
        self.schedule
            .after(&after)
            .take(1000)
            .find(|candidate| {
                !matches!(
                    self.zone.from_local_datetime(&candidate.naive_local()),
                    LocalResult::Ambiguous(first, _) if first != *candidate
                )
            })
            .map(|next| next.timestamp_millis())
    }
}

const DAY_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// The weekday field of standard cron (0 or 7 = Sunday, 1 = Monday, …, names too) with names, which
/// the cron crate reads the same way (its numbers start at Sunday = 1). A range that ends on 7
/// (`5-7`) becomes `FRI-SAT,SUN`.
fn standard_weekdays(field: &str) -> Option<String> {
    let day = |text: &str| -> Option<Result<usize, String>> {
        if text.chars().all(|char| char.is_ascii_digit()) {
            let number: usize = text.parse().ok()?;
            (number <= 7).then_some(Ok(number))
        } else {
            Some(Err(text.to_ascii_uppercase()))
        }
    };
    let name = |day: &Result<usize, String>| match day {
        Ok(number) => DAY_NAMES[number % 7].to_owned(),
        Err(name) => name.clone(),
    };
    let mut parts = Vec::new();
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (part, None),
        };
        let step = step.map(|step| format!("/{step}")).unwrap_or_default();
        if range == "*" || range == "?" {
            parts.push(format!("{range}{step}"));
            continue;
        }
        match range.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (day(from)?, day(to)?);
                match (&from, &to) {
                    (Ok(7), _) => return None,
                    (Ok(from_number), Ok(7)) if *from_number > 0 => {
                        if !step.is_empty() {
                            return None;
                        }
                        parts.push(format!("{}-SAT,SUN", name(&from)));
                    }
                    _ => parts.push(format!("{}-{}{step}", name(&from), name(&to))),
                }
            }
            None => parts.push(format!("{}{step}", name(&day(range)?))),
        }
    }
    Some(parts.join(","))
}

/// `ms` as a person reads it in `zone`: `Wed 11 Nov 2026, 17:00 (Europe/Berlin)`.
#[must_use]
pub fn local_time(ms: i64, zone: &str) -> String {
    let Some(time) = DateTime::<Utc>::from_timestamp_millis(ms) else {
        return ms.to_string();
    };
    match time_zone(zone) {
        Ok(tz) => format!(
            "{} ({zone})",
            time.with_timezone(&tz).format("%a %-d %b %Y, %H:%M")
        ),
        Err(_) => format!("{} (UTC)", time.format("%a %-d %b %Y, %H:%M")),
    }
}

/// A time that a person or the agent wrote: RFC 3339 with an offset (`2026-11-11T17:00:00+02:00`,
/// `…Z`), or a local time without one (`2026-11-11T17:00`, `2026-11-11 17:00:00`) in `zone`. A
/// local time that a clock change skips is refused; one that happens twice means the first.
pub fn parse_time(text: &str, zone: &str) -> Result<i64, &'static str> {
    let text = text.trim();
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Ok(time.timestamp_millis());
    }
    let zone = time_zone(zone).map_err(|_| ScheduleError::TimeZone.detail())?;
    let local = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
    .ok_or("The time must look like 2026-11-11T17:00 (local) or 2026-11-11T15:00:00Z.")?;
    match zone.from_local_datetime(&local) {
        LocalResult::Single(time) | LocalResult::Ambiguous(time, _) => Ok(time.timestamp_millis()),
        LocalResult::None => Err("This local time does not exist: the clock skips it."),
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_INTERVAL_MS, RoutineSchedule, ScheduleError, parse_time};
    use chrono::{DateTime, Utc};

    fn ms(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_millis()
    }

    fn utc(ms: i64) -> String {
        DateTime::<Utc>::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339()
    }

    #[test]
    fn weekdays_at_nine_follow_the_zone_across_the_clock_change() {
        let schedule = RoutineSchedule::check("0 9 * * 1-5", "Europe/Berlin", 0).unwrap();
        // Friday 2026-10-23, summer time (UTC+2): the next run is Monday 26 Oct, winter time (UTC+1).
        let friday_after = ms("2026-10-23T08:00:00Z");
        assert_eq!(
            utc(schedule.next_after(friday_after).unwrap()),
            "2026-10-26T08:00:00+00:00"
        );
        let thursday = ms("2026-10-22T06:00:00Z");
        assert_eq!(
            utc(schedule.next_after(thursday).unwrap()),
            "2026-10-22T07:00:00+00:00"
        );
    }

    #[test]
    fn weekdays_count_from_sunday_zero_as_in_standard_cron() {
        // Sunday 2026-11-15 06:00 UTC: the next run of each schedule, by weekday.
        let sunday = ms("2026-11-15T06:00:00Z");
        let next = |cron: &str| {
            utc(RoutineSchedule::check(cron, "UTC", 0)
                .unwrap()
                .next_after(sunday)
                .unwrap())
        };
        assert_eq!(next("0 9 * * 1-5"), "2026-11-16T09:00:00+00:00");
        assert_eq!(next("0 9 * * MON-FRI"), "2026-11-16T09:00:00+00:00");
        assert_eq!(next("0 9 * * 0"), "2026-11-15T09:00:00+00:00");
        assert_eq!(next("0 9 * * 7"), "2026-11-15T09:00:00+00:00");
        assert_eq!(next("0 5 * * 5-7"), "2026-11-20T05:00:00+00:00");
        assert_eq!(next("0 5 * * 1,3"), "2026-11-16T05:00:00+00:00");
        assert_eq!(next("0 5 * * 6"), "2026-11-21T05:00:00+00:00");
        let every_weekday = RoutineSchedule::check("0 9 * * 1-5", "UTC", 0).unwrap();
        // Friday 20 Nov after 9:00: the next is Monday 23 Nov.
        assert_eq!(
            utc(every_weekday
                .next_after(ms("2026-11-20T10:00:00Z"))
                .unwrap()),
            "2026-11-23T09:00:00+00:00"
        );
        assert!(RoutineSchedule::check("0 9 * * 8", "UTC", 0).is_err());
    }

    #[test]
    fn a_skipped_local_time_does_not_run_that_day() {
        // New York springs forward on 2026-03-08 at 02:00: 02:30 does not exist that day.
        let schedule = RoutineSchedule::check("30 2 * * *", "America/New_York", 0).unwrap();
        let next = schedule.next_after(ms("2026-03-07T08:00:00Z")).unwrap();
        assert_eq!(utc(next), "2026-03-09T06:30:00+00:00");
    }

    #[test]
    fn a_repeated_local_time_runs_once() {
        // New York falls back on 2026-11-01: 01:30 happens at 05:30Z and again at 06:30Z.
        let schedule = RoutineSchedule::check("30 1 * * *", "America/New_York", 0).unwrap();
        let first = schedule.next_after(ms("2026-11-01T00:00:00Z")).unwrap();
        assert_eq!(utc(first), "2026-11-01T05:30:00+00:00");
        let after = schedule.next_after(first).unwrap();
        assert_eq!(utc(after), "2026-11-02T06:30:00+00:00");
    }

    #[test]
    fn bad_schedules_are_refused_with_a_reason() {
        let check = |cron: &str, zone: &str| RoutineSchedule::check(cron, zone, 0).err();
        assert_eq!(check("0 9 * *", "UTC"), Some(ScheduleError::Cron));
        assert_eq!(check("0 0 9 * * *", "UTC"), Some(ScheduleError::Cron));
        assert_eq!(check("every day", "UTC"), Some(ScheduleError::Cron));
        assert_eq!(check("61 9 * * *", "UTC"), Some(ScheduleError::Cron));
        assert_eq!(
            check("0 9 * * *", "Mars/Olympus"),
            Some(ScheduleError::TimeZone)
        );
        assert_eq!(check("*/5 * * * *", "UTC"), Some(ScheduleError::TooOften));
        assert_eq!(check("0,10 9 * * *", "UTC"), Some(ScheduleError::TooOften));
        assert_eq!(check("0 9 30 2 *", "UTC"), Some(ScheduleError::Never));
        assert!(check("*/15 * * * *", "UTC").is_none());
        assert!(check("0 9 * * 1-5", "Asia/Jerusalem").is_none());
    }

    #[test]
    fn every_fifteen_minutes_is_the_shortest_interval() {
        let schedule = RoutineSchedule::check("*/15 * * * *", "UTC", 0).unwrap();
        let first = schedule.next_after(ms("2026-11-11T10:01:00Z")).unwrap();
        assert_eq!(schedule.next_after(first).unwrap() - first, MIN_INTERVAL_MS);
    }

    #[test]
    fn times_parse_with_an_offset_or_in_the_zone() {
        assert_eq!(
            parse_time("2026-11-11T15:00:00Z", "Asia/Jerusalem"),
            Ok(ms("2026-11-11T15:00:00Z"))
        );
        assert_eq!(
            parse_time("2026-11-11T17:00", "Asia/Jerusalem"),
            Ok(ms("2026-11-11T15:00:00Z"))
        );
        assert_eq!(
            parse_time("2026-07-01 17:00:00", "Asia/Jerusalem"),
            Ok(ms("2026-07-01T14:00:00Z"))
        );
        assert!(parse_time("2026-03-08T02:30", "America/New_York").is_err());
        assert_eq!(
            parse_time("2026-11-01T01:30", "America/New_York"),
            Ok(ms("2026-11-01T05:30:00Z"))
        );
        assert!(parse_time("tomorrow at five", "UTC").is_err());
        assert!(parse_time("2026-11-11T17:00", "Nowhere/Land").is_err());
    }
}
