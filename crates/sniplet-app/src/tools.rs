use sniplet_core::{AnnotationKind, AnnotationStyle, Color, ImageRect, Point};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Arrow,
    Rectangle,
    Text,
    Hide,
    Counter,
    Highlight,
    Freehand,
    Ellipse,
    Line,
    Spotlight,
    Zoom,
    Ruler,
}

impl Tool {
    /// Every tool, in dock order: the most used first.
    pub const ALL: [Self; 13] = [
        Self::Select,
        Self::Arrow,
        Self::Rectangle,
        Self::Text,
        Self::Hide,
        Self::Counter,
        Self::Highlight,
        Self::Freehand,
        Self::Ellipse,
        Self::Line,
        Self::Spotlight,
        Self::Zoom,
        Self::Ruler,
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
            Self::Hide => "Hide",
            Self::Zoom => "Magnifier",
        }
    }

    /// Tool keys. Contextual commands such as Z (quick zoom) live in the editor.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Select => Some("V"),
            Self::Arrow => Some("A"),
            Self::Text => Some("T"),
            Self::Rectangle => Some("R"),
            Self::Ellipse => Some("O"),
            Self::Line => Some("L"),
            Self::Highlight => Some("H"),
            Self::Spotlight => Some("S"),
            Self::Counter => Some("C"),
            Self::Hide => Some("B"),
            Self::Ruler | Self::Freehand | Self::Zoom => None,
        }
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
            Self::Hide => "eye-off",
            Self::Zoom => "zoom-in",
        }
    }

    /// The tool that draws `kind`. Kinds that no tool draws map to Select.
    pub fn of_kind(kind: &AnnotationKind) -> Self {
        match kind {
            AnnotationKind::Arrow { .. } => Self::Arrow,
            AnnotationKind::Rectangle { .. } => Self::Rectangle,
            AnnotationKind::Ellipse { .. } => Self::Ellipse,
            AnnotationKind::Line { .. } => Self::Line,
            AnnotationKind::Freehand { .. } => Self::Freehand,
            AnnotationKind::Text { .. } => Self::Text,
            AnnotationKind::Counter { .. } => Self::Counter,
            AnnotationKind::Highlight { .. } => Self::Highlight,
            AnnotationKind::Spotlight { .. } => Self::Spotlight,
            AnnotationKind::Magnifier { .. } => Self::Zoom,
            kind if HideStyle::of(kind).is_some() => Self::Hide,
            _ => Self::Select,
        }
    }

    /// The annotation a drag from `start` to `end` draws, before sizes are applied.
    #[allow(clippy::too_many_arguments)]
    pub fn annotation(
        self,
        start: Point,
        mut end: Point,
        points: &[Point],
        text: &str,
        counter: u32,
        hide: HideStyle,
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
            Self::Ruler | Self::Select => return None,
            Self::Line => AnnotationKind::Line { start, end },
            Self::Rectangle => AnnotationKind::Rectangle { rect },
            Self::Ellipse => AnnotationKind::Ellipse { rect },
            Self::Text => AnnotationKind::Text {
                origin: start,
                text: text.to_owned(),
                font_size: Size::Text.default_value(),
            },
            Self::Counter => AnnotationKind::Counter {
                center: start,
                value: counter,
                font_size: Size::Counter.default_value(),
            },
            Self::Freehand => AnnotationKind::Freehand {
                points: points.to_vec(),
            },
            Self::Highlight => AnnotationKind::Highlight { rect },
            Self::Spotlight => AnnotationKind::Spotlight { rect },
            Self::Hide => hide.kind(rect),
            Self::Zoom => {
                let radius = rect.width.max(rect.height) * 0.5;
                let source = Point::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
                let lens = Point::new(source.x + radius * 4.25, source.y + radius * 2.75);
                AnnotationKind::Magnifier {
                    rect: ImageRect::new(
                        lens.x - radius * 3.0,
                        lens.y - radius * 3.0,
                        radius * 6.0,
                        radius * 6.0,
                    ),
                    zoom: Size::Zoom.default_value(),
                    source: Some(source),
                }
            }
        })
    }
}

/// How the Hide tool covers an area. Each style is its own annotation kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HideStyle {
    #[default]
    Blur,
    Pixelate,
    Erase,
    Redact,
}

impl HideStyle {
    pub const ALL: [Self; 4] = [Self::Blur, Self::Pixelate, Self::Erase, Self::Redact];

