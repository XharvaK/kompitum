//! The keyboard focus ring: a ring painted outside an element's bounds while
//! the element has focus GPUI considers visible (keyboard focus, not a click).
//!
//! It is a paint step rather than an overlay child. An overlay would have to be
//! a child of the element, and an element's own layout is the script's: adding
//! a box there changes what flex sizes, what `size_full` fills and where the
//! next sibling starts. Painting after the element has been painted reads the
//! bounds GPUI already resolved and writes only pixels outside them.
//!
//! The ring is drawn every frame from the live focus state, so a focus change
//! shows or clears it on the next frame without the script re-rendering: the
//! description a mod built is cached, and a ring captured in it would be the
//! ring from the frame the description was built.

use std::panic;

use gpui::{
    AnyElement, App, Bounds, BorderStyle, Corners, Edges, Element, ElementId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Window, px,
};

/// The ring's width and the gap between it and the control's own bounds.
const RING_WIDTH: f32 = 2.0;
const RING_GAP: f32 = 1.0;

/// Wraps `element` so that while `handle` has visible keyboard focus, a ring is
/// painted around the bounds GPUI gave the element.
pub(crate) fn with_focus_ring(
    element: AnyElement,
    handle: FocusHandle,
    color: Hsla,
    radius: Pixels,
    identity: ElementId,
) -> AnyElement {
    FocusRing {
        element,
        handle,
        color,
        radius,
        identity,
    }
    .into_any_element()
}

struct FocusRing {
    element: AnyElement,
    handle: FocusHandle,
    color: Hsla,
    radius: Pixels,
    identity: ElementId,
}

impl IntoElement for FocusRing {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FocusRing {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.element.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(window, cx);
        if !self.handle.is_focused(window) || !window.last_input_was_keyboard() {
            return;
        }
        let ring = bounds.inset(px(-(RING_WIDTH + RING_GAP)));
        let radius = self.radius + px(RING_WIDTH + RING_GAP);
        window.paint_quad(gpui::quad(
            ring,
            Corners::all(radius),
            gpui::transparent_black(),
            Edges::all(px(RING_WIDTH)),
            self.color,
            BorderStyle::Solid,
        ));
        test_support::note(&self.identity, ring, self.color, self.radius);
    }
}

pub mod test_support {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};

    use gpui::{Bounds, ElementId, Hsla, Pixels, point, px, size};

    #[derive(Clone, Debug)]
    pub struct PaintedRing {
        pub identity: String,
        pub bounds: Bounds<Pixels>,
        pub color: Hsla,
        pub radius: Pixels,
    }

    /// The file rings are reported to, when the host's render test asks.
    ///
    /// A file rather than a thread-local: a host can link two builds of this
    /// crate (one with its normal features, one for its tests), and a
    /// thread-local in one is invisible to the other.
    fn log_path() -> Option<std::path::PathBuf> {
        std::env::var_os("KOMPITUM_FOCUS_RING_LOG").map(std::path::PathBuf::from)
    }

    pub(crate) fn note(identity: &ElementId, bounds: Bounds<Pixels>, color: Hsla, radius: Pixels) {
        let Some(path) = log_path() else { return };
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
            return;
        };
        let x = f32::from(bounds.origin.x);
        let y = f32::from(bounds.origin.y);
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let _ = writeln!(
            file,
            "{identity:?}\t{x}\t{y}\t{width}\t{height}\t{}\t{}\t{}\t{}\t{}",
            color.h, color.s, color.l, color.a, f32::from(radius)
        );
        let _ = file.flush();
    }

    /// Forgets the rings painted before this call.
    pub fn reset() {
        if let Some(path) = log_path() {
            let _ = File::create(path);
        }
    }

    /// The rings painted since [`reset`].
    pub fn painted() -> Vec<PaintedRing> {
        let Some(path) = log_path() else { return Vec::new() };
        let Ok(mut file) = File::open(path) else { return Vec::new() };
        let mut text = String::new();
        if file.read_to_string(&mut text).is_err() {
            return Vec::new();
        }
        text.lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                if fields.len() != 10 {
                    return None;
                }
                let number = |index: usize| fields[index].parse::<f32>().ok();
                Some(PaintedRing {
                    identity: fields[0].to_owned(),
                    bounds: Bounds::new(
                        point(px(number(1)?), px(number(2)?)),
                        size(px(number(3)?), px(number(4)?)),
                    ),
                    color: Hsla {
                        h: number(5)?,
                        s: number(6)?,
                        l: number(7)?,
                        a: number(8)?,
                    },
                    radius: px(number(9)?),
                })
            })
            .collect()
    }
}
