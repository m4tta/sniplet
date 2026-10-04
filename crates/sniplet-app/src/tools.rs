use sniplet_core::{AnnotationKind, ImageRect, Point};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Arrow,
    Text,
    Ruler,
    Rectangle,
    Ellipse,
    Line,
    Freehand,
    Highlight,
    Spotlight,
    Counter,
    Pixelate,
    Blur,
    Redact,
    Erase,
    Zoom,
}

impl Tool {
    pub const ALL: [Self; 16] = [
        Self::Select,
        Self::Arrow,
        Self::Text,
        Self::Ruler,
        Self::Rectangle,
        Self::Ellipse,
        Self::Line,
        Self::Freehand,
        Self::Highlight,
        Self::Spotlight,
        Self::Counter,
        Self::Pixelate,
        Self::Blur,
        Self::Redact,
        Self::Erase,
        Self::Zoom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select / Crop",
            Self::Arrow => "Arrow",
            Self::Text => "Text",
            Self::Ruler => "Ruler",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Oval",
            Self::Line => "Line",
            Self::Freehand => "Freehand",
            Self::Highlight => "Highlighter",
            Self::Spotlight => "Spotlight",
            Self::Counter => "Counter",
            Self::Pixelate => "Pixelate",
            Self::Blur => "Blur",
            Self::Redact => "Redact",
            Self::Erase => "Erase",
            Self::Zoom => "Magnifier",
        }
    }

    /// Tool keys. Contextual commands such as Z (quick zoom) live in the editor.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Select => Some("V"),
            Self::Arrow => Some("A"),
            Self::Ellipse => Some("O"),
            Self::Line => Some("L"),
            Self::Highlight => Some("H"),
            Self::Spotlight => Some("S"),
            Self::Counter => Some("C"),
            Self::Blur => Some("B"),
            Self::Text
            | Self::Ruler
            | Self::Rectangle
            | Self::Freehand
            | Self::Pixelate
            | Self::Redact
            | Self::Erase
            | Self::Zoom => None,
        }
    }

    /// Temporary display adapter for the existing toolbar. New UI code should
    /// use `shortcut` so tools without a verified binding omit the suffix.
    pub fn key(self) -> &'static str {
        self.shortcut().unwrap_or("")
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| {
            tool.shortcut()
                .is_some_and(|shortcut| shortcut.eq_ignore_ascii_case(key))
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Select => "mouse-pointer-2",
            Self::Arrow => "arrow-up-right",
            Self::Text => "type",
            Self::Ruler => "ruler",
            Self::Rectangle => "square",
            Self::Ellipse => "circle",
            Self::Line => "slash",
            Self::Freehand => "pencil",
            Self::Highlight => "highlighter",
            Self::Spotlight => "focus",
            Self::Counter => "list-ordered",
            Self::Pixelate => "grid-2x2",
            Self::Blur => "droplets",
            Self::Redact => "square-dashed-bottom",
            Self::Erase => "eraser",
            Self::Zoom => "zoom-in",
        }
    }

    pub fn annotation(
        self,
        start: Point,
        mut end: Point,
        points: &[Point],
        text: &str,
        counter: u32,
        constrain: bool,
    ) -> Option<AnnotationKind> {
        if constrain {
            if matches!(self, Self::Line | Self::Arrow | Self::Ruler) {
                end = constrain_angle(start, end);
            } else {
                let edge = (end.x - start.x).abs().max((end.y - start.y).abs());
                end = Point::new(
                    start.x + edge * (end.x - start.x).signum(),
                    start.y + edge * (end.y - start.y).signum(),
                );
            }
        }
        let rect = ImageRect::from_corners(start, end);
        Some(match self {
            Self::Arrow => AnnotationKind::Arrow {
                start,
                end,
                bend: None,
                variant: Default::default(),
            },
            Self::Line | Self::Ruler => AnnotationKind::Line { start, end },
            Self::Rectangle => AnnotationKind::Rectangle { rect },
            Self::Ellipse => AnnotationKind::Ellipse { rect },
            Self::Text => AnnotationKind::Text {
                origin: start,
                text: text.to_owned(),
                font_size: 24.0,
            },
            Self::Counter => AnnotationKind::Counter {
                center: start,
                value: counter,
                font_size: 28.0,
            },
            Self::Freehand => AnnotationKind::Freehand {
                points: points.to_vec(),
            },
            Self::Highlight => AnnotationKind::Highlight { rect },
            Self::Spotlight => AnnotationKind::Spotlight { rect },
            Self::Pixelate => AnnotationKind::Pixelate {
                rect,
                block_size: 12,
            },
            Self::Blur => AnnotationKind::Blur { rect, radius: 8.0 },
            Self::Redact => AnnotationKind::Redaction { rect },
            Self::Erase => AnnotationKind::RemoveFill { rect, sample: None },
            Self::Zoom => AnnotationKind::Magnifier { rect, zoom: 2.0 },
            Self::Select => return None,
        })
    }
}

pub fn constrain_angle(start: Point, end: Point) -> Point {
    let distance = (end.x - start.x).hypot(end.y - start.y);
    let angle = (end.y - start.y).atan2(end.x - start.x);
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (angle / step).round() * step;
    Point::new(
        start.x + distance * angle.cos(),
        start.y + distance * angle.sin(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_verified_and_requested_tool_shortcuts() {
        assert_eq!(Tool::from_key("v"), Some(Tool::Select));
        assert_eq!(Tool::from_key("b"), Some(Tool::Blur));
        assert_eq!(Tool::from_key("h"), Some(Tool::Highlight));
        assert_eq!(Tool::from_key("s"), Some(Tool::Spotlight));
        assert_eq!(Tool::from_key("c"), Some(Tool::Counter));
        assert_eq!(Tool::from_key("o"), Some(Tool::Ellipse));
        assert_eq!(Tool::from_key("l"), Some(Tool::Line));

        assert_eq!(Tool::from_key("a"), Some(Tool::Arrow));
        assert_eq!(Tool::from_key("A"), Some(Tool::Arrow));
        assert_eq!(Tool::from_key("z"), None);
        assert_eq!(Tool::Rectangle.shortcut(), None);
        assert_eq!(Tool::Pixelate.shortcut(), None);
        assert_eq!(Tool::Erase.shortcut(), None);
    }

    #[test]
    fn shift_snaps_arrows_to_diagonals_and_rectangles_to_squares() {
        let start = Point::new(10.0, 10.0);
        let end = Point::new(70.0, 55.0);
        let AnnotationKind::Arrow { start, end, .. } = Tool::Arrow
            .annotation(start, end, &[], "", 1, true)
            .unwrap()
        else {
            panic!()
        };
        assert!(((end.x - start.x).abs() - (end.y - start.y).abs()).abs() < 0.01);
        let AnnotationKind::Rectangle { rect } = Tool::Rectangle
            .annotation(start, end, &[], "", 1, true)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(rect.width, rect.height);
    }
}