    pub fn label(self) -> &'static str {
        match self {
            Self::Blur => "Blur",
            Self::Pixelate => "Pixelate",
            Self::Erase => "Erase",
            Self::Redact => "Redact",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Blur => "droplets",
            Self::Pixelate => "grid-2x2",
            Self::Erase => "eraser",
            Self::Redact => "square-dashed-bottom",
        }
    }

    /// The next style, for repeated presses of B.
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// The style and area of a hiding annotation.
    pub fn of(kind: &AnnotationKind) -> Option<(Self, ImageRect)> {
        match kind {
            AnnotationKind::Blur { rect, .. } => Some((Self::Blur, *rect)),
            AnnotationKind::Pixelate { rect, .. } => Some((Self::Pixelate, *rect)),
            AnnotationKind::RemoveFill { rect, .. } => Some((Self::Erase, *rect)),
            AnnotationKind::Redaction { rect } => Some((Self::Redact, *rect)),
            _ => None,
        }
    }

    pub fn kind(self, rect: ImageRect) -> AnnotationKind {
        match self {
            Self::Blur => AnnotationKind::Blur {
                rect,
                radius: Size::Blur.default_value(),
            },
            Self::Pixelate => AnnotationKind::Pixelate {
                rect,
                block_size: Size::Pixelate.default_value() as u32,
            },
            Self::Erase => AnnotationKind::RemoveFill { rect, sample: None },
            Self::Redact => AnnotationKind::Redaction { rect },
        }
    }
}

/// The one value that the shared size slider edits for a tool or annotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Stroke,
    Arrow,
    Text,
    Counter,
    Blur,
    Pixelate,
    Dim,
    Zoom,
}

impl Size {
    const COUNT: usize = 8;

    pub fn for_tool(tool: Tool, hide: HideStyle) -> Option<Self> {
        match tool {
            Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Freehand => Some(Self::Stroke),
            Tool::Arrow => Some(Self::Arrow),
            Tool::Text => Some(Self::Text),
            Tool::Counter => Some(Self::Counter),
            Tool::Spotlight => Some(Self::Dim),
            Tool::Zoom => Some(Self::Zoom),
            Tool::Hide => match hide {
                HideStyle::Blur => Some(Self::Blur),
                HideStyle::Pixelate => Some(Self::Pixelate),
                HideStyle::Erase | HideStyle::Redact => None,
            },
            Tool::Select | Tool::Highlight | Tool::Ruler => None,
        }
    }

    pub fn for_kind(kind: &AnnotationKind) -> Option<Self> {
        match kind {
            AnnotationKind::Rectangle { .. }
            | AnnotationKind::Ellipse { .. }
            | AnnotationKind::Line { .. }
            | AnnotationKind::Freehand { .. } => Some(Self::Stroke),
            AnnotationKind::Arrow { .. } => Some(Self::Arrow),
            AnnotationKind::Text { .. } => Some(Self::Text),
            AnnotationKind::Counter { .. } => Some(Self::Counter),
            AnnotationKind::Blur { .. } => Some(Self::Blur),
            AnnotationKind::Pixelate { .. } => Some(Self::Pixelate),
            AnnotationKind::Spotlight { .. } => Some(Self::Dim),
            AnnotationKind::Magnifier { .. } => Some(Self::Zoom),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Blur | Self::Pixelate => "Strength",
            Self::Dim => "Dim",
            Self::Zoom => "Zoom",
            _ => "Size",
        }
    }

    /// Minimum, maximum, and step.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::Stroke => (1.0, 20.0, 1.0),
            Self::Arrow => (1.0, 30.0, 0.5),
            Self::Text => (8.0, 120.0, 1.0),
            Self::Counter => (12.0, 96.0, 1.0),
            Self::Blur => (1.0, 40.0, 1.0),
            Self::Pixelate => (2.0, 48.0, 1.0),
            Self::Dim => (1.0, 9.0, 1.0),
            Self::Zoom => (1.0, 16.0, 0.5),
        }
    }

    pub fn default_value(self) -> f32 {
        match self {
            Self::Stroke => AnnotationStyle::default().stroke_width,
            Self::Arrow => 10.0,
            Self::Text => 24.0,
            Self::Counter => 28.0,
            Self::Blur => 8.0,
            Self::Pixelate => 12.0,
            Self::Dim => 6.0,
            Self::Zoom => 3.0,
        }
    }

    /// `{}` drops trailing zeros: 10 → "10", 10.5 → "10.5".
    pub fn format(self, value: f32) -> String {
        match self {
            Self::Text => format!("{value} pt"),
            Self::Zoom => format!("{value}×"),
            _ => format!("{value}"),
        }
    }

    /// The slider position, from 0 to 1, of `value`.
    pub fn fraction(self, value: f32) -> f32 {
        let (min, max, _) = self.range();
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }

    /// The value at slider position `fraction`, snapped to the step.
    pub fn value_at(self, fraction: f32) -> f32 {
        let (min, max, step) = self.range();
        let value = min + fraction.clamp(0.0, 1.0) * (max - min);
        ((value / step).round() * step).clamp(min, max)
    }

    pub fn get(self, kind: &AnnotationKind, style: &AnnotationStyle) -> f32 {
        match (self, kind) {
            (Self::Stroke | Self::Arrow, _) => style.stroke_width,
            (Self::Text, AnnotationKind::Text { font_size, .. })
            | (Self::Counter, AnnotationKind::Counter { font_size, .. }) => *font_size,
            (Self::Blur, AnnotationKind::Blur { radius, .. }) => *radius,
            (Self::Pixelate, AnnotationKind::Pixelate { block_size, .. }) => *block_size as f32,
            (Self::Dim, _) => style
                .fill
                .map_or(self.default_value(), |fill| (fill.a as f32 / 25.0).round())
                .clamp(1.0, 9.0),
            (Self::Zoom, AnnotationKind::Magnifier { zoom, .. }) => *zoom,
            _ => self.default_value(),
        }
    }

    pub fn set(self, kind: &mut AnnotationKind, style: &mut AnnotationStyle, value: f32) {
        match (self, kind) {
            (Self::Stroke | Self::Arrow, _) => style.stroke_width = value,
            (Self::Text, AnnotationKind::Text { font_size, .. })
            | (Self::Counter, AnnotationKind::Counter { font_size, .. }) => *font_size = value,
            (Self::Blur, AnnotationKind::Blur { radius, .. }) => *radius = value,
            (Self::Pixelate, AnnotationKind::Pixelate { block_size, .. }) => {
                *block_size = value as u32
            }
            (Self::Dim, _) => style.fill = Some(Color::new(0, 0, 0, (value * 25.0) as u8)),
            (Self::Zoom, AnnotationKind::Magnifier { zoom, .. }) => *zoom = value,
            _ => {}
        }
    }
}

