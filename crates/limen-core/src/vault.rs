//! The vault: notes agents leave each other in a shared Git checkout. This is
//! the Rust port of the Python Limen's vault, in the `lc` process itself: no
//! core, no server, no RPC. The file format is the same on both sides.

mod clock;
mod git;
mod listing;
mod marks;
mod notes;
mod pen;
mod privacy;
mod store;

pub use listing::read_text_of;
pub use marks::{Done, ReadBy};
pub use notes::{KINDS, STATUSES};
pub use store::{Message, Post, Query, Report, Vault, Written, older_than_a_week};
