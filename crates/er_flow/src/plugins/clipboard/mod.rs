mod clipboard;
mod clipboard_ops;
mod copied_subgraph;

pub use clipboard::ClipboardPlugin;

pub(crate) use clipboard_ops::{
    extract_subgraph, get_clipboard_subgraph, has_clipboard_subgraph, paste_subgraph,
    set_clipboard_subgraph,
};
