use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use enigo::{Axis, Coordinate, Enigo, Mouse, Settings};
use image::RgbaImage;

use crate::{
    CaptureSource, PlatformError, Result, capture_active_window, capture_monitor_region,
    capture_window, list_monitors,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollCaptureTarget {
    MonitorRegion {
        monitor_index: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Window {
        id: u32,
    },
}

/// Collects captures of one stable screen region and delegates overlap matching
/// to `sniplet_core`. The UI remains in control of scrolling so the session does
/// not synthesize input or unexpectedly move another application.
#[derive(Debug)]
pub struct ScrollSession {
    target: ScrollCaptureTarget,
    frames: Vec<RgbaImage>,
    options: sniplet_core::StitchOptions,
}

impl ScrollSession {
    pub fn for_monitor_region(
        monitor_index: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        options: sniplet_core::StitchOptions,
    ) -> Result<Self> {
        let target = ScrollCaptureTarget::MonitorRegion {
            monitor_index,
            x,
            y,
            width,
            height,
        };
        let first = capture_target(target)?;
        Ok(Self {
            target,
            frames: vec![first],
            options,
        })
    }

    pub fn for_window(id: u32, options: sniplet_core::StitchOptions) -> Result<Self> {
        let target = ScrollCaptureTarget::Window { id };
        let first = capture_target(target)?;
        Ok(Self {
            target,
            frames: vec![first],
            options,
        })
    }

    pub fn for_active_window(options: sniplet_core::StitchOptions) -> Result<Self> {
        let first = capture_active_window()?;
        let CaptureSource::Window { id } = first.source else {
            unreachable!("active-window capture always has a window source")
        };
        Ok(Self {
            target: ScrollCaptureTarget::Window { id },
            frames: vec![first.image],
            options,
        })
    }

    pub fn target(&self) -> ScrollCaptureTarget {
        self.target
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn capture_next(&mut self) -> Result<&RgbaImage> {
        self.frames.push(capture_target(self.target)?);
        Ok(self.frames.last().expect("a frame was just inserted"))
    }

    /// Adds a frame captured by a caller-controlled workflow, such as a delayed
    /// timer after the user scrolls. Width consistency is checked by `finish`.
    pub fn push_frame(&mut self, frame: RgbaImage) {
        self.frames.push(frame);
    }

    pub fn frames(&self) -> &[RgbaImage] {
        &self.frames
    }

    pub fn preview(&self) -> Result<RgbaImage> {
        sniplet_core::stitch_vertical(&self.frames, self.options)
            .map_err(crate::PlatformError::Stitch)
    }

    pub fn finish(self) -> Result<RgbaImage> {
        sniplet_core::stitch_vertical(&self.frames, self.options)
            .map_err(crate::PlatformError::Stitch)
    }
}

fn capture_target(target: ScrollCaptureTarget) -> Result<RgbaImage> {
    let frame = match target {
        ScrollCaptureTarget::MonitorRegion {
            monitor_index,
            x,
            y,
            width,
            height,
        } => capture_monitor_region(monitor_index, x, y, width, height)?,
        ScrollCaptureTarget::Window { id } => capture_window(id)?,
    };
    Ok(frame.image)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticScrollStop {
    EndReached,
    Cancelled,
    FrameLimit,
}

#[derive(Debug, Clone)]
pub struct AutomaticScrollOptions {
    /// Number of wheel notches sent after each frame. Positive values scroll down.
    pub scroll_clicks: i32,
    /// Time allowed for the target application to repaint after a scroll.
    pub settle_delay: Duration,
    /// Hard upper bound including the initial frame.
    pub max_frames: usize,
    /// Identical captures required to decide that the bottom has been reached.
    pub identical_frames_to_stop: usize,
    pub stitch: sniplet_core::StitchOptions,
}

impl Default for AutomaticScrollOptions {
    fn default() -> Self {
        Self {
            scroll_clicks: 5,
            settle_delay: Duration::from_millis(250),
            max_frames: 40,
            identical_frames_to_stop: 1,
            stitch: sniplet_core::StitchOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScrollCancellation(Arc<AtomicBool>);

impl ScrollCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub struct AutomaticScrollCapture {
    pub image: RgbaImage,
    pub frame_count: usize,
    pub stop: AutomaticScrollStop,
}

/// Captures and stitches a vertically scrolling monitor region.
///
/// The region is expressed in capture-image pixels. The cursor is temporarily
/// placed at its center so wheel events reach the selected content, then restored
/// before this function returns. Call this only after hiding the selection overlay.
pub fn capture_scrolling_region(
    monitor_index: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    options: AutomaticScrollOptions,
    cancellation: &ScrollCancellation,
) -> Result<AutomaticScrollCapture> {
    validate_automatic_options(&options)?;
    let monitors = list_monitors()?;
    let available = monitors.len();
    let monitor = monitors
        .into_iter()
        .find(|monitor| monitor.index == monitor_index)
        .ok_or(PlatformError::MonitorNotFound {
            index: monitor_index,
            available,
        })?;

    // xcap's macOS capture rectangle is in points while its returned bitmap is
    // in Retina pixels. Enigo consumes desktop coordinates, so convert only the
    // region offset and keep the monitor's desktop-space origin.
    let center_x = crate::capture::desktop_units(x.saturating_add(width / 2), monitor.scale_factor);
    let center_y =
        crate::capture::desktop_units(y.saturating_add(height / 2), monitor.scale_factor);
    let cursor_x = monitor
        .x
        .saturating_add(i32::try_from(center_x).unwrap_or(i32::MAX));
    let cursor_y = monitor
        .y
        .saturating_add(i32::try_from(center_y).unwrap_or(i32::MAX));

    let mut enigo = Enigo::new(&Settings::default()).map_err(PlatformError::InputInitialization)?;
    let original_cursor = enigo.location().map_err(PlatformError::Input)?;
    enigo
        .move_mouse(cursor_x, cursor_y, Coordinate::Abs)
        .map_err(PlatformError::Input)?;
    wait_for_settle(cancellation, options.settle_delay);

    let collected = collect_automatic_frames(
        &options,
        cancellation,
        || capture_monitor_region(monitor_index, x, y, width, height).map(|frame| frame.image),
        || {
            enigo
                .scroll(options.scroll_clicks, Axis::Vertical)
                .map_err(PlatformError::Input)?;
            Ok(wait_for_settle(cancellation, options.settle_delay))
        },
    );
    let restore = enigo
        .move_mouse(original_cursor.0, original_cursor.1, Coordinate::Abs)
        .map_err(PlatformError::Input);
    let collected = collected?;
    restore?;
    let frame_count = collected.frames.len();
    let image = sniplet_core::stitch_vertical(&collected.frames, options.stitch)
        .map_err(PlatformError::Stitch)?;
    Ok(AutomaticScrollCapture {
        image,
        frame_count,
        stop: collected.stop,
    })
}

fn validate_automatic_options(options: &AutomaticScrollOptions) -> Result<()> {
    if options.scroll_clicks <= 0 {
        return Err(PlatformError::InvalidScrollOptions(
            "scroll_clicks must be positive",
        ));
    }
    if options.max_frames == 0 {
        return Err(PlatformError::InvalidScrollOptions(
            "max_frames must be at least one",
        ));
    }
    if options.identical_frames_to_stop == 0 {
        return Err(PlatformError::InvalidScrollOptions(
            "identical_frames_to_stop must be at least one",
        ));
    }
    Ok(())
}

struct CollectedFrames {
    frames: Vec<RgbaImage>,
    stop: AutomaticScrollStop,
}

fn collect_automatic_frames<Capture, Advance>(
    options: &AutomaticScrollOptions,
    cancellation: &ScrollCancellation,
    mut capture: Capture,
    mut advance: Advance,
) -> Result<CollectedFrames>
where
    Capture: FnMut() -> Result<RgbaImage>,
    Advance: FnMut() -> Result<bool>,
{
    let mut frames = vec![capture()?];
    let mut identical_frames = 0;
    loop {
        if cancellation.is_cancelled() {
            return Ok(CollectedFrames {
                frames,
                stop: AutomaticScrollStop::Cancelled,
            });
        }
        if frames.len() >= options.max_frames {
            return Ok(CollectedFrames {
                frames,
                stop: AutomaticScrollStop::FrameLimit,
            });
        }
        if !advance()? || cancellation.is_cancelled() {
            return Ok(CollectedFrames {
                frames,
                stop: AutomaticScrollStop::Cancelled,
            });
        }

        let next = capture()?;
        if same_pixels(frames.last().expect("the initial frame is retained"), &next) {
            identical_frames += 1;
            if identical_frames >= options.identical_frames_to_stop {
                return Ok(CollectedFrames {
                    frames,
                    stop: AutomaticScrollStop::EndReached,
                });
            }
        } else {
            identical_frames = 0;
            frames.push(next);
        }
    }
}

fn same_pixels(left: &RgbaImage, right: &RgbaImage) -> bool {
    left.dimensions() == right.dimensions() && left.as_raw() == right.as_raw()
}

fn wait_for_settle(cancellation: &ScrollCancellation, duration: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < duration {
        if cancellation.is_cancelled() {
            return false;
        }
        std::thread::sleep(
            duration
                .saturating_sub(started.elapsed())
                .min(Duration::from_millis(20)),
        );
    }
    !cancellation.is_cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn manually_supplied_frames_are_stitched_by_core() {
        let first = RgbaImage::from_fn(2, 4, |_x, y| Rgba([y as u8, 0, 0, 255]));
        let second = RgbaImage::from_fn(2, 4, |_x, y| Rgba([(y + 2) as u8, 0, 0, 255]));
        let session = ScrollSession {
            target: ScrollCaptureTarget::Window { id: 1 },
            frames: vec![first, second],
            options: sniplet_core::StitchOptions {
                min_overlap: 2,
                max_overlap: Some(2),
                max_mean_error: 0.0,
            },
        };
        assert_eq!(session.finish().unwrap().dimensions(), (2, 6));
    }

    fn solid(value: u8) -> RgbaImage {
        RgbaImage::from_pixel(2, 2, Rgba([value, value, value, 255]))
    }

    fn test_options(max_frames: usize) -> AutomaticScrollOptions {
        AutomaticScrollOptions {
            max_frames,
            settle_delay: Duration::ZERO,
            ..AutomaticScrollOptions::default()
        }
    }

    #[test]
    fn automatic_collection_stops_at_an_identical_end_frame() {
        let mut captures = vec![solid(1), solid(1)].into_iter();
        let cancellation = ScrollCancellation::new();
        let result = collect_automatic_frames(
            &test_options(10),
            &cancellation,
            || Ok(captures.next().unwrap()),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(result.stop, AutomaticScrollStop::EndReached);
        assert_eq!(result.frames.len(), 1);
    }

    #[test]
    fn automatic_collection_obeys_the_frame_limit() {
        let mut value = 0;
        let cancellation = ScrollCancellation::new();
        let result = collect_automatic_frames(
            &test_options(3),
            &cancellation,
            || {
                value += 1;
                Ok(solid(value))
            },
            || Ok(true),
        )
        .unwrap();
        assert_eq!(result.stop, AutomaticScrollStop::FrameLimit);
        assert_eq!(result.frames.len(), 3);
    }

    #[test]
    fn automatic_collection_can_be_cancelled_without_scrolling() {
        let cancellation = ScrollCancellation::new();
        cancellation.cancel();
        let mut advances = 0;
        let result = collect_automatic_frames(
            &test_options(10),
            &cancellation,
            || Ok(solid(1)),
            || {
                advances += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(result.stop, AutomaticScrollStop::Cancelled);
        assert_eq!(result.frames.len(), 1);
        assert_eq!(advances, 0);
    }
}
