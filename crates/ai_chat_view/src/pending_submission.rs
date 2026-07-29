use std::collections::{HashMap, VecDeque};

use crate::{ImageAttachment, MentionItem};

#[derive(Clone, Debug)]
pub(crate) struct PendingSubmission {
    pub(crate) text: String,
    pub(crate) mentions: Vec<MentionItem>,
    pub(crate) images: Vec<ImageAttachment>,
}

#[derive(Default)]
pub(crate) struct PendingSubmissions {
    by_session: HashMap<String, VecDeque<PendingSubmission>>,
}

impl PendingSubmissions {
    pub(crate) fn enqueue(&mut self, session_uid: &str, submission: PendingSubmission) {
        self.by_session
            .entry(session_uid.to_string())
            .or_default()
            .push_back(submission);
    }

    pub(crate) fn pop_front(&mut self, session_uid: &str) -> Option<PendingSubmission> {
        let queue = self.by_session.get_mut(session_uid)?;
        let submission = queue.pop_front();
        if queue.is_empty() {
            self.by_session.remove(session_uid);
        }
        submission
    }

    pub(crate) fn front(&self, session_uid: &str) -> Option<&PendingSubmission> {
        self.by_session.get(session_uid)?.front()
    }

    pub(crate) fn items(&self, session_uid: &str) -> Vec<&PendingSubmission> {
        self.by_session
            .get(session_uid)
            .map(|queue| queue.iter().collect())
            .unwrap_or_default()
    }

    pub(crate) fn clear_session(&mut self, session_uid: &str) {
        self.by_session.remove(session_uid);
    }

    pub(crate) fn remove_session(&mut self, session_uid: &str) {
        self.by_session.remove(session_uid);
    }

    pub(crate) fn len(&self, session_uid: &str) -> usize {
        self.by_session.get(session_uid).map_or(0, VecDeque::len)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_session.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{Image, ImageFormat};

    use super::{PendingSubmission, PendingSubmissions};
    use crate::{ImageAttachment, MentionItem};

    fn submission(text: &str) -> PendingSubmission {
        PendingSubmission {
            text: text.to_string(),
            mentions: Vec::new(),
            images: Vec::new(),
        }
    }

    #[test]
    fn pending_submissions_are_fifo_and_session_scoped() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("a1"));
        pending.enqueue("session-b", submission("b1"));
        pending.enqueue("session-a", submission("a2"));

        assert_eq!(2, pending.len("session-a"));
        assert_eq!(1, pending.len("session-b"));
        assert_eq!(
            vec!["a1", "a2"],
            pending
                .items("session-a")
                .into_iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            Some("a1"),
            pending
                .pop_front("session-a")
                .as_ref()
                .map(|item| item.text.as_str())
        );
        assert_eq!(
            Some("a2"),
            pending
                .pop_front("session-a")
                .as_ref()
                .map(|item| item.text.as_str())
        );
        assert!(pending.pop_front("session-a").is_none());
        assert_eq!(
            Some("b1"),
            pending
                .pop_front("session-b")
                .as_ref()
                .map(|item| item.text.as_str())
        );
    }

    #[test]
    fn front_peeks_without_consuming_the_session_queue() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("a1"));
        pending.enqueue("session-a", submission("a2"));
        pending.enqueue("session-b", submission("b1"));

        assert_eq!(
            Some("a1"),
            pending.front("session-a").map(|item| item.text.as_str())
        );
        assert_eq!(2, pending.len("session-a"));
        assert_eq!(
            Some("a1"),
            pending
                .pop_front("session-a")
                .as_ref()
                .map(|item| item.text.as_str())
        );
        assert_eq!(
            Some("a2"),
            pending.front("session-a").map(|item| item.text.as_str())
        );
        assert_eq!(
            Some("b1"),
            pending.front("session-b").map(|item| item.text.as_str())
        );
    }

    #[test]
    fn clearing_or_removing_one_session_preserves_other_sessions() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("a"));
        pending.enqueue("session-b", submission("b"));

        pending.clear_session("session-a");
        assert_eq!(0, pending.len("session-a"));
        assert_eq!(1, pending.len("session-b"));

        pending.enqueue("session-a", submission("a2"));
        pending.remove_session("session-a");
        assert_eq!(0, pending.len("session-a"));
        assert_eq!(1, pending.len("session-b"));
    }

    #[test]
    fn pending_submission_preserves_mentions_and_images() {
        let image = Arc::new(Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]));
        let mention = MentionItem::new("id", "label", "detail", "kind");
        let attachment = ImageAttachment {
            id: "image-id".to_string(),
            name: "image.png".to_string(),
            image: image.clone(),
        };
        let mut pending = PendingSubmissions::default();
        pending.enqueue(
            "session",
            PendingSubmission {
                text: "prompt".to_string(),
                mentions: vec![mention.clone()],
                images: vec![attachment],
            },
        );

        let queued = pending.pop_front("session").expect("queued submission");
        assert_eq!("prompt", queued.text);
        assert_eq!(vec![mention], queued.mentions);
        assert_eq!("image-id", queued.images[0].id);
        assert!(Arc::ptr_eq(&image, &queued.images[0].image));
    }
}
