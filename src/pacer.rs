//! Keeps the shared window's pictures at an even rate.
//!
//! The screencast hands over a picture whenever the page is drawn: for a
//! 30 fps video on a 60 Hz screen that is roughly every 33 ms, but some come
//! after 17 ms and the next after 50. Dropping every picture that comes
//! "too soon" after the last one would then drop good pictures and make
//! the video stutter. Instead the pacer earns credit with time (30 a
//! second, up to a small reserve) and spends one per picture sent: on
//! average no more than 30 a second go out, and uneven arrivals still do.

use std::time::Instant;

/// Most credit kept in reserve (pictures).
const RESERVE: f64 = 2.0;

#[derive(Debug, Clone, Copy)]
pub struct Pacer {
    per_second: f64,
    credit: f64,
    last: Option<Instant>,
}

impl Pacer {
    pub fn new(per_second: u32) -> Self {
        Self { per_second: f64::from(per_second.max(1)), credit: 1.0, last: None }
    }

    /// A picture arrived at `now`: whether to send it on.
    pub fn take(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last {
            let earned = now.saturating_duration_since(last).as_secs_f64() * self.per_second;
            self.credit = (self.credit + earned).min(RESERVE);
        }
        self.last = Some(now);
        if self.credit >= 1.0 {
            self.credit -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn run(pacer: &mut Pacer, start: Instant, gaps_ms: &[u64]) -> usize {
        let mut at = start;
        let mut sent = 0;
        for gap in gaps_ms {
            at += Duration::from_millis(*gap);
            sent += usize::from(pacer.take(at));
        }
        sent
    }

    #[test]
    fn sixty_a_second_become_thirty() {
        let mut pacer = Pacer::new(30);
        let sent = run(&mut pacer, Instant::now(), &[17; 600]);
        // 10.2 s of a 60 Hz screen: 306 out (give or take the first one).
        assert!((305..=307).contains(&sent), "{sent}");
    }

    #[test]
    fn uneven_thirty_a_second_all_go_through() {
        // A 30 fps video drawn on a 60 Hz screen: 17, 50, 34, 34…
        let gaps: Vec<u64> = (0..300).map(|i| [17, 50, 34, 34][i % 4]).collect();
        let mut pacer = Pacer::new(30);
        let sent = run(&mut pacer, Instant::now(), &gaps);
        assert_eq!(sent, 300, "none of them dropped");
        // The old rule (at least 31 ms since the last one) dropped a quarter.
        let mut last = 0u64;
        let mut at = 0u64;
        let mut old = 0;
        for gap in &gaps {
            at += gap;
            if at - last >= 31 {
                old += 1;
                last = at;
            }
        }
        assert!(old < 230, "{old}");
    }

    #[test]
    fn a_pause_does_not_store_a_burst() {
        let mut pacer = Pacer::new(30);
        let start = Instant::now();
        assert!(pacer.take(start));
        // Nothing for 5 s, then 10 pictures at once: only the reserve goes.
        let later = start + Duration::from_secs(5);
        let burst = (0..10).filter(|_| pacer.take(later)).count();
        assert_eq!(burst, 2);
    }
}

#[cfg(test)]
#[path = "tests/pacer_cases.rs"]
mod cases;
