use crate::theme::RgbaColor;

/// Frame-rate independent semi-implicit Euler spring solver.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub const fn new(initial: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            value: initial,
            velocity: 0.0,
            target: initial,
            stiffness,
            damping,
        }
    }

    pub const fn calm(initial: f32) -> Self {
        Self::new(initial, 340.0, 28.0)
    }

    pub const fn smooth(initial: f32) -> Self {
        Self::new(initial, 220.0, 28.0)
    }

    pub const fn snappy(initial: f32) -> Self {
        Self::new(initial, 340.0, 34.0)
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() < 0.001 && self.velocity.abs() < 0.005
    }

    /// Advances the spring by `dt` seconds. Returns `true` if still moving.
    pub fn step(&mut self, dt: f32, reduce_motion: bool) -> bool {
        if reduce_motion {
            self.snap(self.target);
            return false;
        }
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }

        // Clamp large frame stalls and sub-step at <= 4ms for deterministic convergence across 60-240Hz.
        let clamped_dt = dt.clamp(0.0, 0.05);
        let substeps = ((clamped_dt / 0.004).ceil() as usize).max(1);
        let h = clamped_dt / substeps as f32;

        for _ in 0..substeps {
            let accel = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }

        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }
}

/// Manages smooth cosine/Gaussian hover magnification across all dock slots.
#[derive(Debug, Clone)]
pub struct DockMagnifier {
    scales: Vec<Spring>,
    lifts: Vec<Spring>,
    pointer_x: Option<f32>,
    slot_centers: Vec<f32>,
    pub max_scale: f32,
    pub radius: f32,
    pub max_lift: f32,
}

impl DockMagnifier {
    pub fn new(max_scale: f32, radius: f32) -> Self {
        Self {
            scales: Vec::new(),
            lifts: Vec::new(),
            pointer_x: None,
            slot_centers: Vec::new(),
            max_scale: max_scale.max(1.0),
            radius: radius.max(32.0),
            max_lift: 10.0,
        }
    }

    pub fn sync_slots(&mut self, slot_centers: Vec<f32>) {
        let len = slot_centers.len();
        self.slot_centers = slot_centers;
        if self.scales.len() != len {
            self.scales.resize(len, Spring::calm(1.0));
            self.lifts.resize(len, Spring::calm(0.0));
        }
        self.recompute_targets();
    }

    pub fn set_pointer_x(&mut self, x: Option<f32>) {
        self.pointer_x = x;
        self.recompute_targets();
    }

    pub fn pointer_x(&self) -> Option<f32> {
        self.pointer_x
    }

    pub fn update_params(&mut self, max_scale: f32, radius: f32) {
        self.max_scale = max_scale.max(1.0);
        self.radius = radius.max(32.0);
        self.recompute_targets();
    }

    fn recompute_targets(&mut self) {
        for (idx, center) in self.slot_centers.iter().enumerate() {
            let (target_scale, target_lift) = match self.pointer_x {
                Some(px) => {
                    let dist = (px - center).abs();
                    if dist >= self.radius {
                        (1.0, 0.0)
                    } else {
                        // Raised-cosine falloff feels smooth and never flickers at boundaries.
                        let t = 0.5 * (1.0 + (std::f32::consts::PI * dist / self.radius).cos());
                        (1.0 + (self.max_scale - 1.0) * t, self.max_lift * t)
                    }
                }
                None => (1.0, 0.0),
            };
            self.scales[idx].set_target(target_scale);
            self.lifts[idx].set_target(target_lift);
        }
    }

    pub fn step(&mut self, dt: f32, reduce_motion: bool) -> bool {
        let mut active = false;
        for s in &mut self.scales {
            active |= s.step(dt, reduce_motion);
        }
        for l in &mut self.lifts {
            active |= l.step(dt, reduce_motion);
        }
        active
    }

    pub fn is_active(&self) -> bool {
        self.scales.iter().any(|s| !s.is_settled()) || self.lifts.iter().any(|l| !l.is_settled())
    }

    pub fn scale_at(&self, index: usize) -> f32 {
        self.scales.get(index).map_or(1.0, |s| s.value.max(1.0))
    }

    pub fn lift_at(&self, index: usize) -> f32 {
        self.lifts.get(index).map_or(0.0, |l| l.value.max(0.0))
    }
}

/// Critically-damped color and opacity transition for Top Bar backdrop fills.
#[derive(Debug, Clone, Copy)]
pub struct BackdropTransition {
    opacity: Spring,
    color_r: Spring,
    color_g: Spring,
    color_b: Spring,
    has_color: bool,
}

impl BackdropTransition {
    pub fn new(fallback: RgbaColor) -> Self {
        Self {
            opacity: Spring::smooth(0.0),
            color_r: Spring::new(fallback.r, 180.0, 26.0),
            color_g: Spring::new(fallback.g, 180.0, 26.0),
            color_b: Spring::new(fallback.b, 180.0, 26.0),
            has_color: false,
        }
    }

    pub fn set_active_color(&mut self, color: RgbaColor, instant: bool) {
        if instant {
            self.opacity.snap(1.0);
            self.color_r.snap(color.r);
            self.color_g.snap(color.g);
            self.color_b.snap(color.b);
            self.has_color = true;
            return;
        }
        if !self.has_color || self.opacity.value <= 0.03 {
            // Snap RGB before fading in from transparent so stale/theme-base colors never flash.
            self.color_r.snap(color.r);
            self.color_g.snap(color.g);
            self.color_b.snap(color.b);
        } else {
            self.color_r.set_target(color.r);
            self.color_g.set_target(color.g);
            self.color_b.set_target(color.b);
        }
        self.has_color = true;
        self.opacity.set_target(1.0);
    }

