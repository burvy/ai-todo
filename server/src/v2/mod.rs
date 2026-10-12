//! The v2 server, built alongside v1 and replacing it (see
//! `docs/ARCHITECTURE.md` sections 7 and 10).
//!
//! Nothing here is wired into the router yet: `App` and the handlers are the
//! next change, and until then this module only has to be correct and tested.

pub mod store;
