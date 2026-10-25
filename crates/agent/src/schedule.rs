//! When a task is due, in local time. Plain functions of the schedule and
//! the time, so they're tested without waiting for a clock.

use jiff::civil::{Date, Time};
use jiff::{Timestamp, ToSpan, Zoned};
use protocol::Schedule;

/// A run that starts this long after its time is late.
pub const LATE: jiff::SignedDuration = jiff::SignedDuration::from_mins(2);

fn time_of(text: &str) -> Option<Time> {
    let (h, m) = text.split_once(':')?;
    Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()
}

/// Monday is 0, as the schedule counts.
fn weekday(date: Date) -> u8 {
    date.weekday().to_monday_zero_offset() as u8
}

/// The moment `date` at `time` is, where `now` is. None for a time a
/// clock change skips.
fn at(date: Date, time: Time, now: &Zoned) -> Option<Timestamp> {
    date.to_datetime(time)
        .to_zoned(now.time_zone().clone())
        .ok()
        .map(|z| z.timestamp())
}

/// The latest time the schedule was due at or before `now`, if that's no
/// earlier than `since`. Missed several? This is only the last of them.
pub fn last_due(schedule: &Schedule, since: Timestamp, now: &Zoned) -> Option<Timestamp> {
    let due = match schedule {
        Schedule::Manual => return None,
        Schedule::Every { hours } => {
            let step = i64::from((*hours).max(1)) * 3600;
            let gone = now.timestamp().as_second() - since.as_second();
            if gone < step {
                return None;
            }
            since.checked_add(((gone / step) * step).seconds()).ok()?
        }
        Schedule::At { days, time } => {
            let time = time_of(time)?;
            (0..8)
                .filter_map(|back| now.date().checked_sub(back.days()).ok())
                .filter(|date| days.contains(&weekday(*date)))
                .filter_map(|date| at(date, time, now))
                .find(|due| *due <= now.timestamp())?
        }
    };
    (due >= since).then_some(due)
}

/// The next time the schedule is due after `now`, counting from `since`.
pub fn next_due(schedule: &Schedule, since: Timestamp, now: &Zoned) -> Option<Timestamp> {
    match schedule {
        Schedule::Manual => None,
        Schedule::Every { hours } => {
            let step = i64::from((*hours).max(1)) * 3600;
            let gone = (now.timestamp().as_second() - since.as_second()).max(0);
            since.checked_add(((gone / step + 1) * step).seconds()).ok()
        }
        Schedule::At { days, time } => {
            let time = time_of(time)?;
            (0..8)
                .filter_map(|ahead| now.date().checked_add(ahead.days()).ok())
                .filter(|date| days.contains(&weekday(*date)))
                .filter_map(|date| at(date, time, now))
                .find(|due| *due > now.timestamp() && *due >= since)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 26 October 2026, in London: the day after the clocks go back.
    fn monday(time: &str) -> Zoned {
        format!("2026-10-26T{time}[Europe/London]").parse().unwrap()
    }

    fn weekdays_at(time: &str) -> Schedule {
        Schedule::At {
            days: vec![0, 1, 2, 3, 4],
            time: time.into(),
        }
    }

    #[test]
    fn due_once_its_time_has_come() {
        let since = monday("06:00").timestamp();
        let schedule = weekdays_at("08:00");
        assert_eq!(last_due(&schedule, since, &monday("07:59")), None);
        assert_eq!(
            last_due(&schedule, since, &monday("08:00")),
            Some(monday("08:00").timestamp())
        );
        assert_eq!(
            next_due(&schedule, since, &monday("08:00")),
            Some(monday("08:00").timestamp() + 24.hours())
        );
    }

    #[test]
    fn a_missed_weekend_is_one_run() {
        // Last handled Friday at 08:00; the app comes back on Monday at 09:30.
        let friday = "2026-10-23T08:00:01[Europe/London]"
            .parse::<Zoned>()
            .unwrap();
        let due = last_due(&weekdays_at("08:00"), friday.timestamp(), &monday("09:30"));
        assert_eq!(due, Some(monday("08:00").timestamp()));
    }

    #[test]
    fn nothing_before_it_was_made() {
        let made = monday("09:00").timestamp();
        assert_eq!(
            last_due(&weekdays_at("08:00"), made, &monday("10:00")),
            None
        );
        let next = next_due(&weekdays_at("08:00"), made, &monday("10:00")).unwrap();
        assert_eq!(
            next,
            "2026-10-27T08:00[Europe/London]"
                .parse::<Zoned>()
                .unwrap()
                .timestamp()
        );
    }

    #[test]
    fn every_few_hours_counts_from_when_it_was_made() {
        let made = monday("09:00").timestamp();
        let every = Schedule::Every { hours: 2 };
        assert_eq!(last_due(&every, made, &monday("10:59")), None);
        assert_eq!(
            last_due(&every, made, &monday("15:30")),
            Some(monday("15:00").timestamp())
        );
        assert_eq!(
            next_due(&every, made, &monday("15:30")),
            Some(monday("17:00").timestamp())
        );
    }

    #[test]
    fn manual_never_comes_due() {
        let made = monday("09:00").timestamp();
        assert_eq!(last_due(&Schedule::Manual, made, &monday("23:00")), None);
        assert_eq!(next_due(&Schedule::Manual, made, &monday("23:00")), None);
    }
}