    pub fn set_inactive(&mut self, instant: bool) {
        if instant {
            self.opacity.snap(0.0);
        } else {
            self.opacity.set_target(0.0);
        }
    }

    pub fn step(&mut self, dt: f32, reduce_motion: bool) -> bool {
        let mut active = false;
        active |= self.opacity.step(dt, reduce_motion);
        active |= self.color_r.step(dt, reduce_motion);
        active |= self.color_g.step(dt, reduce_motion);
        active |= self.color_b.step(dt, reduce_motion);
        self.opacity.value = self.opacity.value.clamp(0.0, 1.0);
        self.color_r.value = self.color_r.value.clamp(0.0, 1.0);
        self.color_g.value = self.color_g.value.clamp(0.0, 1.0);
        self.color_b.value = self.color_b.value.clamp(0.0, 1.0);
        active
    }

    pub fn is_active(&self) -> bool {
        !self.opacity.is_settled()
            || !self.color_r.is_settled()
            || !self.color_g.is_settled()
            || !self.color_b.is_settled()
    }

    pub fn opacity(&self) -> f32 {
        self.opacity.value.clamp(0.0, 1.0)
    }

    pub fn current_color(&self) -> Option<RgbaColor> {
        let a = self.opacity();
        if a <= 0.004 {
            None
        } else {
            Some(RgbaColor::rgba(
                self.color_r.value.clamp(0.0, 1.0),
                self.color_g.value.clamp(0.0, 1.0),
                self.color_b.value.clamp(0.0, 1.0),
                a,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_consistently_across_refresh_rates() {
        for hz in [60, 120, 240] {
            let dt = 1.0 / hz as f32;
            let mut s = Spring::calm(1.0);
            s.set_target(1.4);
            let mut steps = 0;
            while s.step(dt, false) && steps < hz * 3 {
                steps += 1;
            }
            assert!(
                s.is_settled(),
                "Spring failed to settle within 3s at {hz}Hz (val={})",
                s.value
            );
            assert!((s.value - 1.4).abs() < 0.002);
        }
    }

    #[test]
    fn reduce_motion_snaps_immediately() {
        let mut s = Spring::calm(1.0);
        s.set_target(1.5);
        assert!(!s.step(1.0 / 60.0, true));
        assert!(s.is_settled());
        assert_eq!(s.value, 1.5);
    }

    #[test]
    fn dock_magnifier_peaks_at_pointer_and_settles_on_exit() {
        let mut mag = DockMagnifier::new(1.4, 120.0);
        mag.sync_slots(vec![40.0, 100.0, 160.0]);
        mag.set_pointer_x(Some(100.0));
        for _ in 0..90 {
            mag.step(1.0 / 120.0, false);
        }
        assert!(!mag.is_active());
        assert!((mag.scale_at(1) - 1.4).abs() < 0.01);
        assert!(mag.scale_at(0) < mag.scale_at(1));
        assert!((mag.scale_at(0) - mag.scale_at(2)).abs() < 0.01);

        mag.set_pointer_x(None);
        for _ in 0..120 {
            mag.step(1.0 / 120.0, false);
        }
        assert!(!mag.is_active());
        assert!((mag.scale_at(1) - 1.0).abs() < 0.01);
    }

    #[test]
    fn backdrop_transition_snaps_rgb_from_transparent_crossfades_when_visible_and_fades_out() {
        let fallback = RgbaColor::rgba(1.0, 1.0, 1.0, 1.0);
        let mut bd = BackdropTransition::new(fallback);
        assert_eq!(bd.current_color(), None);
        assert!(!bd.is_active());

        // Appearing from transparent snaps RGB so no white flash appears during fade-in.
        let dark_app = RgbaColor::rgba(0.12, 0.14, 0.19, 1.0);
        bd.set_active_color(dark_app, false);
        assert!(bd.is_active());
        bd.step(1.0 / 60.0, false);
        let c = bd.current_color().expect("visible during fade-in");
        assert!((c.r - dark_app.r).abs() < 1e-5);
        assert!((c.g - dark_app.g).abs() < 1e-5);
        assert!((c.b - dark_app.b).abs() < 1e-5);
        assert!(c.a > 0.0 && c.a < 1.0);

        for _ in 0..180 {
            bd.step(1.0 / 60.0, false);
        }
        assert!(!bd.is_active());
        assert!((bd.opacity() - 1.0).abs() < 0.01);

        // Switching while visible smoothly interpolates RGB.
        let light_app = RgbaColor::rgba(0.95, 0.96, 0.98, 1.0);
        bd.set_active_color(light_app, false);
        assert!(bd.is_active());
        bd.step(1.0 / 60.0, false);
        let mid = bd.current_color().expect("still visible");
        assert!(mid.r > dark_app.r && mid.r < light_app.r);

        for _ in 0..180 {
            bd.step(1.0 / 60.0, false);
        }
        let settled = bd.current_color().unwrap();
        assert!((settled.r - light_app.r).abs() < 0.01);

        // Hiding fades smoothly to None.
        bd.set_inactive(false);
        assert!(bd.is_active());
        for _ in 0..180 {
            bd.step(1.0 / 60.0, false);
        }
        assert!(!bd.is_active());
        assert_eq!(bd.current_color(), None);
    }
}
