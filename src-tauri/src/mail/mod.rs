pub mod autoconfig;
pub mod imap_client;
pub mod oauth;
pub mod parse;
pub mod sync;
pub mod threading;

#[cfg(test)]
#[path = "threading_tests.rs"]
mod threading_tests;
