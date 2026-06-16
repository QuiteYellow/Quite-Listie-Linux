//! Quite Listie core — platform-agnostic business logic shared by every front-end.
//!
//! Contains the data model, the Nextcloud WebDAV sync engine, the three-way merge,
//! reminders/recurrence, deeplinks, markdown, and location parsing. No GUI toolkit
//! dependency: the GTK (GNOME) front-end and the legacy cxx-qt (KDE) front-end both
//! consume this crate.

pub mod engine;
pub mod model;
pub mod presets;
pub mod util;
