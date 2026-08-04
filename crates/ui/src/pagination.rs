use std::{ops::Range, rc::Rc};

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, prelude::FluentBuilder, px,
};
use rust_i18n::t;

use crate::{
    Disableable, Icon, Selectable, Sizable, Size, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    icon::IconName,
    menu::{DropdownMenu as _, PopupMenuItem},
};

const MIN_VISIBLE_PAGES: usize = 5;

/// Pagination with page navigation, next and previous links.
#[derive(IntoElement)]
pub struct Pagination {
    id: ElementId,
    style: StyleRefinement,
    size: Size,
    current_page: usize,
    total_pages: usize,
    disabled: bool,
    compact: bool,
    visible_pages: usize,
    on_click: Option<Rc<dyn Fn(&usize, &mut Window, &mut App)>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum PageItem {
    Page(usize),
    Ellipsis(Range<usize>),
}

impl Pagination {
    /// Create a new Pagination component with the given ID.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            size: Size::default(),
            current_page: 1,
            total_pages: 1,
            visible_pages: 5,
            disabled: false,
            compact: false,
            on_click: None,
        }
    }

    /// Set the current page number (1-based).
    ///
    pub fn current_page(mut self, page: usize) -> Self {
        self.current_page = page;
        self
    }

    /// Set the total number of pages.
    pub fn total_pages(mut self, pages: usize) -> Self {
        self.total_pages = pages;
        self
    }

    /// Set the handler for page change (when clicking on page numbers, prev, or next).
    ///
    /// This handler receives the new page number to navigate to.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// Pagination::new("my-pagination")
    ///     .current_page(current_page)
    ///     .total_pages(total_pages)
    ///     .on_click(|page, _, cx| {
    ///         // Handle page change
    ///     })
    /// ```
    pub fn on_click(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Set to display as compact style.
    ///
    /// If true, only the prev, next buttons with only icon.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Set viewable maximum number of page buttons, default
    pub fn visible_pages(mut self, max: usize) -> Self {
        self.visible_pages = max.max(MIN_VISIBLE_PAGES);
        self
    }

    fn normalized_total_pages(&self) -> usize {
        self.total_pages.max(1)
    }

    fn normalized_current_page(&self) -> usize {
        self.current_page.clamp(1, self.normalized_total_pages())
    }

    fn render_nav_button(&self, is_prev: bool, current_page: usize, total_pages: usize) -> Button {
        let (id, label, icon, disabled) = if is_prev {
            (
                "prev",
                t!("Pagination.previous"),
                IconName::ChevronLeft,
                current_page <= 1,
            )
        } else {
            (
                "next",
                t!("Pagination.next"),
                IconName::ChevronRight,
                current_page >= total_pages,
            )
        };

        let target_page = if is_prev {
            current_page.saturating_sub(1).max(1)
        } else {
            current_page.saturating_add(1).min(total_pages)
        };

        Button::new(id)
            .ghost()
            .compact()
            .with_size(self.size)
            .disabled(self.disabled || disabled)
            .tooltip(label.clone())
            .when(self.compact, |this| this.icon(icon))
            .when(!self.compact, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .flex_nowrap()
                        .when(is_prev, |this| this.flex_row_reverse())
                        .child(SharedString::from(label))
                        .child(Icon::new(icon)),
                )
            })
            .when_some(self.on_click.clone(), |this, handler| {
                this.on_click(move |_, window, cx| {
                    handler(&target_page, window, cx);
                })
            })
    }
}

impl Disableable for Pagination {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Sizable for Pagination {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Styled for Pagination {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Pagination {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let current_page = self.normalized_current_page();
        let total_pages = self.normalized_total_pages();
        let page_numbers = if !self.compact {
            calculate_page_range(current_page, total_pages, self.visible_pages)
        } else {
            vec![]
        };

        let is_disabled = self.disabled;
        let on_click = self.on_click.clone();

