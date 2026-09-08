//! Known Hosts view: browse and import the SSH servers trusted in the app's
//! host-key trust store. The data layer lives in the `ssh` crate; this crate
//! only renders it as a `TabContent` page and never touches the trust store on
//! the UI thread.

rust_i18n::i18n!("locales", fallback = "en");

mod page;
mod render;

pub use page::KnownHostsPage;
