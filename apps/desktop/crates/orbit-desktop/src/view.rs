//! The views: the sign-in screens and the signed-in window (sidebar, message list, thread panel,
//! composer, socket status). Flat panes split by 1 px borders, 48 px header rows, as the web app.

use std::rc::Rc;

use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::message_scroller::MessageScroller;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use orbit_client::{ConversationKind, ConversationRecord, MessageKind};

use crate::app::OrbitApp;
use crate::state::{Delivery, Link, Row, Screen, display_body, short_time};

impl Render for OrbitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.screen {
            Screen::SignedIn => self.signed_in(window, cx).into_any_element(),
            _ => self.auth(cx).into_any_element(),
        }
    }
}

impl OrbitApp {
    fn auth(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let host = self.server.as_ref().map(|u| u.host().to_owned()).unwrap_or_default();
        let title = |v: String| div().text_lg().font_semibold().child(v);
        let note = |v: String| div().text_sm().text_color(theme.muted_foreground).child(v);
        let label = |v: &'static str| div().text_xs().font_medium().text_color(theme.muted_foreground).child(v);
        let primary = |id: &'static str, text: &'static str, busy: bool| Button::new(id).primary().label(text).loading(busy).disabled(busy);
        let other_server = Button::new("other-server").ghost().label("Use another server").on_click(cx.listener(|this, _, _, cx| this.change_server(cx)));

        let card = v_flex().id("auth-card").w(px(380.)).gap_3().p_6().rounded_lg().border_1().border_color(theme.border).bg(theme.background);
        let card = match self.screen {
            Screen::Server => card
                .child(title("Connect to Orbit".into()))
                .child(note("Enter the address of your Orbit server.".into()))
                .child(label("Server address"))
                .child(Input::new(&self.inputs.server))
                .child(primary("continue", "Continue", self.busy).on_click(cx.listener(|this, _, window, cx| this.submit_server(window, cx)))),
            Screen::Gateway => card
                .child(title(format!("Sign in to {host}")))
                .child(note("Finish signing in in the window that opened. It is the server's gateway (exe.dev); Orbit asks for its own password next.".into()))
                .child(h_flex().gap_2().items_center()
                    .child(dot(theme.primary))
                    .child(div().text_sm().child("Waiting for the sign-in window…")))
                .child(Button::new("reopen").outline().label("Open the window again").on_click(cx.listener(|this, _, _, cx| this.open_gateway(cx))))
                .child(other_server),
            Screen::Login => card
                .child(title("Sign in to Orbit".into()))
                .child(note(host))
                .child(label("Email"))
                .child(Input::new(&self.inputs.email))
                .child(label("Password"))
                .child(Input::new(&self.inputs.password).mask_toggle())
                .child(primary("sign-in", "Sign in", self.busy).on_click(cx.listener(|this, _, window, cx| this.submit_login(window, cx))))
                .child(other_server),
            Screen::TwoFactor => card
                .child(title("Two-factor sign-in".into()))
                .child(note("Enter the 6-digit code from your authenticator app, or a recovery code.".into()))
                .child(label("Code"))
                .child(Input::new(&self.inputs.code))
                .child(primary("verify", "Verify", self.busy).on_click(cx.listener(|this, _, window, cx| this.submit_code(window, cx))))
                .child(other_server),
            Screen::SignedIn => card,
        };
        let card = card.when_some(self.error.clone(), |card, error| card.child(div().id("error").text_sm().text_color(theme.danger).child(error)));
        v_flex().size_full().items_center().justify_center().bg(theme.secondary).text_color(theme.foreground).child(card)
    }

