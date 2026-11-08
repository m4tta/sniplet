//! What every capture overlay shares: rectangles, monitor geometry in desktop units, and the
//! borderless, always-on-top window that covers one monitor.
use std::sync::atomic::{AtomicU64, Ordering};

use rune::{AppSpec, Cx, El, KeyEvent, Tx, Waker, WindowHandle, WindowOptions, WindowSpec};
use sniplet_core::Point as ImagePoint;

use crate::ui::Inbox;

/// An axis-aligned rectangle, in points or desktop units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_corners(a: ImagePoint, b: ImagePoint) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The overlap, with a zero size when there is none.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Rect::new(
            x,
            y,
            (self.right().min(other.right()) - x).max(0.0),
            (self.bottom().min(other.bottom()) - y).max(0.0),
        )
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        !self.intersect(other).is_empty()
    }

    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

/// Subtracts axis-aligned cuts from a rectangle. Each cut partitions an intersecting piece
/// into at most four non-overlapping rectangles.
pub(crate) fn subtract_rectangles(source: Rect, cuts: &[Rect]) -> Vec<Rect> {
    cuts.iter().fold(vec![source], |pieces, cut| {
        pieces
            .into_iter()
            .flat_map(|piece| subtract_rectangle(piece, *cut))
            .collect()
    })
}

