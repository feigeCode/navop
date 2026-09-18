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

    /// 删除队列中第 `index` 条；越界返回 `None`。删除不改变其余条目的相对顺序。
    pub(crate) fn remove_at(
        &mut self,
        session_uid: &str,
        index: usize,
    ) -> Option<PendingSubmission> {
        let queue = self.by_session.get_mut(session_uid)?;
        let removed = queue.remove(index);
        if queue.is_empty() {
            self.by_session.remove(session_uid);
        }
        removed
    }

    /// 用编辑后的条目替换第 `index` 条，**不改变队列位置**；越界返回 `None`。
    ///
    /// 「编辑」的语义是**取回输入框**（见 [`Self::remove_at`]），所以生产路径不经过这里；
    /// 保留它是为了让「就地改文案、位置不动」这条后续路径有落点，且已有单测锁住语义。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn replace_at(
        &mut self,
        session_uid: &str,
        index: usize,
        submission: PendingSubmission,
    ) -> Option<PendingSubmission> {
        let queue = self.by_session.get_mut(session_uid)?;
        let slot = queue.get_mut(index)?;
        Some(std::mem::replace(slot, submission))
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
    fn remove_at_drops_one_entry_and_keeps_order() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("a1"));
        pending.enqueue("session-a", submission("a2"));
        pending.enqueue("session-a", submission("a3"));
        pending.enqueue("session-b", submission("b1"));

        let removed = pending.remove_at("session-a", 1).expect("removed entry");
        assert_eq!("a2", removed.text);
        assert_eq!(
            vec!["a1", "a3"],
            pending
                .items("session-a")
                .into_iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(1, pending.len("session-b"));
        assert!(pending.remove_at("session-a", 9).is_none());
    }

    #[test]
    fn remove_at_drops_the_session_when_the_queue_empties() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("only"));

        assert!(pending.remove_at("session-a", 0).is_some());
        assert_eq!(0, pending.len("session-a"));
        assert!(pending.items("session-a").is_empty());
    }

    #[test]
    fn replace_at_edits_in_place_without_reordering() {
        let mut pending = PendingSubmissions::default();
        pending.enqueue("session-a", submission("a1"));
        pending.enqueue("session-a", submission("a2"));
        pending.enqueue("session-a", submission("a3"));

        let previous = pending
            .replace_at("session-a", 1, submission("a2-edited"))
            .expect("replaced entry");
        assert_eq!("a2", previous.text);
        assert_eq!(
            vec!["a1", "a2-edited", "a3"],
            pending
                .items("session-a")
                .into_iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
        );
        assert!(pending.replace_at("session-a", 9, submission("nope")).is_none());
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
