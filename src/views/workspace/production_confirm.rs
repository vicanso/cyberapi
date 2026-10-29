//! D6: asked before a request that may change something goes in an
//! environment marked as production — once, or not again in this project
//! until the app quits.

use crate::views::name_dialog::ok_button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};
use rust_i18n::t;

/// Asks whether `what` (the method and the address) goes in `environment`;
/// `send` is told, when it does, whether to stop asking.
pub fn confirm_production(
    environment: String,
    what: String,
    window: &mut Window,
    cx: &mut App,
    send: impl Fn(bool, &mut Window, &mut App) + 'static,
) {
    let view = cx.new(|_| Asking {
        environment,
        what,
        stop: false,
    });
    let send = std::rc::Rc::new(send);
    window.open_dialog(cx, move |dialog, _, _| {
        let go = {
            let (view, send) = (view.clone(), send.clone());
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let stop = view.read(cx).stop;
                send(stop, window, cx);
                true
            }
        };
        dialog
            .title(t!("production.title").to_string())
            .w(px(520.))
            .child(crate::views::dialog_body(view.clone()))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label(t!("common.cancel").to_string())))
                    .child(ok_button("production-send", t!("production.send").to_string(), go)),
            )
            // Enter does not send it: the reader is to mean it.
            .on_ok(|_, _, _| false)
    });
}

struct Asking {
    environment: String,
    what: String,
    stop: bool,
}

impl Render for Asking {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .child(t!("production.ask", environment = self.environment).to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().danger)
                    .child(self.what.clone()),
            )
            .child(
                Checkbox::new("production-stop")
                    .checked(self.stop)
                    .label(t!("production.stop_asking").to_string())
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.stop = *checked;
                        cx.notify();
                    })),
            )
    }
}