    fn signed_in(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.sidebar(cx))
            .child(self.conversation(window, cx))
            .when(self.chat.thread.is_some(), |this| this.child(self.thread_panel(cx)))
    }

    fn sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut list = v_flex().id("conversations").flex_1().min_h_0().overflow_y_scroll().p_2().gap_px();
        let mut section = None;
        for r in &self.chat.rows {
            let heading = if r.kind == ConversationKind::Dm { "Direct messages" } else { "Channels" };
            if section != Some(heading) {
                section = Some(heading);
                list = list.child(div().px_2().pt_3().pb_1().text_xs().font_medium().text_color(theme.sidebar_foreground.opacity(0.6)).child(heading));
            }
            let selected = self.chat.selected.as_deref() == Some(r.id.as_str());
            let strong = r.unread > 0 && !r.muted;
            let sigil = match r.kind {
                ConversationKind::Private => "🔒",
                ConversationKind::Dm => "@",
                _ => "#",
            };
            let id = r.id.clone();
            let line = h_flex()
                .id(SharedString::from(format!("conv-{}", r.id)))
                .h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded_md()
                .text_sm()
                .when(selected, |l| l.bg(theme.sidebar_accent).text_color(theme.sidebar_accent_foreground))
                .when(!selected, |l| l.hover(|s| s.bg(theme.sidebar_accent.opacity(0.5))))
                .text_color(if strong || selected { theme.sidebar_foreground } else { theme.sidebar_foreground.opacity(0.7) })
                .when(strong, |l| l.font_semibold())
                .on_click(cx.listener(move |this, _, window, cx| this.select(&id, window, cx)))
                .child(div().w(px(14.)).text_color(theme.muted_foreground).child(sigil))
                .child(div().flex_1().truncate().child(r.name.clone()))
                .when(r.mentions > 0, |l| l.child(badge(r.mentions, theme.primary, theme.primary_foreground)))
                .when(r.mentions == 0 && strong, |l| l.child(badge(r.unread, theme.muted, theme.muted_foreground)));
            list = list.child(line);
        }
        let status = v_flex()
            .id("status")
            .px_4()
            .py_2()
            .gap_1()
            .border_t_1()
            .border_color(theme.sidebar_border)
            .child(link_line("Live", &self.live, &theme))
            .child(link_line("Updates", &self.events, &theme))
            .when(self.browser_mode, |s| {
                s.child(div().text_xs().text_color(theme.muted_foreground).child("Browser mode: the gateway does not pass the app's session"))
            });
        v_flex()
            .id("sidebar")
            .w(px(260.))
            .h_full()
            .flex_none()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(h_flex().h(px(48.)).px_4().items_center().border_b_1().border_color(theme.sidebar_border)
                .child(div().flex_1().truncate().font_semibold().text_color(theme.sidebar_foreground).child(self.chat.workspace.clone())))
            .child(list)
            .child(status)
    }

    fn conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let open = self.chat.open_row().cloned();
        let header = h_flex()
            .h(px(48.))
            .flex_none()
            .px_4()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .map(|h| match &open {
                Some(r) => h
                    .child(div().font_semibold().child(match r.kind {
                        ConversationKind::Dm => r.name.clone(),
                        _ => format!("# {}", r.name),
                    }))
                    .child(div().flex_1().truncate().text_sm().text_color(theme.muted_foreground).child(r.topic.clone())),
                None => h.child(div().flex_1()),
            })
            .child(div().text_sm().text_color(theme.muted_foreground).child(self.user.clone()))
            .child(Button::new("open-web").ghost().small().label("Open in browser").on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))))
            .child(Button::new("sign-out").ghost().small().label("Sign out").on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))));

        let update = self.update.clone().map(|latest| {
            let url = latest.download().unwrap_or_default().to_owned();
            h_flex().id("update").px_4().py_2().gap_3().items_center().bg(theme.info.opacity(0.12)).border_b_1().border_color(theme.border)
                .child(div().flex_1().text_sm().child(format!("Orbit {} is available.", latest.version)))
                .child(Button::new("download").small().primary().label("Download").on_click(cx.listener(move |this, _, _, cx| this.open_url(&url, cx))))
        });

        let body = match (&open, self.chat.messages.is_empty()) {
            (None, _) => centered(self.error.clone().unwrap_or_else(|| "Pick a conversation.".into()), theme.muted_foreground),
            (Some(_), true) if self.chat.loading => centered("Loading messages…".into(), theme.muted_foreground),
            (Some(_), true) => centered(self.chat.error.clone().unwrap_or_else(|| "No messages yet. Say hello.".into()), theme.muted_foreground),
            (Some(_), false) => {
                let rows = Rc::new(RowContext::new(self, cx.entity().downgrade(), false));
                MessageScroller::new("messages", self.scroller.clone(), move |ix, _, cx| rows.render(ix, cx))
                    .flex_1()
                    .min_h_0()
                    .into_any_element()
            }
        };
        let error = (!self.chat.messages.is_empty()).then(|| self.chat.error.clone()).flatten();

        v_flex()
            .id("conversation")
            .flex_1()
            .min_w_0()
            .h_full()
            .child(header)
            .children(update)
            .child(body)
            .when_some(error, |c, e| c.child(div().px_4().py_1().text_sm().text_color(theme.danger).child(e)))
            .when(open.is_some(), |c| c.child(self.composer(window, cx)))
    }

    fn composer(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let editing = self.chat.editing.is_some();
        v_flex()
            .id("composer")
            .flex_none()
            .px_4()
            .pb_4()
            .pt_1()
            .gap_1()
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let empty = this.inputs.composer.read(cx).value().is_empty();
                match e.keystroke.key.as_str() {
                    "escape" if this.chat.editing.is_some() => this.cancel_edit(window, cx),
                    "up" if empty && this.chat.editing.is_none() => {
                        if let Some(id) = this.chat.last_own_message() {
                            this.start_edit(&id, window, cx);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .when(editing, |c| {
                c.child(h_flex().gap_2().items_center().text_xs().text_color(theme.muted_foreground)
                    .child(div().flex_1().child("Editing your message · Enter saves · Esc cancels"))
                    .child(Button::new("cancel-edit").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, window, cx| this.cancel_edit(window, cx)))))
            })
            .when_some(self.composer_error.clone(), |c, e| c.child(div().id("composer-error").text_xs().text_color(theme.danger).child(e)))
            .child(Textarea::new(&self.inputs.composer))
    }

    fn thread_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(thread) = self.chat.thread.clone() else { return div().into_any_element() };
        let rows = Rc::new(RowContext::new(self, cx.entity().downgrade(), true));
        let root = thread.root.clone().map(|m| Row { message: m, delivery: Delivery::Sent });
        let root_view = root.map(|r| rows.message(&r, "thread-root", true, cx));
        let replies = if thread.loading && thread.replies.is_empty() {
            centered("Loading the thread…".into(), theme.muted_foreground)
        } else if thread.replies.is_empty() {
            centered("No replies yet.".into(), theme.muted_foreground)
        } else {
            let rows = rows.clone();
            MessageScroller::new("thread", self.thread_scroller.clone(), move |ix, _, cx| rows.render(ix, cx)).flex_1().min_h_0().into_any_element()
        };
        let count = thread.replies.len();
        v_flex()
            .id("thread-panel")
            .w(px(360.))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(theme.border)
            .child(h_flex().h(px(48.)).flex_none().px_4().items_center().border_b_1().border_color(theme.border)
                .child(div().flex_1().font_semibold().child("Thread"))
                .child(Button::new("close-thread").ghost().small().label("Close").on_click(cx.listener(|this, _, _, cx| this.close_thread(cx)))))
            .children(root_view.map(|v| div().border_b_1().border_color(theme.border).pb_2().child(v)))
            .child(div().px_4().py_1().text_xs().text_color(theme.muted_foreground).child(match count {
                1 => "1 reply".to_owned(),
                n => format!("{n} replies"),
            }))
            .child(replies)
            .child(div().flex_none().px_4().pb_4().pt_1().child(Textarea::new(&self.inputs.reply)))
            .into_any_element()
    }
}

