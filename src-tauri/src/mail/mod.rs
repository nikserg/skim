pub mod autoconfig;
pub mod ics;
pub mod imap_client;
pub mod lang;
pub mod oauth;
pub mod parse;
pub mod sanitize;
pub mod smtp;
pub mod suspicion;
pub mod sync;
pub mod threading;
pub mod translate;

#[cfg(test)]
#[path = "threading_tests.rs"]
mod threading_tests;
