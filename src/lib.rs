#[cfg(windows)]
pub mod app;
mod auth;
#[cfg(windows)]
mod auto_updates;
mod brand;
mod clipboard;
pub mod config;
mod diagnostics;
#[cfg(windows)]
mod fluent;
#[cfg(windows)]
mod flyout;
mod http;
mod image;
#[cfg(test)]
mod image_tests;
#[cfg(windows)]
pub mod integration;
#[cfg(windows)]
mod iphone;
mod lifecycle;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod network;
#[cfg(windows)]
mod popup;
#[cfg(windows)]
mod preferences_window;
#[cfg(test)]
mod receive_tests;
mod settings;
#[cfg(windows)]
mod setup;
mod sharing;
mod tailnet;
mod transport;
#[cfg(test)]
mod transport_tests;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
mod updates;
#[cfg(windows)]
mod watcher;
#[cfg(windows)]
mod wizard;
mod writes;