/// The size each new annotation starts with, one value per `Size`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes([f32; Size::COUNT]);

impl Default for Sizes {
    fn default() -> Self {
        let mut values = [0.0; Size::COUNT];
        for size in [
            Size::Stroke,
            Size::Arrow,
            Size::Text,
            Size::Counter,
            Size::Blur,
            Size::Pixelate,
            Size::Dim,
            Size::Zoom,
        ] {
            values[size as usize] = size.default_value();
        }
        Self(values)
    }
}

impl Sizes {
    pub fn get(&self, size: Size) -> f32 {
        self.0[size as usize]
    }

    pub fn set(&mut self, size: Size, value: f32) {
        self.0[size as usize] = value;
    }

    /// Applies the current size to a new annotation.
    pub fn apply(&self, kind: &mut AnnotationKind, style: &mut AnnotationStyle) {
        if let Some(size) = Size::for_kind(kind) {
            size.set(kind, style, self.get(size));
        }
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
        assert_eq!(Tool::from_key("b"), Some(Tool::Hide));
        assert_eq!(Tool::from_key("h"), Some(Tool::Highlight));
        assert_eq!(Tool::from_key("s"), Some(Tool::Spotlight));
        assert_eq!(Tool::from_key("c"), Some(Tool::Counter));
        assert_eq!(Tool::from_key("o"), Some(Tool::Ellipse));
        assert_eq!(Tool::from_key("l"), Some(Tool::Line));
        assert_eq!(Tool::from_key("t"), Some(Tool::Text));
        assert_eq!(Tool::from_key("r"), Some(Tool::Rectangle));

        assert_eq!(Tool::from_key("a"), Some(Tool::Arrow));
        assert_eq!(Tool::from_key("A"), Some(Tool::Arrow));
        assert_eq!(Tool::from_key("z"), None);
        assert_eq!(Tool::Freehand.shortcut(), None);
    }

    #[test]
    fn shift_snaps_arrows_to_diagonals_and_rectangles_to_squares() {
        let start = Point::new(10.0, 10.0);
        let end = Point::new(70.0, 55.0);
        let hide = HideStyle::Blur;
        let AnnotationKind::Arrow { start, end, .. } = Tool::Arrow
            .annotation(start, end, &[], "", 1, hide, true)
            .unwrap()
        else {
            panic!()
        };
        assert!(((end.x - start.x).abs() - (end.y - start.y).abs()).abs() < 0.01);
        let AnnotationKind::Rectangle { rect } = Tool::Rectangle
            .annotation(start, end, &[], "", 1, hide, true)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(rect.width, rect.height);
    }

    #[test]
    fn sizes_round_trip_through_the_slider_and_annotations() {
        assert_eq!(Size::Arrow.value_at(Size::Arrow.fraction(14.0)), 14.0);
        assert_eq!(Size::Arrow.value_at(0.0), 1.0);
        assert_eq!(Size::Arrow.value_at(1.0), 30.0);
        assert_eq!(Size::Text.format(24.0), "24 pt");

        let mut kind = HideStyle::Pixelate.kind(ImageRect::new(0.0, 0.0, 10.0, 10.0));
        let mut style = AnnotationStyle::default();
        Size::Pixelate.set(&mut kind, &mut style, 20.0);
        assert_eq!(Size::Pixelate.get(&kind, &style), 20.0);
        Size::Dim.set(&mut kind, &mut style, 2.0);
        assert_eq!(style.fill, Some(Color::new(0, 0, 0, 50)));
        assert_eq!(Size::Dim.get(&kind, &style), 2.0);
    }
}
