//! When a gap after an answer falls due.
//!
//! One rule for both of `bdi`'s refresh clocks — the project poll and the
//! band under the forest. They count in different units, so the largest
//! interval each can be given differs by a factor of a thousand, and a bound
//! written at either site would be the wrong number at the other.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

/// The instant `gap` after `answered`, or nothing where that is further
/// ahead than an instant can reach.
///
/// `DateTime + Duration` panics on both counts chrono can overflow on: the
/// conversion to a `TimeDelta`, and the addition. So an interval read from
/// config large enough to outrun time takes `bdi` down at the first answer,
/// rather than at the config line that asked for it.
///
/// Nothing is the answer both clocks already hold for a gap they will not
/// wait out, so a gap that cannot be reached is one nothing waits for. What
/// a bound at config load could not be is right: the reach shrinks as the
/// clock advances — it was 1.8 billion seconds longer at the epoch than it
/// is now — so the only literal safe at every instant is one no measurement
/// gives.
pub(super) fn due_after(answered: DateTime<Utc>, gap: Duration) -> Option<DateTime<Utc>> {
    TimeDelta::from_std(gap)
        .ok()
        .and_then(|gap| answered.checked_add_signed(gap))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("an instant inside the epoch")
    }

    #[test]
    fn a_gap_an_instant_can_reach_falls_due_that_long_after_the_answer() {
        assert_eq!(due_after(at(100), Duration::from_secs(30)), Some(at(130)));
    }

    /// The bound itself: the last instant there is falls due, and a
    /// nanosecond past it does not.
    #[test]
    fn the_gap_to_the_end_of_time_falls_due_and_a_nanosecond_more_does_not() {
        let answered = at(100);
        let to_the_end = (DateTime::<Utc>::MAX_UTC - answered)
            .to_std()
            .expect("the end of time is after the epoch");

        assert_eq!(
            due_after(answered, to_the_end),
            Some(DateTime::<Utc>::MAX_UTC)
        );
        assert_eq!(
            due_after(answered, to_the_end + Duration::from_nanos(1)),
            None
        );
    }

    /// The other count chrono overflows on, which the addition never
    /// reaches: a `Duration` too large to be a `TimeDelta` at all. It is a
    /// thousand times the reach the addition has, so nothing but this test
    /// distinguishes the two.
    #[test]
    fn a_gap_too_long_to_be_an_interval_at_all_falls_due_never() {
        assert_eq!(due_after(at(100), Duration::MAX), None);
    }
}
