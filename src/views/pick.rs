//! A small button that shows the current choice and drops a menu of all of
//! them — a role, a team, an account.

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{Anchor, App, ElementId, IntoElement, Window};
use std::rc::Rc;

pub fn pick<T: PartialEq + Clone + 'static>(
    id: impl Into<ElementId>,
    label: String,
    options: Vec<(T, String)>,
    chosen: Option<T>,
    disabled: bool,
    on_choose: impl Fn(T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_choose = Rc::new(on_choose);
    Button::new(id)
        .outline()
        .xsmall()
        .label(label)
        .disabled(disabled)
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
            options.iter().fold(menu, |menu, (value, text)| {
                let (value, on_choose) = (value.clone(), on_choose.clone());
                let checked = chosen.as_ref() == Some(&value);
                menu.item(
                    PopupMenuItem::new(text.clone())
                        .checked(checked)
                        .on_click(move |_, window, cx| on_choose(value.clone(), window, cx)),
                )
            })
        })
}
