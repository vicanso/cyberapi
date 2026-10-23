//! A part of the page laid out in the rem it opened with, whatever the
//! interface's text size becomes meanwhile: the settings, so the control
//! that changes the size stays under the pointer while it is used.

use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Window,
};
use std::panic::Location;

pub struct FixedRem {
    rem: Pixels,
    child: AnyElement,
}

/// `child`, measured in `rem` rather than the window's.
pub fn fixed_rem(rem: Pixels, child: impl IntoElement) -> FixedRem {
    FixedRem {
        rem,
        child: child.into_any_element(),
    }
}

impl IntoElement for FixedRem {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FixedRem {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let child = &mut self.child;
        let layout = window.with_rem_size(Some(self.rem), |window| child.request_layout(window, cx));
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let child = &mut self.child;
        window.with_rem_size(Some(self.rem), |window| {
            child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        window.with_rem_size(Some(self.rem), |window| child.paint(window, cx));
    }
}
