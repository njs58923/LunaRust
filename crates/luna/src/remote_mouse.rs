//! Remote pointer input: absolute samples with filtered cursor recentering.
use bevy::prelude::*;

#[derive(Resource)]
pub(crate) struct RemoteMouse {
    pub absolute: bool,
}

impl Default for RemoteMouse {
    fn default() -> Self {
        let forced = match std::env::var("LUNA_MOUSE_INPUT").ok().as_deref() {
            Some("absolute") => Some(true),
            Some("raw") => Some(false),
            _ => None,
        };
        let absolute = forced.unwrap_or_else(remote_session);
        if absolute {
            info!("Remote mouse: captured absolute input; right click to look, Escape to release");
        }
        // Detect once during plugin initialization, never in the frame loop.
        Self { absolute }
    }
}

#[cfg(test)]
impl RemoteMouse {
    pub fn forced(absolute: bool) -> Self {
        Self { absolute }
    }
}

#[cfg(not(target_os = "windows"))]
fn remote_session() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn remote_session() -> bool {
    use std::ffi::c_void;
    #[link(name = "Wtsapi32")]
    extern "system" {
        fn WTSQuerySessionInformationW(
            server: *mut c_void,
            session: u32,
            class: i32,
            buffer: *mut *mut u16,
            bytes: *mut u32,
        ) -> i32;
        fn WTSFreeMemory(buffer: *mut c_void);
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }

    // Current process session, not the active console session or a cached env var.
    // WTSIsRemoteSession also handles configurations where SM_REMOTESESSION fails.
    // https://learn.microsoft.com/windows/win32/api/wtsapi32/ne-wtsapi32-wts_info_class
    unsafe {
        for (class, width) in [(29, 4), (16, 2)] {
            let mut buffer = std::ptr::null_mut();
            let mut bytes = 0;
            let ok = WTSQuerySessionInformationW(
                std::ptr::null_mut(),
                u32::MAX,
                class,
                &mut buffer,
                &mut bytes,
            );
            let result = if ok != 0 && !buffer.is_null() && bytes >= width {
                Some(if width == 4 {
                    std::ptr::read_unaligned(buffer.cast::<i32>()) != 0
                } else {
                    std::ptr::read_unaligned(buffer) != 0
                })
            } else {
                None
            };
            if !buffer.is_null() {
                WTSFreeMemory(buffer.cast());
            }
            if let Some(remote) = result {
                return remote;
            }
        }
        GetSystemMetrics(0x1000) != 0 // SM_REMOTESESSION; legacy fallback only.
    }
}

#[derive(Default)]
pub(crate) struct AbsolutePointer {
    previous: Option<Vec2>,
    metrics: Option<(Vec2, f32)>,
    pending_center: Option<Vec2>,
}

impl AbsolutePointer {
    pub fn reset(&mut self) {
        self.previous = None;
        self.metrics = None;
        self.pending_center = None;
    }

    /// Warp only near an edge, after draining input. Keep the old reference
    /// until the warp arrives: queued pre-warp RDP packets remain valid input.
    pub fn recenter(&mut self) -> Option<Vec2> {
        let (size, _) = self.metrics?;
        let position = self.previous?;
        if self.pending_center.is_some() { return None; }
        let margin = size * 0.15;
        if position.cmplt(margin).any() || position.cmpgt(size - margin).any() {
            let center = size * 0.5;
            self.pending_center = Some(center);
            return Some(center);
        }
        None
    }

    pub fn sample(&mut self, position: Vec2, size: Vec2, scale: f32) -> Vec2 {
        if !position.is_finite()
            || !size.is_finite()
            || size.min_element() <= 0.0
            || position.cmplt(Vec2::ZERO).any()
            || position.cmpgt(size).any()
        {
            self.reset();
            return Vec2::ZERO;
        }
        if self.metrics != Some((size, scale)) {
            self.previous = None;
            self.metrics = Some((size, scale));
            self.pending_center = None;
        }
        if let Some(center) = self.pending_center {
            // Windows/RDP may coalesce the synthetic center event with the
            // next real motion. Rebase that first central sample as well.
            if ((position - center).abs() / size).max_element() < 0.20 {
                self.pending_center = None;
                self.previous = Some(position);
                return Vec2::ZERO;
            }
        }
        let previous = self.previous.replace(position);
        let Some(previous) = previous else {
            return Vec2::ZERO;
        };
        let delta = position - previous;
        // Rebase implausible teleports (remote reconnect / cursor reposition),
        // rather than clamping them into an unwanted camera rotation.
        if delta.abs().max_element() > size.min_element() * 0.75 {
            Vec2::ZERO
        } else {
            delta
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_mouse_filters_warp_and_coalesced_echo_without_losing_queued_motion() {
        let size = Vec2::new(1000.0, 800.0);
        for echo in [size * 0.5, size * 0.5 + Vec2::new(3.0, -2.0)] {
            let mut p = AbsolutePointer::default();
            p.sample(Vec2::new(920.0, 400.0), size, 1.0);
            assert_eq!(p.recenter(), Some(size * 0.5));
            assert_eq!(p.recenter(), None);
            assert_eq!(p.sample(Vec2::new(925.0, 400.0), size, 1.0), Vec2::new(5.0, 0.0));
            assert_eq!(p.sample(echo, size, 1.0), Vec2::ZERO);
            assert_eq!(p.sample(echo + Vec2::X * 4.0, size, 1.0), Vec2::X * 4.0);
            assert_eq!(p.recenter(), None);
        }
    }
    #[test]
    fn remote_mouse_uses_differences_not_absolute_coordinates() {
        let mut p = AbsolutePointer::default();
        let size = Vec2::new(1920.0, 1080.0);
        assert_eq!(p.sample(Vec2::new(1200.0, 700.0), size, 1.0), Vec2::ZERO);
        assert_eq!(
            p.sample(Vec2::new(1210.0, 697.0), size, 1.0),
            Vec2::new(10.0, -3.0)
        );
        assert_eq!(p.sample(Vec2::new(1210.0, 697.0), size, 1.0), Vec2::ZERO);
        p.reset();
        assert_eq!(p.sample(Vec2::new(20.0, 20.0), size, 1.0), Vec2::ZERO);
    }
    #[test]
    fn remote_mouse_rebases_resizes_dpi_changes_and_teleports() {
        let mut p = AbsolutePointer::default();
        let size = Vec2::new(1920.0, 1080.0);
        p.sample(Vec2::new(30.0, 30.0), size, 1.0);
        assert_eq!(p.sample(Vec2::new(1800.0, 900.0), size, 1.0), Vec2::ZERO);
        assert_eq!(
            p.sample(Vec2::new(1802.0, 900.0), size, 1.0),
            Vec2::new(2.0, 0.0)
        );
        assert_eq!(
            p.sample(Vec2::new(900.0, 450.0), size / 2.0, 2.0),
            Vec2::ZERO
        );
        assert_eq!(
            p.sample(Vec2::new(902.0, 450.0), size / 2.0, 1.0),
            Vec2::ZERO
        );
        assert_eq!(p.sample(Vec2::NAN, size, 1.0), Vec2::ZERO);
    }
}
