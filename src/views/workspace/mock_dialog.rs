//! G2: the project's mock server: a port, Start / Stop, where it listens,
//! how many APIs answer with an example, and the requests it answered,
//! newest first. Closing the dialog leaves it running; it stops with the
//! project.

use crate::states::{MockState, WorkspaceState};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use rust_i18n::t;

pub fn open_mock_dialog(workspace: Entity<WorkspaceState>, window: &mut Window, cx: &mut App) {
    let mock = workspace.update(cx, |workspace, cx| workspace.mock(cx));
    let view = cx.new(|cx| MockView::new(mock, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("mock.title").to_string())
            .w(px(620.))
            .on_ok(|_, _, _| false)
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("mock-close")
                        .label(t!("common.close").to_string())
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct MockView {
    mock: Entity<MockState>,
    port: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl MockView {
    fn new(mock: Entity<MockState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = mock.read(cx).port();
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::states::MOCK_PORT.to_string())
                .default_value(current.to_string())
        });
        let subscriptions = vec![
            cx.observe(&mock, |_, _, cx| cx.notify()),
            cx.subscribe(&port, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let typed = input.read(cx).value().trim().parse::<u16>().ok();
                    if let Some(port) = typed {
                        this.mock.update(cx, |mock, cx| mock.set_port(port, cx));
                    }
                }
            }),
        ];
        Self {
            mock,
            port,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for MockView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mock = self.mock.read(cx);
        let server_url = mock.server_url(cx);
        let (address, starting, error) = (mock.address(), mock.starting(), mock.error().map(str::to_string));
        let (apis, answering) = mock.count();
        let heard = mock.heard().to_vec();
        let serves_local = crate::this_build::serves_mocks();
        let theme = cx.theme();
        let (muted, border, danger) = (theme.muted_foreground, theme.border, theme.danger);
        let control: AnyElement = match &address {
            Some(address) => {
                let copied = address.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .child(address.clone()),
                    )
                    .child(
                        Button::new("mock-copy")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip(t!("mock.copy").to_string())
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("mock-stop")
                            .danger()
                            .small()
                            .label(t!("mock.stop").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.mock.update(cx, |mock, cx| mock.stop(cx)))),
                    )
                    .into_any_element()
            }
            None => h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().child(t!("mock.port").to_string()))
                .child(div().w(px(90.)).child(Input::new(&self.port).small()))
                .child(
                    Button::new("mock-start")
                        .primary()
                        .small()
                        .label(t!("mock.start").to_string())
                        .loading(starting)
                        .on_click(cx.listener(|this, _, _, cx| this.mock.update(cx, |mock, cx| mock.start(cx)))),
                )
                .into_any_element(),
        };
        let rows: Vec<AnyElement> = heard
            .iter()
            .enumerate()
            .map(|(index, heard)| {
                let when = chrono::DateTime::from_timestamp_millis(heard.at)
                    .map(|at| at.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                    .unwrap_or_default();
                let status_color = if heard.status < 400 {
                    theme.success
                } else {
                    theme.warning
                };
                h_flex()
                    .id(("mock-heard", index))
                    .gap_3()
                    .py_1()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .child(div().w(px(56.)).flex_none().text_color(muted).child(when))
                    .child(div().w(px(56.)).flex_none().child(heard.method.clone()))
                    .child(div().flex_1().min_w_0().truncate().child(heard.path.clone()))
                    .child(
                        div()
                            .w(px(36.))
                            .flex_none()
                            .text_color(status_color)
                            .child(heard.status.to_string()),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_none()
                            .truncate()
                            .text_color(muted)
                            .child(heard.api.clone().unwrap_or_default()),
                    )
                    .into_any_element()
            })
            .collect();
        let mono = theme.mono_font_family.clone();
        v_flex()
            .gap_3()
            .when_some(server_url.clone(), |this, url| {
                let copied = url.clone();
                this.child(
                    v_flex()
                        .gap_1p5()
                        .p_2p5()
                        .border_1()
                        .border_color(border)
                        .rounded(theme.radius)
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("mock.server_url").to_string()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_family(mono.clone())
                                        .child(url),
                                )
                                .child(
                                    Button::new("mock-copy-server")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Copy)
                                        .tooltip(t!("mock.copy").to_string())
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("mock.server_hint").to_string()),
                        ),
                )
            })
            .when(serves_local, |this| {
                this.child(div().text_sm().text_color(muted).child(t!("mock.about").to_string()))
                    .child(control)
                    .when_some(error, |this, error| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(danger)
                                .child(t!("mock.failed", why = error).to_string()),
                        )
                    })
            })
            .when(address.is_some() || server_url.is_some(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("mock.answering", apis = apis, answering = answering).to_string()),
                )
            })
            .when(address.is_some(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(t!("mock.heard").to_string()))
                        .child(match rows.is_empty() {
                            true => div()
                                .py_3()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("mock.none_heard").to_string())
                                .into_any_element(),
                            false => div()
                                .id("mock-log")
                                .max_h(px(260.))
                                .overflow_y_scroll()
                                .child(v_flex().children(rows))
                                .into_any_element(),
                        }),
                )
            })
    }
}
