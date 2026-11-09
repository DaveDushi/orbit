//! The views: the sign-in screens and the signed-in window (sidebar, socket status).

use orbit_client::ConversationKind;
use rune::tok::{self, radius, type_};
use rune::{Cx, El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit, TextInput};

use crate::app::{Link, Screen, State};

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    match s.screen {
        Screen::SignedIn => signed_in(s, dark),
        _ => auth(s, dark),
    }
}

fn error_color(dark: bool) -> rune::Color {
    rune::oklch(if dark { 0.72 } else { 0.55 }, 0.19, 27.0)
}

fn auth(s: &State, dark: bool) -> El {
    let t = tok::theme(dark);
    let k = Kit::neutral(dark);
    let host = s.server.as_ref().map(|u| u.host().to_owned()).unwrap_or_default();
    let title = |v: &str| text(v.to_owned()).font(type_::TITLE).color(t.text);
    let note = |v: String| text(v).font(type_::BODY).color(t.text_secondary).wrap();
    let label = |v: &str| text(v.to_owned()).font(type_::LABEL).color(t.text_secondary);
    let primary = |v: &str, key: &str| {
        Button::new(v).variant(if s.busy { ButtonVariant::Loading } else { ButtonVariant::Submit }).disabled(s.busy).build(&k).key(key.to_owned())
    };
    let other_server = Button::new("Use another server").variant(ButtonVariant::Ghost)
        .on_press(|s: &mut State| s.change_server()).build(&k).key("other-server");

    let mut card = col().key(format!("{:?}", s.screen).to_lowercase()).w(360.0).gap(12.0).padding(24.0).radius(radius::LG)
        .fill(t.card).border(1.0, t.card_border).items_stretch();
    card = match s.screen {
        Screen::Server => card
            .child(title("Connect to Orbit"))
            .child(note("Enter the address of your Orbit server.".into()))
            .child(label("Server address"))
            .child(TextInput::new(&s.server_edit).placeholder("orbit.example.com").label("Server address")
                .width(Some(312.0)).on_edit(|s: &mut State| &mut s.server_edit).on_submit(|s: &mut State, _| s.submit_server()).build(&k).key("server"))
            .child(primary("Continue", "continue").on_press(|s: &mut State| s.submit_server())),
        Screen::Gateway => card
            .child(title(&format!("Sign in to {host}")))
            .child(note("Finish signing in in the window that opened. It is the server's gateway (exe.dev); Orbit asks for its own password next.".into()))
            .child(row().gap(8.0).center_y()
                .child(el().w(8.0).h(8.0).radius(4.0).fill(tok::accent(0, dark)))
                .child(text("Waiting for the sign-in window…").font(type_::BODY).color(t.text)))
            .child(Button::new("Open the window again").variant(ButtonVariant::Outline)
                .on_press(|s: &mut State| s.open_gateway()).build(&k).key("reopen"))
            .child(other_server),
        Screen::Login => card
            .child(title("Sign in to Orbit"))
            .child(note(host.clone()))
            .child(label("Email"))
            .child(TextInput::new(&s.email).placeholder("you@example.com").label("Email")
                .width(Some(312.0)).on_edit(|s: &mut State| &mut s.email).build(&k).key("email"))
            .child(label("Password"))
            .child(TextInput::new(&s.password).secure().label("Password")
                .width(Some(312.0)).on_edit(|s: &mut State| &mut s.password).on_submit(|s: &mut State, _| s.submit_login()).build(&k).key("password"))
            .child(primary("Sign in", "sign-in").on_press(|s: &mut State| s.submit_login()))
            .child(other_server),
        Screen::TwoFactor => card
            .child(title("Two-factor sign-in"))
            .child(note("Enter the 6-digit code from your authenticator app, or a recovery code.".into()))
            .child(label("Code"))
            .child(TextInput::new(&s.code).placeholder("123456").label("Code")
                .width(Some(312.0)).on_edit(|s: &mut State| &mut s.code).on_submit(|s: &mut State, _| s.submit_code()).build(&k).key("code"))
            .child(primary("Verify", "verify").on_press(|s: &mut State| s.submit_code()))
            .child(other_server),
        Screen::SignedIn => card,
    };
    if let Some(error) = &s.error {
        card = card.child(text(error.clone()).key("error").role("alert").font(type_::BODY).color(error_color(dark)).wrap());
    }
    col().w_full().h_full().fill(t.bg).center().child(card)
}

