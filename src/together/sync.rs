//! Watch together: keeping a guest's player where the host's is.
//!
//! Pure logic (no clocks, no webviews): it gets the host's state, the
//! guest's own reports and the time (ms), and answers with what to do.
//!
//! - **Clock:** the host stamps its state with its own clock; pings tell
//!   the difference between both clocks (the sample with the shortest round
//!   trip of the last five wins).
//! - **Where the host is now:** its position plus the time since it was
//!   measured, at its speed.
//! - **Corrections:** play or pause like the host, and seek when the guest
//!   is more than 1.5 s away (0.75 s when paused). After an action it waits
//!   for its effect before judging again; three failed seeks and it stops
//!   trying (some sites ignore them) until something changes.
//! - **Adverts:** an advert on the host holds everyone (paused); one on the
//!   guest is left to play and the guest catches up afterwards.
//! - **The guest's own clicks:** a pause, play or jump that the app did not
//!   cause becomes a request to the host (if guests may control playback)
//!   or is undone.

use url::Url;

use super::protocol::{follow_url, HostState, Page, Request};
use crate::media::MediaAction;

/// How far off a playing video may drift before seeking (seconds).
const DRIFT_PLAYING: f64 = 1.5;
const DRIFT_PAUSED: f64 = 0.75;
/// Seeking takes a moment; aiming a little ahead lands closer.
const SEEK_LEAD: f64 = 0.35;
/// Time to let an action show in the reports before judging again (ms).
const SETTLE_PLAY_MS: u64 = 2000;
const SETTLE_SEEK_MS: u64 = 2500;
/// A request to the host: time for its answer before undoing it (ms).
const REQUEST_WAIT_MS: u64 = 3500;
const MAX_SEEKS: u32 = 3;
/// A jump bigger than this that the app did not cause was the user's (s).
const USER_JUMP: f64 = 3.0;
/// The page is opened again if it is still not the host's after this (ms).
const NAVIGATE_RETRY_MS: u64 = 12_000;
/// At most one page change this often, whatever the host does (ms).
const NAVIGATE_MIN_MS: u64 = 3_000;
/// Clock samples kept.
const SAMPLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Nothing from the host yet.
    Waiting,
    /// The host is on the start screen, in My music or nowhere.
    HostElsewhere,
    /// The host's page is not a catalog service: open it by hand.
    OtherSite,
    /// Opening the host's page.
    Opening,
    /// The page is open but has no video yet.
    NoVideo,
    InSync,
    Syncing,
    /// An advert on the host: waiting for it.
    HostAd,
    /// An advert here: it plays, and then catches up.
    OwnAd,
    /// Live: only play and pause follow the host.
    Live,
    /// The site ignores seeking.
    CannotSeek,
}

/// The guest's own player, from its reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Own {
    pub media: bool,
    pub playing: bool,
    pub pos: f64,
    pub live: bool,
    pub rate: f64,
    pub ad: bool,
    /// What the page is (protocol::identity), from the webview's address.
    pub identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Media(MediaAction),
    Navigate(Url),
    Request(Request, f64),
    /// Only the host can pause or seek (said at most every 10 s).
    NoControl,
}

#[derive(Debug, Default)]
struct ClockOffset {
    /// (round trip, host − guest), ms.
    samples: Vec<(u64, i64)>,
}

impl ClockOffset {
    fn add(&mut self, t0: u64, t1: u64, t2: u64) {
        if t2 < t0 || t2 - t0 > 10_000 {
            return;
        }
        let rtt = t2 - t0;
        let offset = t1 as i64 - ((t0 + t2) / 2) as i64;
        self.samples.push((rtt, offset));
        if self.samples.len() > SAMPLES {
            self.samples.remove(0);
        }
    }

    fn best(&self) -> Option<i64> {
        self.samples.iter().min_by_key(|(rtt, _)| *rtt).map(|(_, offset)| *offset)
    }

    fn rtt(&self) -> Option<u64> {
        self.samples.iter().map(|(rtt, _)| *rtt).min()
    }
}

#[derive(Debug, Default)]
pub struct Follower {
    host: Option<(HostState, u64)>,
    clock: ClockOffset,
    own: Option<(Own, u64)>,
    /// What the app asked for and until when it waits to see it.
    pending_play: Option<(bool, u64)>,
    pending_seek_until: u64,
    seeks: u32,
    gave_up: bool,
    navigated: Option<(String, u64)>,
    rate_sent: Option<f64>,
    last_no_control: Option<u64>,
    pub control: bool,
    status: Option<Status>,
}

