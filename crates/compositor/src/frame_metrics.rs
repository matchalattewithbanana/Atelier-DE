//! Opt-in bounded wall-clock diagnostics. No GPU queries or extra fences.
use std::time::{Duration, Instant};

const PERIOD: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Samples {
    count: u64,
    total: Duration,
    max: Duration,
}

impl Samples {
    fn add(&mut self, value: Duration) {
        self.count = self.count.saturating_add(1);
        self.total = self.total.saturating_add(value);
        self.max = self.max.max(value);
    }

    fn mean_ms(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.total.as_secs_f64() * 1000.0 / self.count as f64 }
    }

    fn max_ms(&self) -> f64 { self.max.as_secs_f64() * 1000.0 }
}

/// A disabled sample takes no clock readings. Only successfully submitted
/// frames enter the statistics; an abandoned render is silently discarded.
pub struct FrameTiming {
    start: Option<Instant>,
    scene: Option<Instant>,
    render: Option<Instant>,
}

impl FrameTiming {
    fn new(enabled: bool) -> Self {
        Self { start: enabled.then(Instant::now), scene: None, render: None }
    }
    pub fn scene_ready(&mut self) {
        if self.start.is_some() { self.scene = Some(Instant::now()); }
    }
    pub fn render_ready(&mut self) {
        if self.scene.is_some() { self.render = Some(Instant::now()); }
    }
}

struct Window {
    start: Instant,
    preparation: Samples,
    rendering: Samples,
    submission: Samples,
    total: Samples,
    flips: Samples,
}

impl Window {
    fn new(start: Instant) -> Self {
        Self { start, preparation: Samples::default(), rendering: Samples::default(),
            submission: Samples::default(), total: Samples::default(), flips: Samples::default() }
    }

    fn record(&mut self, timing: FrameTiming, end: Instant) {
        let (Some(start), Some(scene), Some(render)) = (timing.start, timing.scene, timing.render) else { return; };
        self.preparation.add(scene.saturating_duration_since(start));
        self.rendering.add(render.saturating_duration_since(scene));
        self.submission.add(end.saturating_duration_since(render));
        self.total.add(end.saturating_duration_since(start));
    }

    fn rotate(&mut self, now: Instant) -> Option<Self> {
        if now.saturating_duration_since(self.start) < PERIOD { return None; }
        Some(std::mem::replace(self, Self::new(now)))
    }
}

/// The only switch is a process-local environment variable. Aggregation is
/// constant space and emits at most one line per five-second window.
pub struct FrameMetrics {
    backend: &'static str,
    window: Option<Window>,
}

impl FrameMetrics {
    pub fn new(backend: &'static str) -> Self {
        let enabled = std::env::var("ATELIER_FRAME_METRICS").as_deref() == Ok("1");
        Self { backend, window: enabled.then(|| Window::new(Instant::now())) }
    }

    pub fn begin_frame(&self) -> FrameTiming { FrameTiming::new(self.window.is_some()) }

    pub fn submitted(&mut self, timing: FrameTiming) {
        if let Some(window) = &mut self.window { window.record(timing, Instant::now()); }
    }

    #[cfg(feature = "native-session")]
    pub fn flip_completed(&mut self, submitted: Option<Instant>) {
        if let (Some(window), Some(submitted)) = (&mut self.window, submitted) {
            window.flips.add(submitted.elapsed());
        }
    }

    pub fn report_due(&mut self) {
        let Some(window) = &mut self.window else { return; };
        let now = Instant::now();
        let Some(previous) = window.rotate(now) else { return; };
        let seconds = now.saturating_duration_since(previous.start).as_secs_f64();
        println!(
            "Atelier frame metrics backend={} interval_s={:.3} submitted={} submit_hz={:.2} prepare_mean_ms={:.3} prepare_max_ms={:.3} render_mean_ms={:.3} render_max_ms={:.3} submit_mean_ms={:.3} submit_max_ms={:.3} total_mean_ms={:.3} total_max_ms={:.3} observed_flips={} flip_mean_ms={:.3} flip_max_ms={:.3}",
            self.backend, seconds, previous.total.count, previous.total.count as f64 / seconds,
            previous.preparation.mean_ms(), previous.preparation.max_ms(),
            previous.rendering.mean_ms(), previous.rendering.max_ms(),
            previous.submission.mean_ms(), previous.submission.max_ms(),
            previous.total.mean_ms(), previous.total.max_ms(),
            previous.flips.count, previous.flips.mean_ms(), previous.flips.max_ms()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_phase_cost_and_rotates_even_when_idle() {
        let start = Instant::now();
        let mut window = Window::new(start);
        for ms in [2, 4] {
            window.record(FrameTiming {
                start: Some(start),
                scene: Some(start + Duration::from_millis(ms)),
                render: Some(start + Duration::from_millis(ms + 5)),
            }, start + Duration::from_millis(ms + 6));
        }
        assert_eq!(window.total.count, 2);
        assert_eq!(window.preparation.mean_ms(), 3.0);
        assert_eq!(window.preparation.max_ms(), 4.0);
        assert_eq!(window.rendering.mean_ms(), 5.0);
        assert_eq!(window.submission.mean_ms(), 1.0);
        assert_eq!(window.total.mean_ms(), 9.0);
        assert!(window.rotate(start + PERIOD - Duration::from_millis(1)).is_none());
        let report = window.rotate(start + PERIOD).unwrap();
        assert_eq!(report.total.count, 2);
        let idle = window.rotate(start + PERIOD + PERIOD).unwrap();
        assert_eq!(idle.total.count, 0);
        assert_eq!(idle.total.mean_ms(), 0.0);
    }

    #[test]
    fn disabled_or_incomplete_samples_are_not_counted() {
        let now = Instant::now();
        let mut window = Window::new(now);
        let mut disabled = FrameTiming::new(false);
        disabled.scene_ready();
        disabled.render_ready();
        assert!(disabled.start.is_none() && disabled.scene.is_none() && disabled.render.is_none());
        window.record(disabled, now);
        window.record(FrameTiming { start: Some(now), scene: None, render: None }, now);
        assert_eq!(window.total.count, 0);
    }
}