fn signed_in(s: &State, dark: bool) -> El {
    let t = tok::theme(dark);
    let k = Kit::neutral(dark);
    let accent = tok::accent(0, dark);

    let mut list = col().key("conversations").w_full().gap(2.0).padding(8.0).role("list").label("Conversations");
    let mut dms_started = false;
    for r in &s.rows {
        if r.kind == ConversationKind::Dm && !dms_started {
            dms_started = true;
            list = list.child(text("Direct messages").font(type_::SECTION).color(t.text_secondary).px(8.0).py(6.0));
        }
        let selected = s.selected.as_deref() == Some(r.id.as_str());
        let sigil = match r.kind {
            ConversationKind::Private => "🔒",
            ConversationKind::Dm => "@",
            _ => "#",
        };
        let strong = r.unread > 0 && !r.muted;
        let name = text(format!("{sigil} {}", r.name)).font(if strong { type_::TITLE } else { type_::BODY })
            .color(if strong || selected { t.text } else { t.text_secondary }).grow(1.0).truncate();
        let mut line = row().key(format!("conv-{}", r.id)).role("button").label(r.name.clone())
            .h(30.0).px(8.0).gap(8.0).center_y().radius(radius::SM)
            .fill(if selected { t.press } else { t.bg.alpha(0.0) })
            .on_press({
                let id = r.id.clone();
                move |s: &mut State| s.selected = Some(id.clone())
            })
            .child(name);
        if r.mentions > 0 {
            line = line.child(badge(r.mentions, accent, t.bg));
        } else if r.unread > 0 && !r.muted {
            line = line.child(badge(r.unread, t.control_off, t.text));
        }
        list = list.child(line);
    }

    let header = row().h(48.0).px(16.0).center_y()
        .child(text(s.workspace.clone()).font(type_::TITLE).color(t.text).grow(1.0).truncate());
    let status = col().key("status").px(16.0).py(10.0).gap(4.0).border_sides([1.0, 0.0, 0.0, 0.0], t.separator)
        .child(link_line("Live", &s.live, dark))
        .child(link_line("Updates", &s.events, dark));
    let sidebar = col().key("sidebar").w(260.0).h_full().fill(t.card).border_sides([0.0, 1.0, 0.0, 0.0], t.separator)
        .child(header)
        .child(list.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).shrink().min_h(0.0))
        .child(status);

    let top = row().h(48.0).px(16.0).gap(12.0).center_y().border_sides([0.0, 0.0, 1.0, 0.0], t.separator);
    let open = s.selected.as_ref().and_then(|id| s.rows.iter().find(|r| &r.id == id));
    let top = match open {
        Some(r) => top
            .child(text(r.name.clone()).font(type_::TITLE).color(t.text))
            .child(text(r.topic.clone()).font(type_::BODY).color(t.text_secondary).truncate().grow(1.0)),
        None => top.child(el().grow(1.0)),
    }
    .child(text(s.user.clone()).key("user").font(type_::BODY).color(t.text_secondary))
    .child(Button::new("Sign out").variant(ButtonVariant::Ghost).on_press(|s: &mut State| s.sign_out()).build(&k).key("sign-out"));
    let body = match (&s.error, open) {
        (Some(error), _) => centered(text(error.clone()).font(type_::BODY).color(error_color(dark))),
        (None, Some(_)) => centered(text("Messages come in a later milestone.").font(type_::BODY).color(t.text_secondary)),
        (None, None) => centered(text("Pick a conversation.").font(type_::BODY).color(t.text_secondary)),
    };
    let main = col().key("conversation").grow(1.0).h_full().child(top).child(body);

    row().w_full().h_full().fill(t.bg).child(sidebar).child(main)
}

fn link_line(name: &str, link: &Link, dark: bool) -> El {
    let t = tok::theme(dark);
    let (color, words) = match link {
        Link::Off => (t.control_off, "off".to_owned()),
        Link::Connecting => (rune::oklch(0.78, 0.15, 75.0), "connecting…".to_owned()),
        Link::Connected => (rune::oklch(0.7, 0.17, 150.0), "connected".to_owned()),
        Link::Retrying(secs) => (rune::oklch(0.78, 0.15, 75.0), format!("reconnecting in {secs} s")),
        Link::Ended(why) => (error_color(dark), format!("stopped: {why}")),
    };
    row().key(format!("link-{}", name.to_lowercase())).gap(8.0).center_y().role("status").label(format!("{name}: {words}"))
        .child(el().w(8.0).h(8.0).radius(4.0).fill(color))
        .child(text(format!("{name}: {words}")).font(type_::LABEL).color(t.text_secondary).truncate())
}

fn badge(count: i64, fill: rune::Color, color: rune::Color) -> El {
    let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
    el().h(18.0).px(6.0).radius(9.0).fill(fill).center()
        .child(text(label).font(type_::LABEL).color(color))
}

fn centered(child: El) -> El {
    col().grow(1.0).h_full().center().child(child)
}