impl Follower {
    pub fn status(&self) -> Status {
        self.status.unwrap_or(Status::Waiting)
    }

    pub fn host(&self) -> Option<&HostState> {
        self.host.as_ref().map(|(state, _)| state)
    }

    /// Round trip to the host (ms), once measured.
    pub fn rtt(&self) -> Option<u64> {
        self.clock.rtt()
    }

    pub fn on_pong(&mut self, t0: u64, t1: u64, now: u64) {
        self.clock.add(t0, t1, now);
    }

    /// Host ms that corresponds to `now` here.
    fn host_now(&self, now: u64) -> Option<i64> {
        let (state, received) = self.host.as_ref()?;
        // Before the first pong: as if the state had arrived instantly.
        let offset = self.clock.best().unwrap_or(state.at as i64 - *received as i64);
        Some(now as i64 + offset)
    }

    /// Where the host is now (seconds).
    pub fn expected(&self, now: u64) -> Option<f64> {
        let (state, _) = self.host.as_ref()?;
        Some(state.position_after(self.host_now(now)? - state.at as i64))
    }

    /// "Resync" in the panel: try again from scratch.
    pub fn reset(&mut self) {
        self.seeks = 0;
        self.gave_up = false;
        self.pending_play = None;
        self.pending_seek_until = 0;
        self.navigated = None;
        self.rate_sent = None;
    }

    pub fn on_host_state(&mut self, state: HostState, now: u64) -> Vec<Action> {
        // A jump or a new video on the host's side: start counting again.
        let jumped = match self.host.as_ref() {
            Some((old, received)) => {
                let elapsed = now.saturating_sub(*received) as i64;
                old.page.identity() != state.page.identity() || (old.position_after(elapsed) - state.pos).abs() > 2.0
            }
            None => true,
        };
        if jumped {
            self.seeks = 0;
            self.gave_up = false;
        }
        self.host = Some((state, now));
        self.evaluate(now)
    }

    pub fn on_own(&mut self, own: Own, now: u64) -> Vec<Action> {
        let mut actions = Vec::new();
        if let (Some((prev, prev_at)), Some((host, _))) = (self.own.as_ref(), self.host.as_ref()) {
            let same_page = own.identity.is_some() && own.identity == host.page.identity() && prev.identity == own.identity;
            let calm = same_page && own.media && prev.media && !own.ad && !prev.ad && !host.ad && host.media;
            if calm {
                actions.extend(self.user_actions(prev.clone(), *prev_at, &own, now));
            }
        }
        self.own = Some((own, now));
        actions.extend(self.evaluate(now));
        actions
    }

    /// Clicks on the guest's own player that the app did not cause.
    fn user_actions(&mut self, prev: Own, prev_at: u64, own: &Own, now: u64) -> Vec<Action> {
        let Some((host, _)) = self.host.as_ref() else { return Vec::new() };
        let host_playing = host.playing;
        let ours = self.pending_play.is_some_and(|(_, until)| now < until);
        let mut actions = Vec::new();
        if prev.playing != own.playing && !ours && own.playing != host_playing {
            if self.control {
                let request = if own.playing { Request::Play } else { Request::Pause };
                actions.push(Action::Request(request, own.pos));
                self.pending_play = Some((own.playing, now + REQUEST_WAIT_MS));
            } else {
                actions.extend(self.no_control(now));
            }
        }
        let predicted = prev.pos + if prev.playing { now.saturating_sub(prev_at) as f64 / 1000.0 * prev.rate.max(0.25) } else { 0.0 };
        // Landing where the host is was the app's own seek.
        let away_from_host = self.expected(now).is_none_or(|expected| (own.pos - expected).abs() > USER_JUMP);
        if !own.live && (own.pos - predicted).abs() > USER_JUMP && away_from_host && now >= self.pending_seek_until {
            if self.control {
                actions.push(Action::Request(Request::Seek, own.pos));
                self.pending_seek_until = now + REQUEST_WAIT_MS;
            } else {
                actions.extend(self.no_control(now));
            }
        }
        actions
    }