        h_flex()
            .id(self.id.clone())
            .px_2()
            .py_2()
            .gap_1()
            .items_center()
            .refine_style(&self.style)
            .child(self.render_nav_button(true, current_page, total_pages))
            .children({
                page_numbers.into_iter().map(|item| match item {
                    PageItem::Page(page) => {
                        let is_selected = page == current_page;

                        Button::new(page)
                            .with_size(self.size)
                            .selected(is_selected)
                            .map(|this| {
                                if is_selected {
                                    this.outline()
                                } else {
                                    this.ghost()
                                }
                            })
                            .label(page.to_string())
                            .compact()
                            .disabled(is_disabled)
                            .when(!is_selected, |this| {
                                this.when_some(on_click.clone(), |this, handler| {
                                    this.on_click(move |_, window, cx| {
                                        handler(&page, window, cx);
                                    })
                                })
                            })
                            .into_any_element()
                    }
                    PageItem::Ellipsis(range) => Button::new(SharedString::from(format!(
                        "ellipsis-{}-{}",
                        range.start, range.end
                    )))
                    .ghost()
                    .with_size(self.size)
                    .compact()
                    .disabled(self.disabled)
                    .tooltip(t!("Pagination.select_hidden_page"))
                    .icon(IconName::Ellipsis)
                    .dropdown_menu({
                        let on_click = on_click.clone();
                        move |mut menu, _, _| {
                            for page in range.clone() {
                                menu = menu.item(
                                    PopupMenuItem::new(format!("{}", page))
                                        .checked(page == current_page)
                                        .on_click({
                                            let on_click = on_click.clone();
                                            move |_, window, cx| {
                                                if let Some(handler) = &on_click {
                                                    handler(&page, window, cx);
                                                }
                                            }
                                        }),
                                )
                            }

                            menu.min_w(px(55.)).max_h(px(240.)).scrollable(true)
                        }
                    })
                    .into_any_element(),
                })
            })
            .child(self.render_nav_button(false, current_page, total_pages))
    }
}

fn calculate_page_range(current: usize, total: usize, max_visible: usize) -> Vec<PageItem> {
    let total = total.max(1);
    let current = current.clamp(1, total);

    if total <= 1 {
        return vec![];
    }

    let max_visible = max_visible.max(MIN_VISIBLE_PAGES);

    if total <= max_visible {
        return (1..=total).map(PageItem::Page).collect();
    }

    let mut pages = vec![];
    let side_pages = (max_visible - 3) / 2;

    pages.push(PageItem::Page(1));

    let start = if current <= side_pages + 1 {
        2
    } else if current > total - side_pages - 1 {
        total - side_pages - 1
    } else {
        current - side_pages
    };

    if start > 2 {
        pages.push(PageItem::Ellipsis(2..start));
    }

    let end = if current >= total - side_pages {
        total - 1
    } else if current <= side_pages + 1 {
        side_pages + 2
    } else {
        current + side_pages
    };

    for page in start..=end {
        pages.push(PageItem::Page(page));
    }

    if end < total - 1 {
        pages.push(PageItem::Ellipsis(end + 1..total));
    }

    pages.push(PageItem::Page(total));

    pages
}

#[cfg(test)]
mod tests {
    use super::{MIN_VISIBLE_PAGES, PageItem, Pagination, calculate_page_range};

    #[test]
    fn test_calculate_page_range() {
        let result = calculate_page_range(1, 10, 7);
        let expected = vec![
            PageItem::Page(1),
            PageItem::Page(2),
            PageItem::Page(3),
            PageItem::Page(4),
            PageItem::Ellipsis(5..10),
            PageItem::Page(10),
        ];
        assert_eq!(result, expected);

        let result = calculate_page_range(5, 10, 7);
        let expected = vec![
            PageItem::Page(1),
            PageItem::Ellipsis(2..3),
            PageItem::Page(3),
            PageItem::Page(4),
            PageItem::Page(5),
            PageItem::Page(6),
            PageItem::Page(7),
            PageItem::Ellipsis(8..10),
            PageItem::Page(10),
        ];
        assert_eq!(result, expected);

        let result = calculate_page_range(10, 10, 7);
        let expected = vec![
            PageItem::Page(1),
            PageItem::Ellipsis(2..7),
            PageItem::Page(7),
            PageItem::Page(8),
            PageItem::Page(9),
            PageItem::Page(10),
        ];
        assert_eq!(result, expected);
    }

    #[test]
    fn pagination_normalization_is_builder_order_independent() {
        let current_then_total = Pagination::new("a").current_page(10).total_pages(3);
        let total_then_current = Pagination::new("b").total_pages(3).current_page(10);

        assert_eq!(current_then_total.normalized_total_pages(), 3);
        assert_eq!(current_then_total.normalized_current_page(), 3);
        assert_eq!(total_then_current.normalized_total_pages(), 3);
        assert_eq!(total_then_current.normalized_current_page(), 3);
    }

    #[test]
    fn pagination_normalizes_zero_values_and_visible_page_floor() {
        let pagination = Pagination::new("zero")
            .current_page(0)
            .total_pages(0)
            .visible_pages(0);

        assert_eq!(pagination.normalized_total_pages(), 1);
        assert_eq!(pagination.normalized_current_page(), 1);
        assert_eq!(pagination.visible_pages, MIN_VISIBLE_PAGES);
        assert!(calculate_page_range(0, 0, 0).is_empty());
    }

    #[test]
    fn calculate_page_range_clamps_out_of_range_current_pages() {
        assert_eq!(
            calculate_page_range(usize::MAX, 3, 1),
            vec![PageItem::Page(1), PageItem::Page(2), PageItem::Page(3)]
        );
        assert_eq!(
            calculate_page_range(0, 3, 4),
            vec![PageItem::Page(1), PageItem::Page(2), PageItem::Page(3)]
        );
    }
}
