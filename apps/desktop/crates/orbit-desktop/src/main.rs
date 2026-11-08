//! Orbit desktop, M0: one window with the workspace's conversations, from a fixture backend.
//! `orbit-desktop --fixture apps/desktop/fixtures/demo` opens it; add `--snap out.png` to render
//! it headless (`--scene selected` opens the first unread conversation, `--theme light|dark`).

use std::path::PathBuf;

use orbit_client::{ConversationKind, Fixture, SidebarRow};
use rune::tok::{self, radius, type_};
use rune::{AppSpec, Cx, El, ScrollSpec, col, el, row, text};

#[derive(Default)]
struct State {
    workspace: String,
    rows: Vec<SidebarRow>,
    selected: Option<String>,
    /// Why there is nothing to show.
    error: Option<String>,
}

fn init() -> State {
    let args = rune::extra_args();
    let fixture = args.iter().position(|a| a == "--fixture").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let Some(dir) = fixture else {
        return State { error: Some("Not signed in. M0 runs only with --fixture <dir>.".into()), ..State::default() };
    };
    match Fixture::load(&dir) {
        Ok(f) => State { workspace: f.workspace_name.clone(), rows: f.sidebar(), ..State::default() },
        Err(e) => State { error: Some(e.to_string()), ..State::default() },
    }
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
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
        .child(text(s.workspace.clone()).font(type_::TITLE).color(t.text));
    let sidebar = col().key("sidebar").w(260.0).h_full().fill(t.card).border_sides([0.0, 1.0, 0.0, 0.0], t.separator)
        .child(header)
        .child(list.scroll(ScrollSpec { stick_bottom: false }).shrink().min_h(0.0));

    let open = s.selected.as_ref().and_then(|id| s.rows.iter().find(|r| &r.id == id));
    let main = match (&s.error, open) {
        (Some(error), _) => centered(text(error.clone()).font(type_::BODY).color(t.text_secondary)),
        (None, Some(r)) => col().key("conversation").grow(1.0).h_full()
            .child(row().h(48.0).px(16.0).gap(12.0).center_y().border_sides([0.0, 0.0, 1.0, 0.0], t.separator)
                .child(text(r.name.clone()).font(type_::TITLE).color(t.text))
                .child(text(r.topic.clone()).font(type_::BODY).color(t.text_secondary).truncate()))
            .child(centered(text("Messages come in M1.").font(type_::BODY).color(t.text_secondary))),
        (None, None) => centered(text("Pick a conversation.").font(type_::BODY).color(t.text_secondary)),
    };

    row().w_full().h_full().fill(t.bg).child(sidebar).child(main)
}

fn badge(count: i64, fill: rune::Color, color: rune::Color) -> El {
    let label = if count > 99 { "99+".to_owned() } else { count.to_string() };
    el().h(18.0).px(6.0).radius(9.0).fill(fill).center()
        .child(text(label).font(type_::LABEL).color(color))
}

fn centered(child: El) -> El {
    col().grow(1.0).h_full().center().child(child)
}

fn main() {
    rune::run(AppSpec::<State> {
        scenes: &[("selected", |s| s.selected = s.rows.iter().find(|r| r.unread > 0).map(|r| r.id.clone()))],
        min_size: Some((640.0, 400.0)),
        ..AppSpec::new("Orbit", (900.0, 560.0), init, view)
    });
}
