use gpui::{Bounds, Pixels, Point};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Win32ClientPhysicalBounds {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

pub(super) fn logical_bounds_to_physical(
    bounds: Bounds<Pixels>,
    parent_client_origin: Point<Pixels>,
    scale_factor: f32,
) -> Option<Win32ClientPhysicalBounds> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }

    let origin_x = pixels_to_f64(bounds.origin.x) - pixels_to_f64(parent_client_origin.x);
    let origin_y = pixels_to_f64(bounds.origin.y) - pixels_to_f64(parent_client_origin.y);
    let width = pixels_to_f64(bounds.size.width);
    let height = pixels_to_f64(bounds.size.height);
    if width < 0.0 || height < 0.0 {
        return None;
    }

    Some(Win32ClientPhysicalBounds {
        x: scale_physical(origin_x, scale_factor)?,
        y: scale_physical(origin_y, scale_factor)?,
        width: scale_physical(width, scale_factor)?,
        height: scale_physical(height, scale_factor)?,
    })
}

fn pixels_to_f64(value: Pixels) -> f64 {
    let value: f32 = value.into();
    f64::from(value)
}

fn scale_physical(value: f64, scale_factor: f32) -> Option<i32> {
    let value = (value * f64::from(scale_factor)).round();
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return None;
    }
    Some(value as i32)
}

trait NativePresentationSink {
    type Error;

    fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error>;
    fn show(&mut self) -> Result<(), Self::Error>;
    fn focus_child(&mut self) -> Result<(), Self::Error>;
    fn focus_parent(&mut self) -> Result<(), Self::Error>;
    fn hide(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Default)]
struct WindowsNativePresentation {
    active: bool,
    visible: bool,
    latest_bounds: Option<Win32ClientPhysicalBounds>,
}

impl WindowsNativePresentation {
    fn update_bounds<S: NativePresentationSink>(
        &mut self,
        bounds: Win32ClientPhysicalBounds,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        self.latest_bounds = Some(bounds);
        if self.active {
            sink.set_bounds(bounds)?;
        }
        Ok(())
    }

    fn activate<S: NativePresentationSink>(
        &mut self,
        focus_child: bool,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        if self.active {
            return Ok(());
        }

        if let Some(bounds) = self.latest_bounds {
            sink.set_bounds(bounds)?;
        }
        if !self.visible {
            sink.show()?;
            self.visible = true;
        }
        if focus_child {
            sink.focus_child()?;
        }
        self.active = true;
        Ok(())
    }

    fn focus<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<(), S::Error> {
        if self.active && self.visible {
            sink.focus_child()?;
        }
        Ok(())
    }

