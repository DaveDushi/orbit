//! The helpdesk built into Tasks (docs/support.md): customer email in and out of a project with a
//! connected support mailbox. `inbound` parses raw MIME, `html` cleans HTML bodies, `thread` signs
//! and reads the Message-IDs that thread replies, and `outbound` is the send job. The SQL lives in
//! `repositories::support`, the routes in `support_routes`.

pub mod html;
pub mod inbound;
pub mod outbound;
pub mod thread;