    fn no_control(&mut self, now: u64) -> Option<Action> {
        if self.last_no_control.is_some_and(|at| now < at + 10_000) {
            return None;
        }
        self.last_no_control = Some(now);
        Some(Action::NoControl)
    }

    /// Compares and corrects. Also called on the timer.
    pub fn evaluate(&mut self, now: u64) -> Vec<Action> {
        let (status, actions) = self.decide(now);
        self.status = Some(status);
        actions
    }

    fn decide(&mut self, now: u64) -> (Status, Vec<Action>) {
        let mut actions = Vec::new();
        let Some((host, _)) = self.host.clone() else { return (Status::Waiting, actions) };
        let identity = host.page.identity();
        let follow = match &host.page {
            Page::Service { url } => follow_url(url),
            Page::Site { .. } => None,
            Page::Home | Page::Music | Page::None => return (Status::HostElsewhere, actions),
        };
        let Some((own, own_at)) = self.own.clone() else {
            // No report yet (start screen): open the host's page.
            if let Some(url) = follow {
                if self.navigation_due(identity.as_ref(), now) {
                    self.navigated = identity.clone().map(|id| (id, now));
                    actions.push(Action::Navigate(url));
                }
                return (Status::Opening, actions);
            }
            return (Status::OtherSite, actions);
        };
        if own.identity != identity {
            return match follow {
                Some(url) => {
                    if self.navigation_due(identity.as_ref(), now) {
                        self.navigated = identity.map(|id| (id, now));
                        self.reset_corrections();
                        actions.push(Action::Navigate(url));
                    }
                    (Status::Opening, actions)
                }
                None => (Status::OtherSite, actions),
            };
        }
        if !own.media || !host.media {
            return (Status::NoVideo, actions);
        }
        if own.ad {
            return (Status::OwnAd, actions);
        }
        let pending_play = self.pending_play.filter(|(_, until)| now < *until);
        if host.ad {
            if own.playing && pending_play.is_none() {
                actions.push(Action::Media(MediaAction::Pause));
                self.pending_play = Some((false, now + SETTLE_PLAY_MS));
            }
            return (Status::HostAd, actions);
        }
        // Speed.
        if !host.live && (own.rate - host.rate).abs() > 0.01 && self.rate_sent != Some(host.rate) {
            self.rate_sent = Some(host.rate);
            actions.push(Action::Media(MediaAction::Rate((host.rate * 100.0).round() as u16)));
        }
        // Play and pause.
        let mut playing_differs = false;
        if own.playing != host.playing {
            playing_differs = true;
            if pending_play.is_none() {
                actions.push(Action::Media(if host.playing { MediaAction::Play } else { MediaAction::Pause }));
                self.pending_play = Some((host.playing, now + SETTLE_PLAY_MS));
            }
        }
        if host.live || own.live {
            return (if playing_differs { Status::Syncing } else { Status::Live }, actions);
        }
        // Pausing: where it stops is judged once it has stopped. Starting:
        // the seek goes along with the play.
        if playing_differs && !host.playing {
            return (Status::Syncing, actions);
        }
        // Position.
        let Some(expected) = self.expected(now) else { return (Status::Waiting, actions) };
        let elapsed = now.saturating_sub(own_at) as f64 / 1000.0;
        let own_pos = own.pos + if own.playing { elapsed * own.rate.max(0.25) } else { 0.0 };
        let drift = own_pos - expected;
        let limit = if host.playing { DRIFT_PLAYING } else { DRIFT_PAUSED };
        if now < self.pending_seek_until {
            return (Status::Syncing, actions);
        }
        if drift.abs() <= limit {
            self.seeks = 0;
            self.gave_up = false;
            return (if playing_differs { Status::Syncing } else { Status::InSync }, actions);
        }
        if self.gave_up || self.seeks >= MAX_SEEKS {
            self.gave_up = true;
            return (Status::CannotSeek, actions);
        }
        let target = expected + if host.playing { SEEK_LEAD } else { 0.0 };
        let target = if host.dur > 0.0 { target.min(host.dur) } else { target };
        actions.push(Action::Media(MediaAction::SeekToMs((target.max(0.0) * 1000.0).round() as u64)));
        self.pending_seek_until = now + SETTLE_SEEK_MS;
        self.seeks += 1;
        (Status::Syncing, actions)
    }