    fn deactivate<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<(), S::Error> {
        if !self.active && !self.visible {
            return Ok(());
        }

        sink.focus_parent()?;
        if self.visible {
            sink.hide()?;
        }
        self.active = false;
        self.visible = false;
        Ok(())
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) struct WindowsNativeAdapter {
    presentation: WindowsNativePresentation,
    host: windows_rdp_host::WindowsRdpHost,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl WindowsNativeAdapter {
    pub(crate) fn create(window: &gpui::Window, generation: u64) -> anyhow::Result<Self> {
        use raw_window_handle::RawWindowHandle;
        use windows_rdp_host::{WindowsRdpHostOptions, WindowsRdpParentWindow};

        let raw = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("failed to get GPUI window handle: {error:?}"))?
            .as_raw();
        let RawWindowHandle::Win32(handle) = raw else {
            anyhow::bail!("GPUI window did not expose a Win32 parent handle");
        };
        let parent = unsafe { WindowsRdpParentWindow::from_raw(handle.hwnd.get() as usize) };
        let host = unsafe {
            windows_rdp_host::WindowsRdpHost::create_with_parent(
                parent,
                WindowsRdpHostOptions::new(generation),
            )
        }?;

        Ok(Self {
            presentation: WindowsNativePresentation::default(),
            host,
        })
    }

    pub(crate) fn update_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        parent_client_origin: Point<Pixels>,
        scale_factor: f32,
    ) -> anyhow::Result<()> {
        let bounds = logical_bounds_to_physical(bounds, parent_client_origin, scale_factor)
            .ok_or_else(|| anyhow::anyhow!("invalid native child bounds or scale factor"))?;
        let mut sink = WindowsNativeHostSink {
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.update_bounds(bounds, &mut sink)?;
        Ok(())
    }

    pub(crate) fn activate(&mut self, focus_child: bool) -> anyhow::Result<()> {
        let mut sink = WindowsNativeHostSink {
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.activate(focus_child, &mut sink)?;
        Ok(())
    }

    pub(crate) fn focus(&mut self) -> anyhow::Result<()> {
        let mut sink = WindowsNativeHostSink {
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.focus(&mut sink)?;
        Ok(())
    }

    pub(crate) fn deactivate(&mut self, focus_parent: &mut dyn FnMut()) -> anyhow::Result<()> {
        let mut sink = WindowsNativeHostSink {
            host: &mut self.host,
            focus_parent: Some(focus_parent),
        };
        self.presentation.deactivate(&mut sink)?;
        Ok(())
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativeHostSink<'a> {
    host: &'a mut windows_rdp_host::WindowsRdpHost,
    focus_parent: Option<&'a mut dyn FnMut()>,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl NativePresentationSink for WindowsNativeHostSink<'_> {
    type Error = windows_rdp_host::WindowsRdpHostError;

    fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
        self.host
            .set_bounds(bounds.x, bounds.y, bounds.width, bounds.height)
    }

    fn show(&mut self) -> Result<(), Self::Error> {
        self.host.set_visible(true)
    }

    fn focus_child(&mut self) -> Result<(), Self::Error> {
        self.host.focus()
    }

    fn focus_parent(&mut self) -> Result<(), Self::Error> {
        if let Some(focus_parent) = self.focus_parent.as_mut() {
            focus_parent();
        }
        Ok(())
    }

    fn hide(&mut self) -> Result<(), Self::Error> {
        self.host.set_visible(false)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use gpui::{Bounds, point, px, size};

    use super::{
        NativePresentationSink, Win32ClientPhysicalBounds, WindowsNativePresentation,
        logical_bounds_to_physical,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Command {
        SetBounds(Win32ClientPhysicalBounds),
        Show,
        FocusChild,
        FocusParent,
        Hide,
    }

    #[derive(Default)]
    struct Recorder {
        commands: Vec<Command>,
    }

    impl NativePresentationSink for Recorder {
        type Error = Infallible;

        fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
            self.commands.push(Command::SetBounds(bounds));
            Ok(())
        }

        fn show(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Show);
            Ok(())
        }

        fn focus_child(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusChild);
            Ok(())
        }

        fn focus_parent(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusParent);
            Ok(())
        }

        fn hide(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Hide);
            Ok(())
        }
    }

    fn bounds() -> Win32ClientPhysicalBounds {
        Win32ClientPhysicalBounds {
            x: 20,
            y: 40,
            width: 1600,
            height: 900,
        }
    }

    #[test]
    fn activate_applies_bounds_then_shows_and_focuses() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.activate(true, &mut recorder).unwrap();
        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
            ],
            recorder.commands
        );
    }

    #[test]
    fn deactivate_focuses_parent_before_hiding() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();
        presentation.deactivate(&mut recorder).unwrap();

        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
    }

    #[test]
    fn activate_and_deactivate_are_idempotent() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        assert_eq!(3, recorder.commands.len());

        presentation.deactivate(&mut recorder).unwrap();
        presentation.deactivate(&mut recorder).unwrap();
        assert_eq!(5, recorder.commands.len());
    }

    #[test]
    fn inactive_resize_only_updates_the_cached_bounds() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();
        let latest = Win32ClientPhysicalBounds {
            x: 30,
            y: 50,
            width: 1280,
            height: 720,
        };

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.update_bounds(latest, &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.activate(false, &mut recorder).unwrap();
        assert_eq!(
            vec![Command::SetBounds(latest), Command::Show],
            recorder.commands
        );
    }

    #[test]
    fn active_resize_updates_bounds_without_changing_visibility() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();
        let latest = Win32ClientPhysicalBounds {
            x: 30,
            y: 50,
            width: 1280,
            height: 720,
        };

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();
        presentation.update_bounds(latest, &mut recorder).unwrap();

        assert_eq!(vec![Command::SetBounds(latest)], recorder.commands);
    }

    #[test]
    fn converts_logical_bounds_at_supported_dpi_scales() {
        let logical = Bounds::new(point(px(10.0), px(20.0)), size(px(800.0), px(600.0)));
        let parent_origin = point(px(2.0), px(4.0));

        for (scale_factor, expected) in [
            (
                1.0,
                Win32ClientPhysicalBounds {
                    x: 8,
                    y: 16,
                    width: 800,
                    height: 600,
                },
            ),
            (
                1.25,
                Win32ClientPhysicalBounds {
                    x: 10,
                    y: 20,
                    width: 1000,
                    height: 750,
                },
            ),
            (
                1.5,
                Win32ClientPhysicalBounds {
                    x: 12,
                    y: 24,
                    width: 1200,
                    height: 900,
                },
            ),
            (
                2.0,
                Win32ClientPhysicalBounds {
                    x: 16,
                    y: 32,
                    width: 1600,
                    height: 1200,
                },
            ),
        ] {
            assert_eq!(
                Some(expected),
                logical_bounds_to_physical(logical, parent_origin, scale_factor)
            );
        }
    }

    #[test]
    fn conversion_preserves_negative_origins_and_zero_size() {
        let logical = Bounds::new(point(px(1.0), px(2.0)), size(px(0.0), px(0.0)));
        let parent_origin = point(px(3.0), px(5.0));

        assert_eq!(
            Some(Win32ClientPhysicalBounds {
                x: -3,
                y: -5,
                width: 0,
                height: 0,
            }),
            logical_bounds_to_physical(logical, parent_origin, 1.5)
        );
    }

    #[test]
    fn conversion_rejects_invalid_scale_factors() {
        let logical = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        let origin = point(px(0.0), px(0.0));

        assert_eq!(None, logical_bounds_to_physical(logical, origin, 0.0));
        assert_eq!(None, logical_bounds_to_physical(logical, origin, f32::NAN));
    }
}
