//! Everything that talks to GUC lives here, independent of the UI, so the
//! `probe` tool and tests can use it too.

pub mod academic;
pub mod client;
pub mod creds;
pub mod mail;
pub mod ntlm;
pub mod page;
pub mod portal;
