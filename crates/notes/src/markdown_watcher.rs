use crate::NotesView;
#[cfg(not(test))]
use gpui::{AppContext, AsyncApp};
use gpui::{Context, Window};
#[cfg(not(test))]
use notify::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use std::path::PathBuf;

pub(crate) fn watch_markdown_file(
    path: PathBuf,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> anyhow::Result<RecommendedWatcher> {
    #[cfg(test)]
    {
        let _ = (path, document_id, window, cx);
        return Ok(notify::recommended_watcher(|_| {})?);
    }

    #[cfg(not(test))]
    {
        let (sender, receiver) = smol::channel::unbounded();
        let watched_path = canonical_path(&path);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    )
                    && event
                        .paths
                        .iter()
                        .any(|event_path| canonical_path(event_path) == watched_path)
                {
                    let _ = sender.try_send(());
                }
            })?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Markdown file has no parent directory"))?;
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        observe_file_events(receiver, document_id, window, cx);
        Ok(watcher)
    }
}

fn canonical_path(path: &std::path::Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(not(test))]
fn observe_file_events(
    receiver: smol::channel::Receiver<()>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    let weak = cx.entity().downgrade();
    let window_handle = window.window_handle();
    cx.spawn(async move |_, cx: &mut AsyncApp| {
        while receiver.recv().await.is_ok() {
            while receiver.try_recv().is_ok() {}
            let id = document_id.clone();
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.markdown_file_changed_on_disk(&id, window, cx);
                });
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn watcher_reports_target_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("watch.md");
        std::fs::write(&path, "before").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let watched_path = canonical_path(&path);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.as_ref().is_ok_and(|event| {
                    event
                        .paths
                        .iter()
                        .any(|path| canonical_path(path) == watched_path)
                }) {
                    let _ = sender.send(());
                }
            })
            .unwrap();
        watcher
            .watch(temp.path(), RecursiveMode::NonRecursive)
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&path, "after").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("file watcher must report the target change");
    }
}