    /// A new page now and then; the same one again only if it never loaded.
    fn navigation_due(&self, identity: Option<&String>, now: u64) -> bool {
        match &self.navigated {
            None => true,
            Some((id, at)) if Some(id) == identity => now >= at + NAVIGATE_RETRY_MS,
            Some((_, at)) => now >= at + NAVIGATE_MIN_MS,
        }
    }

    fn reset_corrections(&mut self) {
        self.seeks = 0;
        self.gave_up = false;
        self.pending_play = None;
        self.pending_seek_until = 0;
        self.rate_sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

    fn host(pos: f64, playing: bool, at: u64) -> HostState {
        HostState {
            at,
            page: Page::Service { url: VIDEO.into() },
            media: true,
            playing,
            pos,
            dur: 600.0,
            rate: 1.0,
            ..Default::default()
        }
    }

    fn own(pos: f64, playing: bool) -> Own {
        Own { media: true, playing, pos, rate: 1.0, identity: Some("yt:dQw4w9WgXcQ".into()), ..Default::default() }
    }

    fn seeks(actions: &[Action]) -> Vec<u64> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Media(MediaAction::SeekToMs(ms)) => Some(*ms),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn opens_the_hosts_video_first() {
        let mut f = Follower::default();
        let actions = f.on_host_state(host(10.0, true, 5_000), 1_000);
        assert_eq!(actions, vec![Action::Navigate(Url::parse(VIDEO).unwrap())]);
        assert_eq!(f.status(), Status::Opening);
        // Not again while it loads...
        let other = Own { identity: Some("www.youtube.com".into()), ..own(0.0, false) };
        assert!(f.on_own(other.clone(), 2_000).is_empty());
        // ...but again if it never got there.
        assert_eq!(f.on_own(other, 1_000 + NAVIGATE_RETRY_MS).len(), 1);
    }

    #[test]
    fn a_host_cannot_bounce_guests_between_pages() {
        let mut f = Follower::default();
        let page = |n: u32| HostState { page: Page::Service { url: format!("https://www.netflix.com/watch/{n}") }, ..host(0.0, true, 0) };
        assert_eq!(f.on_host_state(page(1), 0).len(), 1);
        assert!(f.on_host_state(page(2), 500).is_empty(), "too soon");
        assert_eq!(f.on_host_state(page(3), NAVIGATE_MIN_MS).len(), 1);
    }

    #[test]
    fn catches_up_with_a_playing_host_using_the_clock_offset() {
        let mut f = Follower::default();
        // Host clock = guest clock + 100 000 ms; a 40 ms round trip.
        f.on_pong(1_000, 101_020, 1_040);
        f.on_host_state(host(30.0, true, 101_500), 1_520);
        // The guest is at 12 s, paused: play, and seek where the host is.
        let actions = f.on_own(own(12.0, false), 2_000);
        assert!(actions.contains(&Action::Media(MediaAction::Play)));
        let ms = seeks(&actions);
        assert_eq!(ms.len(), 1);
        // Host at 2 000 + 100 000 → 30 s + 0.5 s; plus the lead.
        assert!((ms[0] as f64 - (30.5 + SEEK_LEAD) * 1000.0).abs() < 2.0, "{ms:?}");
        assert_eq!(f.status(), Status::Syncing);
        // Once there, in sync.
        let actions = f.on_own(own(33.0, true), 5_000);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(f.status(), Status::InSync);
    }

    #[test]
    fn small_drift_is_left_alone_and_pauses_follow() {
        let mut f = Follower::default();
        f.on_host_state(host(100.0, true, 50_000), 50_000);
        assert!(f.on_own(own(101.0, true), 50_000).is_empty());
        assert_eq!(f.status(), Status::InSync);
        let actions = f.on_host_state(host(101.2, false, 51_200), 51_200);
        assert_eq!(actions, vec![Action::Media(MediaAction::Pause)]);
    }

    #[test]
    fn gives_up_on_sites_that_ignore_seeking() {
        let mut f = Follower::default();
        f.on_host_state(host(300.0, false, 0), 0);
        let mut now = 0;
        let mut total = 0;
        for _ in 0..6 {
            now += SETTLE_SEEK_MS + 1;
            total += seeks(&f.on_own(own(10.0, false), now)).len();
        }
        assert_eq!(total, MAX_SEEKS as usize);
        assert_eq!(f.status(), Status::CannotSeek);
        // "Resync" tries again.
        f.reset();
        assert_eq!(seeks(&f.on_own(own(10.0, false), now + 10)).len(), 1);
    }

    #[test]
    fn adverts_hold_or_wait() {
        let mut f = Follower::default();
        f.on_host_state(HostState { ad: true, ..host(50.0, true, 0) }, 0);
        assert_eq!(f.on_own(own(50.0, true), 100), vec![Action::Media(MediaAction::Pause)]);
        assert_eq!(f.status(), Status::HostAd);
        let mut f = Follower::default();
        f.on_host_state(host(50.0, true, 0), 0);
        assert!(f.on_own(Own { ad: true, ..own(3.0, true) }, 100).is_empty());
        assert_eq!(f.status(), Status::OwnAd);
    }

    #[test]
    fn the_guests_own_pause_becomes_a_request_or_is_undone() {
        let mut f = Follower { control: true, ..Default::default() };
        f.on_host_state(host(10.0, true, 10_000), 10_000);
        f.on_own(own(10.0, true), 10_000);
        let actions = f.on_own(own(11.0, false), 11_000);
        assert_eq!(actions, vec![Action::Request(Request::Pause, 11.0)], "a request, and no undo while waiting");
        // Without control: undone, and said once.
        let mut f = Follower::default();
        f.on_host_state(host(10.0, true, 10_000), 10_000);
        f.on_own(own(10.0, true), 10_000);
        let actions = f.on_own(own(11.0, false), 11_000);
        assert!(actions.contains(&Action::NoControl));
        assert!(actions.contains(&Action::Media(MediaAction::Play)));
    }

    #[test]
    fn a_jump_by_the_guest_is_a_seek_request() {
        let mut f = Follower { control: true, ..Default::default() };
        f.on_host_state(host(10.0, true, 0), 0);
        f.on_own(own(10.0, true), 0);
        let actions = f.on_own(own(200.0, true), 1_000);
        assert_eq!(actions, vec![Action::Request(Request::Seek, 200.0)]);
        // The host moves there: in sync, no seek back.
        assert!(f.on_host_state(host(200.0, true, 1_300), 1_300).is_empty());
    }

    #[test]
    fn live_streams_only_follow_play_and_pause() {
        let mut f = Follower::default();
        f.on_host_state(HostState { live: true, ..host(5000.0, true, 0) }, 0);
        assert!(seeks(&f.on_own(Own { live: true, ..own(10.0, true) }, 100)).is_empty());
        assert_eq!(f.status(), Status::Live);
    }

    #[test]
    fn host_elsewhere_or_on_another_site() {
        let mut f = Follower::default();
        assert!(f.on_host_state(HostState { page: Page::Music, ..Default::default() }, 0).is_empty());
        assert_eq!(f.status(), Status::HostElsewhere);
        let site = HostState { page: Page::Site { url: "https://example.com/v".into() }, ..host(1.0, true, 0) };
        assert!(f.on_host_state(site, 0).is_empty());
        assert_eq!(f.status(), Status::OtherSite);
        // Opened by hand: it syncs.
        let mine = Own { identity: Some("example.com/v".into()), ..own(1.0, true) };
        assert!(f.on_own(mine, 10).is_empty());
        assert_eq!(f.status(), Status::InSync);
    }

    #[test]
    fn speed_follows_the_host() {
        let mut f = Follower::default();
        f.on_host_state(HostState { rate: 1.5, ..host(10.0, true, 0) }, 0);
        let actions = f.on_own(own(10.0, true), 0);
        assert_eq!(actions, vec![Action::Media(MediaAction::Rate(150))]);
        assert!(f.on_own(own(10.0, true), 10).is_empty(), "asked once");
    }

    #[test]
    fn clock_uses_the_fastest_round_trip() {
        let mut c = ClockOffset::default();
        c.add(0, 1_000_100, 400); // slow
        c.add(1_000, 1_001_020, 1_040); // fast
        assert_eq!(c.best(), Some(1_000_000));
        assert_eq!(c.rtt(), Some(40));
        c.add(2_000, 0, 1_000); // answer before the question: ignored
        assert_eq!(c.samples.len(), 2);
    }
}
