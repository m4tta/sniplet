use serde::{Deserialize, Serialize};

use crate::Point;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewportTransform {
    zoom: f32,
    pan: Point,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self::new(1.0, Point::default())
    }
}

impl ViewportTransform {
    pub fn new(zoom: f32, pan: Point) -> Self {
        Self {
            zoom: valid_zoom(zoom),
            pan,
        }
    }

    pub fn zoom(self) -> f32 {
        self.zoom
    }

    pub fn pan(self) -> Point {
        self.pan
    }

    pub fn set_pan(&mut self, pan: Point) {
        self.pan = pan;
    }

    pub fn image_to_screen(self, point: Point) -> Point {
        Point::new(
            point.x * self.zoom + self.pan.x,
            point.y * self.zoom + self.pan.y,
        )
    }

    pub fn screen_to_image(self, point: Point) -> Point {
        Point::new(
            (point.x - self.pan.x) / self.zoom,
            (point.y - self.pan.y) / self.zoom,
        )
    }

    /// Sets the zoom while keeping the image pixel under `cursor` stationary.
    pub fn set_zoom_around(&mut self, zoom: f32, cursor: Point) {
        let anchor = self.screen_to_image(cursor);
        self.zoom = valid_zoom(zoom);
        self.pan = Point::new(
            cursor.x - anchor.x * self.zoom,
            cursor.y - anchor.y * self.zoom,
        );
    }

    pub fn zoom_by(&mut self, factor: f32, cursor: Point) {
        self.set_zoom_around(self.zoom * factor, cursor);
    }
}

fn valid_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(0.01, 128.0)
    } else {
        1.0
    }
}
