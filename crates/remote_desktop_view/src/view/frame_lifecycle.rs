pub(crate) struct RenderedFrameLifecycle<T> {
    current: Option<T>,
    previous: Option<T>,
}

impl<T> Default for RenderedFrameLifecycle<T> {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
        }
    }
}

impl<T: PartialEq> RenderedFrameLifecycle<T> {
    pub(crate) fn promote(&mut self, latest: T) -> Option<T> {
        if self.current.as_ref() == Some(&latest) {
            return None;
        }
        let current = self.current.replace(latest);
        std::mem::replace(&mut self.previous, current)
    }

    pub(crate) fn current(&self) -> Option<&T> {
        self.current.as_ref()
    }

    pub(crate) fn take_all_distinct(&mut self, latest: Option<T>) -> Vec<T> {
        let mut frames = Vec::new();
        for frame in [latest, self.current.take(), self.previous.take()]
            .into_iter()
            .flatten()
        {
            if !frames.contains(&frame) {
                frames.push(frame);
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn third_distinct_frame_retires_first_generation() {
        let mut lifecycle = RenderedFrameLifecycle::default();

        assert_eq!(None, lifecycle.promote(1));
        assert_eq!(None, lifecycle.promote(2));
        assert_eq!(Some(1), lifecycle.promote(3));
        assert_eq!(Some(&3), lifecycle.current());
    }

    #[test]
    fn promoting_current_frame_does_not_advance_generations() {
        let mut lifecycle = RenderedFrameLifecycle::default();

        lifecycle.promote(4);

        assert_eq!(None, lifecycle.promote(4));
        assert_eq!(vec![4], lifecycle.take_all_distinct(None));
    }

    #[test]
    fn release_deduplicates_latest_and_rendered_generations() {
        let mut lifecycle = RenderedFrameLifecycle::default();
        lifecycle.promote(4);
        lifecycle.promote(5);

        assert_eq!(vec![5, 4], lifecycle.take_all_distinct(Some(5)));
    }
}