/// What a row needs to draw itself inside the virtual list (which renders rows lazily, outside
/// `render`): a snapshot of the rows and names, and the app to call back.
struct RowContext {
    rows: Vec<Row>,
    names: std::collections::HashMap<String, String>,
    channels: Vec<ConversationRecord>,
    me: String,
    app: WeakEntity<OrbitApp>,
    in_thread: bool,
}

impl RowContext {
    fn new(app: &OrbitApp, weak: WeakEntity<OrbitApp>, in_thread: bool) -> Self {
        let rows = match (in_thread, &app.chat.thread) {
            (true, Some(t)) => t.replies.clone(),
            (true, None) => Vec::new(),
            (false, _) => app.chat.messages.clone(),
        };
        RowContext { rows, names: app.chat.names.clone(), channels: app.chat.conversations.clone(), me: app.chat.me.clone(), app: weak, in_thread }
    }

    fn render(&self, ix: usize, cx: &mut App) -> AnyElement {
        match self.rows.get(ix) {
            Some(row) => self.message(row, &format!("{}-{ix}", if self.in_thread { "t" } else { "m" }), false, cx),
            None => div().into_any_element(),
        }
    }

    fn message(&self, row: &Row, key: &str, is_root: bool, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let m = &row.message;
        let author = self.names.get(&m.author_id).cloned().unwrap_or_else(|| "Unknown member".into());
        let time = short_time(&m.created_at, &chrono::Local::now());
        if m.kind != MessageKind::Message {
            let what = match m.kind {
                MessageKind::Join => "joined the conversation",
                MessageKind::Leave => "left the conversation",
                MessageKind::Pin => "pinned a message",
                _ => "did something this app cannot show yet",
            };
            return h_flex().px_4().py_1().gap_2().text_sm().text_color(theme.muted_foreground)
                .child(div().w(px(32.)))
                .child(format!("{author} {what}"))
                .child(div().text_xs().child(time))
                .into_any_element();
        }
        let id = SharedString::from(format!("msg-{key}-{}", m.id));
        let own_sent = m.author_id == self.me && row.delivery == Delivery::Sent && !m.deleted;
        let body = if m.deleted { "_This message was deleted._".to_owned() } else { display_body(&m.body, &self.names, &self.channels) };
        let header = h_flex().gap_2().items_baseline()
            .child(div().text_sm().font_semibold().child(author.clone()))
            .child(div().text_xs().text_color(theme.muted_foreground).child(time))
            .when(m.edited_at.is_some(), |h| h.child(div().text_xs().text_color(theme.muted_foreground).child("(edited)")));
        let reactions = (!m.reactions.is_empty()).then(|| {
            h_flex().gap_1().flex_wrap().children(m.reactions.iter().map(|r| {
                let mine = r.user_ids.iter().any(|u| u == &self.me);
                h_flex().px_2().h(px(22.)).gap_1().items_center().rounded_full().border_1().text_xs()
                    .border_color(if mine { theme.primary } else { theme.border })
                    .bg(if mine { theme.primary.opacity(0.1) } else { theme.secondary })
                    .child(r.emoji.clone())
                    .child(r.user_ids.len().to_string())
            }))
        });
        let thread = (!is_root && !self.in_thread && m.reply_count > 0).then(|| {
            let (app, root) = (self.app.clone(), m.id.clone());
            Button::new(SharedString::from(format!("replies-{}", m.id))).link().small()
                .label(match m.reply_count {
                    1 => "1 reply".to_owned(),
                    n => format!("{n} replies"),
                })
                .on_click(move |_, _, cx| {
                    let _ = app.update(cx, |app, cx| app.open_thread(&root, cx));
                })
        });
        let delivery = match &row.delivery {
            Delivery::Sent => None,
            Delivery::Sending => Some(div().text_xs().text_color(theme.muted_foreground).child("Sending…").into_any_element()),
            Delivery::Failed(error) => {
                let (app, nonce) = (self.app.clone(), m.nonce.clone().unwrap_or_default());
                Some(h_flex().gap_2().items_center().text_xs().text_color(theme.danger)
                    .child(format!("Not sent: {error}"))
                    .child(Button::new(SharedString::from(format!("retry-{nonce}"))).xsmall().outline().label("Retry")
                        .on_click(move |_, _, cx| {
                            let _ = app.update(cx, |app, cx| app.retry(&nonce, cx));
                        }))
                    .into_any_element())
            }
        };
        let edit = (own_sent && !self.in_thread && !is_root).then(|| {
            let (app, id) = (self.app.clone(), m.id.clone());
            div().absolute().top_1().right_4().invisible().group_hover("message", |s| s.visible())
                .child(Button::new(SharedString::from(format!("edit-{}", m.id))).xsmall().outline().label("Edit")
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |app, cx| app.start_edit(&id, window, cx));
                    }))
        });
        h_flex()
            .id(id.clone())
            .group("message")
            .relative()
            .items_start()
            .gap_3()
            .px_4()
            .py_1p5()
            .hover(|s| s.bg(theme.secondary.opacity(0.5)))
            .when(row.delivery != Delivery::Sent, |r| r.opacity(0.75))
            .child(Avatar::new().name(author).small())
            .child(v_flex().flex_1().min_w_0().gap_1()
                .child(header)
                .child(TextView::markdown(id, body).selectable(true))
                .children(reactions)
                .children(thread)
                .children(delivery))
            .children(edit)
            .into_any_element()
    }
}

