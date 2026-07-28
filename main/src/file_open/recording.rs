use crate::onetcli_app::GlobalTabContainer;
use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext as _, AsyncApp, Window};
use one_core::tab_container::TabItem;
use std::path::{Path, PathBuf};
use terminal::recording::{RecordingFileLimits, RecordingPlayback, RecordingPlaybackLimits};
use terminal_view::{RecordingPlaybackViewConfig, TerminalWorkspace};

use super::{show_file_open_error, stable_file_key};

struct PreparedRecordingPlayback {
    path: PathBuf,
    playback: RecordingPlayback,
    display_name: String,
}

pub(super) fn open_recording_file(path: PathBuf, window: &mut Window, cx: &mut App) {
    let window_handle = window.window_handle();
    let load_task = cx.background_spawn(async move { load_recording_playback(path) });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = load_task.await;
        let _ = cx.update_window(window_handle, move |_, window, cx| match result {
            Ok(prepared) => {
                if let Err(error) = open_prepared_recording(prepared, window, cx) {
                    show_file_open_error(&error, window, cx);
                }
            }
            Err(error) => show_file_open_error(&error, window, cx),
        });
    })
    .detach();
}

fn load_recording_playback(path: PathBuf) -> Result<PreparedRecordingPlayback> {
    let absolute = absolute_recording_path(path)?;
    let metadata = std::fs::metadata(&absolute)
        .with_context(|| format!("读取终端录制文件信息失败: {}", absolute.display()))?;
    if !metadata.is_file() {
        bail!("终端录制路径不是文件: {}", absolute.display());
    }
    let path = absolute
        .canonicalize()
        .with_context(|| format!("规范化终端录制文件路径失败: {}", absolute.display()))?;
    let display_name = recording_display_name(&path);
    let playback = RecordingPlayback::open(
        &path,
        RecordingFileLimits::default(),
        RecordingPlaybackLimits::default(),
    )
    .with_context(|| format!("解析终端录制失败: {display_name}"))?;

    Ok(PreparedRecordingPlayback {
        path,
        playback,
        display_name,
    })
}

fn absolute_recording_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .context("resolve current directory for terminal recording")?
        .join(path))
}

fn recording_display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "recording.cast".to_string())
}

