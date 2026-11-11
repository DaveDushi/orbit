//! Work that runs later: the agent's routines (a recurring job that the agent does as its
//! creator and posts in a conversation), reminders (a one-time DM from the agent) and scheduled
//! messages (posted as their author). `runner` runs what is due; `store` keeps the rows and
//! checks who may change them. Design notes: `.ai/agent-design.md`, "Routines".

mod routes;
pub mod runner;
pub mod schedule;
pub mod store;

pub use routes::router;
pub use runner::{Round, run_due, run_service};