fn subtract_rectangle(source: Rect, cut: Rect) -> Vec<Rect> {
    let overlap = source.intersect(&cut);
    if overlap.is_empty() {
        return vec![source];
    }
    let piece = |left: f32, top: f32, right: f32, bottom: f32| {
        (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
    };
    [
        piece(source.x, source.y, source.right(), overlap.y),
        piece(source.x, overlap.bottom(), source.right(), source.bottom()),
        piece(source.x, overlap.y, overlap.x, overlap.bottom()),
        piece(overlap.right(), overlap.y, source.right(), overlap.bottom()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// One monitor in desktop units: physical pixels on Windows and Linux, Quartz points on
/// macOS. These are the units monitor origins and window frames use on each platform.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Monitor {
    pub id: u32,
    pub bounds: Rect,
    /// The monitor's size in image pixels.
    pub pixels: (u32, u32),
}

impl Monitor {
    pub fn from_frame(frame: &sniplet_platform::CapturedFrame) -> Self {
        let id = match frame.source {
            sniplet_platform::CaptureSource::Monitor { id, .. }
            | sniplet_platform::CaptureSource::MonitorRegion { id, .. }
            | sniplet_platform::CaptureSource::Window { id } => id,
        };
        Self::new(
            id,
            frame.origin,
            frame.image.dimensions(),
            frame.scale_factor,
        )
    }

    #[cfg(target_os = "macos")]
    pub fn from_info(info: &sniplet_platform::MonitorInfo) -> Self {
        Self::new(
            info.id,
            sniplet_platform::ScreenPoint {
                x: info.x,
                y: info.y,
            },
            (info.width, info.height),
            info.scale_factor,
        )
    }

    fn new(
        id: u32,
        origin: sniplet_platform::ScreenPoint,
        pixels: (u32, u32),
        scale_factor: f32,
    ) -> Self {
        let units = if cfg!(target_os = "macos") {
            scale_factor.max(1.0)
        } else {
            1.0
        };
        Self {
            id,
            bounds: Rect::new(
                origin.x as f32,
                origin.y as f32,
                pixels.0 as f32 / units,
                pixels.1 as f32 / units,
            ),
            pixels,
        }
    }

    /// Image pixels per desktop unit.
    pub fn scale(&self) -> f32 {
        self.pixels.0 as f32 / self.bounds.w.max(1.0)
    }

    /// A borderless, always-on-top window over the whole monitor. It takes input, and its
    /// background is transparent when `transparent` is set.
    pub fn window_options(&self, transparent: bool) -> WindowOptions {
        // Rune places windows in physical desktop pixels; macOS bounds are in points.
        let scale = if cfg!(target_os = "macos") {
            self.scale()
        } else {
            1.0
        };
        WindowOptions {
            transparent,
            borderless: true,
            click_through: false,
            always_on_top: true,
            visible: true,
            position: Some((
                (self.bounds.x * scale).round() as i32,
                (self.bounds.y * scale).round() as i32,
            )),
            physical_size: Some(self.pixels),
            min_size: Some((0.0, 0.0)),
            max_size: None,
            resizable: false,
        }
    }
}

/// A point in an overlay window, in points, as desktop units. The overlay covers `bounds`
/// with a `viewport` of that many points.
pub(crate) fn to_desktop(bounds: Rect, viewport: (f32, f32), x: f32, y: f32) -> ImagePoint {
    ImagePoint::new(
        bounds.x + x * bounds.w / viewport.0.max(1.0),
        bounds.y + y * bounds.h / viewport.1.max(1.0),
    )
}

/// A rectangle in desktop units as points in the overlay that covers `bounds`.
pub(crate) fn to_local(bounds: Rect, viewport: (f32, f32), rect: Rect) -> Rect {
    let sx = viewport.0 / bounds.w.max(1.0);
    let sy = viewport.1 / bounds.h.max(1.0);
    Rect::new(
        (rect.x - bounds.x) * sx,
        (rect.y - bounds.y) * sy,
        rect.w * sx,
        rect.h * sy,
    )
}

/// The viewport, in points, of an overlay over `monitor` before its first layout reports it.
pub(crate) fn initial_viewport(monitor: &Monitor, scale_factor: f32) -> (f32, f32) {
    let points = if cfg!(target_os = "macos") {
        1.0
    } else {
        scale_factor.max(1.0)
    };
    (monitor.bounds.w / points, monitor.bounds.h / points)
}

/// The state of a window Sniplet opens at runtime: its inbox carries results and messages.
pub(crate) trait WindowState: Sized + 'static {
    fn inbox(&self) -> &Inbox<Self>;
}

fn setup<S: WindowState>(state: &mut S, waker: Waker) {
    state.inbox().set_waker(waker);
}

fn poll<S: WindowState>(state: &mut S, _: f64, tx: &mut Tx) -> bool {
    Inbox::drain(state, S::inbox, tx)
}

/// How a window Sniplet opens at runtime looks and takes keys.
pub(crate) struct Window<S: 'static> {
    pub title: &'static str,
    /// Logical size, used unless the options set a physical one.
    pub size: (f32, f32),
    pub view: fn(&S, &mut Cx<S>) -> El,
    pub on_key: fn(&mut S, &KeyEvent, &mut Tx) -> bool,
    pub options: WindowOptions,
}

/// Open a window with its own state in the running app. Each gets a unique key.
pub(crate) fn open<S: WindowState>(
    window: Window<S>,
    state: S,
    on_ready: impl FnMut(&WindowHandle) + 'static,
) -> WindowHandle {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let key = format!("sniplet-window-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let spec = AppSpec {
        min_size: Some((0.0, 0.0)),
        setup: Some(setup::<S>),
        poll: Some(poll::<S>),
        on_key: Some(window.on_key),
        ..AppSpec::new(
            window.title,
            window.size,
            || unreachable!("the opener supplies the state"),
            window.view,
        )
    };
    rune::open_window(
        WindowSpec::new(key, spec)
            .state(state)
            // Translucent dims and labels blend as they did before Rune, in sRGB.
            .srgb_blend(true)
            .on_ready(on_ready)
            .options(window.options),
    )
}

/// Open an overlay over `monitor`. macOS places it on the monitor by its display ID, which
/// is exact across displays of different scales.
pub(crate) fn open_overlay<S: WindowState>(
    monitor: &Monitor,
    window: Window<S>,
    state: S,
) -> WindowHandle {
    let handle = open(window, state, |_handle: &WindowHandle| {
        #[cfg(target_os = "macos")]
        crate::macos::focus_capture_under_pointer(_handle);
    });
    #[cfg(target_os = "macos")]
    handle.place(
        rune::DisplayTarget::Native(rune::NativeDisplayId::MacOS(monitor.id)),
        rune::OverlayPlacement::FullScreen,
    );
    // Other platforms placed it through its options.
    let _ = monitor;
    handle
}

/// The translucent black outside a selection.
pub(crate) fn dim(alpha: u8) -> rune::Color {
    rune::Color::BLACK.alpha(f32::from(alpha) / 255.0)
}

/// The dark pill behind overlay hints and labels.
pub(crate) fn pill() -> rune::Color {
    crate::ui::rgb(0x202020).alpha(232.0 / 255.0)
}

/// The hint centered near the bottom of an overlay.
pub(crate) fn hint(label: &str) -> El {
    rune::row()
        .pin(Some(12.0), None, Some(12.0), Some(36.0))
        .justify_center()
        .child(
            rune::el().px(16.0).py(8.0).radius(8.0).fill(pill()).child(
                rune::text(label.to_owned())
                    .font(crate::ui::BODY)
                    .color(rune::Color::WHITE)
                    .single_line(),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect::new(x as f32, y as f32, width as f32, height as f32)
    }

    #[test]
    fn desktop_and_overlay_coordinates_round_trip_at_fractional_scale() {
        // A 3840 × 2160 monitor at 150 % shows a 2560 × 1440 point overlay.
        let bounds = Rect::new(-2880.0, 120.0, 3840.0, 2160.0);
        let viewport = (2560.0, 1440.0);
        assert_eq!(
            to_desktop(bounds, viewport, 100.0, 40.0),
            ImagePoint::new(-2730.0, 180.0)
        );
        assert_eq!(
            to_local(bounds, viewport, window(-2730, 180, 600, 300)),
            Rect::new(100.0, 40.0, 400.0, 200.0)
        );
    }

    #[test]
    fn point_desktops_do_not_scale_window_geometry() {
        let bounds = Rect::new(-1440.0, 180.0, 1440.0, 900.0);
        let viewport = (1440.0, 900.0);
        assert_eq!(
            to_desktop(bounds, viewport, 100.0, 40.0),
            ImagePoint::new(-1340.0, 220.0)
        );
        assert_eq!(
            to_local(bounds, viewport, window(-1340, 220, 400, 200)),
            Rect::new(100.0, 40.0, 400.0, 200.0)
        );
    }

    #[test]
    fn overlays_cover_the_monitor_in_physical_pixels() {
        let monitor = Monitor::from_frame(&sniplet_platform::CapturedFrame {
            image: image::RgbaImage::new(3840, 2160),
            origin: sniplet_platform::ScreenPoint { x: 3840, y: 0 },
            scale_factor: 1.5,
            source: sniplet_platform::CaptureSource::Monitor { index: 1, id: 9 },
        });
        let options = monitor.window_options(false);
        assert_eq!(options.physical_size, Some((3840, 2160)));
        if !cfg!(target_os = "macos") {
            assert_eq!(monitor.bounds, Rect::new(3840.0, 0.0, 3840.0, 2160.0));
            assert_eq!(options.position, Some((3840, 0)));
            assert_eq!(initial_viewport(&monitor, 1.5), (2560.0, 1440.0));
        }
    }

    #[test]
    fn rectangle_subtraction_splits_around_a_center_cut_without_overlap() {
        let source = Rect::new(0.0, 0.0, 100.0, 80.0);
        let cut = Rect::new(20.0, 10.0, 50.0, 40.0);
        let pieces = subtract_rectangles(source, &[cut]);

        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces.iter().map(Rect::area).sum::<f32>(), 6_000.0);
        for (index, left) in pieces.iter().enumerate() {
            assert_eq!(left.intersect(&cut).area(), 0.0);
            for right in pieces.iter().skip(index + 1) {
                assert_eq!(left.intersect(right).area(), 0.0);
            }
        }
    }

    #[test]
    fn rectangle_subtraction_handles_overlapping_cuts_and_full_coverage() {
        let source = Rect::new(0.0, 0.0, 100.0, 100.0);
        let cuts = [
            Rect::new(20.0, 0.0, 40.0, 100.0),
            Rect::new(40.0, 30.0, 60.0, 40.0),
        ];
        let pieces = subtract_rectangles(source, &cuts);

        assert_eq!(pieces.iter().map(Rect::area).sum::<f32>(), 4_400.0);
        assert!(subtract_rectangles(source, &[source]).is_empty());
    }
}