fn open_prepared_recording(
    prepared: PreparedRecordingPlayback,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let tab_container = cx
        .try_global::<GlobalTabContainer>()
        .map(|global| global.primary_pane())
        .context("home page is not ready")?;
    let tab_id = format!("recording-playback-{}", stable_file_key(&prepared.path));
    let tab_id_for_create = tab_id.clone();
    let config = RecordingPlaybackViewConfig::new(prepared.playback, prepared.display_name);

    window.defer(cx, move |window, cx| {
        tab_container.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy(
                tab_id,
                move |window, cx| {
                    let workspace =
                        cx.new(|cx| TerminalWorkspace::new_recording_playback(config, window, cx));
                    TabItem::new(tab_id_for_create, "terminal-recording", workspace)
                },
                window,
                cx,
            );
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::time::Duration;
    use terminal::TerminalSize;
    use terminal::recording::{
        RecordingBackend, RecordingCompleteness, RecordingEvent, RecordingEventKind,
        RecordingFileConfig, RecordingFileWriter, RecordingMetadata,
    };

    #[test]
    fn complete_recording_is_prepared_with_a_basename_only() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("private").join("session.cast");
        write_complete_recording(&path)?;

        let prepared = load_recording_playback(path.clone())?;

        assert_eq!(path.canonicalize()?, prepared.path);
        assert_eq!("session.cast", prepared.display_name);
        assert_eq!(
            &RecordingCompleteness::Complete,
            prepared.playback.completeness()
        );
        assert!(
            !prepared
                .display_name
                .contains(temp.path().to_string_lossy().as_ref())
        );
        Ok(())
    }

    #[test]
    fn truncated_partial_recording_is_recovered_without_mutating_the_source() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let final_path = temp.path().join("session.cast");
        let partial_path = write_partial_recording(&final_path)?;
        append_truncated_event(&partial_path)?;
        let before = fs::read(&partial_path)?;

        let prepared = load_recording_playback(partial_path.clone())?;

        assert!(matches!(
            prepared.playback.completeness(),
            RecordingCompleteness::Partial { discarded_bytes } if *discarded_bytes > 0
        ));
        assert_eq!(before, fs::read(partial_path)?);
        Ok(())
    }

    #[test]
    fn truncated_published_recording_is_rejected() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("session.cast");
        write_complete_recording(&path)?;
        append_truncated_event(&path)?;

        assert!(load_recording_playback(path).is_err());
        Ok(())
    }

    #[test]
    fn unknown_recording_version_is_rejected() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("future.cast");
        fs::write(
            &path,
            concat!(
                "{\"version\":3,\"width\":80,\"height\":24,\"timestamp\":1700000000,",
                "\"navop\":{\"format_version\":1,\"recording_id\":\"recording\",",
                "\"session_id\":\"session\",\"backend\":\"local\",",
                "\"application_version\":\"test\",\"started_at_unix_ms\":1700000000000,",
                "\"capture_input\":false,\"event_stream\":\"terminal_parser_input_v1\"}}\n"
            ),
        )?;

        assert!(load_recording_playback(path).is_err());
        Ok(())
    }

    #[test]
    fn opening_contract_keeps_parsing_off_ui_and_fails_closed() {
        let source = include_str!("recording.rs");
        let production = source.split_once("#[cfg(test)]").unwrap().0;
        let scheduler = production
            .split_once("pub(super) fn open_recording_file")
            .unwrap()
            .1
            .split_once("fn load_recording_playback")
            .unwrap()
            .0;
        let loader = production
            .split_once("fn load_recording_playback")
            .unwrap()
            .1
            .split_once("fn absolute_recording_path")
            .unwrap()
            .0;
        let entity_creation = production
            .split_once("fn open_prepared_recording")
            .unwrap()
            .1;

        assert!(scheduler.contains("cx.background_spawn"));
        assert!(scheduler.contains("load_recording_playback(path)"));
        assert!(!scheduler.contains("RecordingPlayback::open"));
        assert_eq!(1, loader.matches("RecordingPlayback::open(").count());
        assert!(entity_creation.contains("TerminalWorkspace::new_recording_playback"));
        assert!(entity_creation.contains("activate_or_add_tab_lazy"));
        assert!(!production.contains("TerminalWorkspace::new("));
        assert!(!production.contains("TerminalWorkspace::new_ssh"));
        assert!(!production.contains("TerminalWorkspace::new_serial"));
    }

    fn write_complete_recording(path: &Path) -> Result<()> {
        let mut writer = recording_writer(path)?;
        writer.append(&output_event())?;
        writer.stop()?;
        Ok(())
    }

    fn write_partial_recording(final_path: &Path) -> Result<PathBuf> {
        let mut writer = recording_writer(final_path)?;
        writer.append(&output_event())?;
        writer.flush()?;
        let partial_path = writer.partial_path().to_path_buf();
        drop(writer);
        Ok(partial_path)
    }

    fn recording_writer(path: &Path) -> Result<RecordingFileWriter> {
        Ok(RecordingFileWriter::create(
            path,
            RecordingMetadata {
                recording_id: "recording".to_string(),
                session_id: "session".to_string(),
                backend: RecordingBackend::Local,
                application_version: "test".to_string(),
                started_at_unix_ms: 1_700_000_000_000,
                capture_input: false,
            },
            TerminalSize::default(),
            RecordingFileConfig::default(),
        )?)
    }

    fn output_event() -> RecordingEvent {
        RecordingEvent {
            elapsed: Duration::from_millis(100),
            kind: RecordingEventKind::Output(b"hello".to_vec()),
        }
    }

    fn append_truncated_event(path: &Path) -> Result<()> {
        OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(b"[0.2,\"o\",\"truncated")?;
        Ok(())
    }
}