fn link_line(name: &str, link: &Link, theme: &gpui_kit::component::Theme) -> impl IntoElement {
    let (color, words) = match link {
        Link::Off => (theme.muted_foreground, "off".to_owned()),
        Link::Connecting => (theme.warning, "connecting…".to_owned()),
        Link::Connected => (theme.success, "connected".to_owned()),
        Link::Retrying(secs) => (theme.warning, format!("reconnecting in {secs} s")),
        Link::Ended(why) => (theme.danger, format!("stopped: {why}")),
    };
    h_flex().id(SharedString::from(format!("link-{}", name.to_lowercase()))).gap_2().items_center()
        .child(dot(color))
        .child(div().text_xs().truncate().text_color(theme.sidebar_foreground.opacity(0.7)).child(format!("{name}: {words}")))
}

fn dot(color: Hsla) -> impl IntoElement {
    div().size(px(8.)).flex_none().rounded_full().bg(color)
}

fn badge(count: i64, fill: Hsla, color: Hsla) -> impl IntoElement {
    let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
    div().h(px(18.)).min_w(px(18.)).px_1p5().rounded_full().bg(fill).text_color(color).text_xs().font_medium()
        .flex().items_center().justify_center().child(label)
}

fn centered(text: String, color: Hsla) -> AnyElement {
    v_flex().flex_1().min_h_0().items_center().justify_center().text_sm().text_color(color).child(text).into_any_element()
}
