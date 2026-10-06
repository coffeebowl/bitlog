//! A project as its page shows it: the activity of the last 30 days, the
//! stronger a day the more time was spent, and below it miniatures of its
//! notes and files.

use gettextrs::gettext;
use gtk::graphene;
use gtk::prelude::*;

use super::{append_layout, fill_rounded, layout};
use crate::colors::{parse, with_alpha};

const DAYS: usize = 30;
const MAX_CELL: f32 = 12.0;
/// Between the activity and the pages.
const GAP: f32 = 20.0;
const MAX_PAGE_WIDTH: f32 = 96.0;
const PAGE_RATIO: f32 = 1.3;
const PAGE_SPACING: f32 = 16.0;
/// Below a page, for its name.
const NAME_HEIGHT: f32 = 24.0;
pub(super) const HEIGHT: f32 = MAX_CELL + GAP + MAX_PAGE_WIDTH * PAGE_RATIO + NAME_HEIGHT;

/// How strong each day of a week is, Monday first: busy workdays, a free
/// weekend.
const WEEK: [f32; 7] = [0.75, 1.0, 0.5, 1.0, 0.3, 0.0, 0.0];

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let color = parse("#3584e4");

    // A row of days, a quarter day apart, as in the project view.
    let cell = (width / (DAYS as f32 * 1.25 - 0.25)).min(MAX_CELL);
    let gap = cell / 4.0;
    for day in 0..DAYS {
        let level = WEEK[day % 7] * if day % 11 == 4 { 0.4 } else { 1.0 };
        let fill = if level > 0.0 {
            with_alpha(&color, level)
        } else {
            with_alpha(&foreground, 0.08)
        };
        let x = day as f32 * (cell + gap);
        fill_rounded(
            snapshot,
            graphene::Rect::new(x, 0.0, cell, cell),
            gap,
            &fill,
        );
    }

    let page_width = ((width - 2.0 * PAGE_SPACING) / 3.0).min(MAX_PAGE_WIDTH);
    let page_height = page_width * PAGE_RATIO;
    let top = MAX_CELL + GAP;
    let pages = [
        (gettext("deployment"), Page::Note),
        (gettext("checkout-flow"), Page::Note),
        (gettext("offer.pdf"), Page::File),
    ];
    for (index, (name, page)) in pages.iter().enumerate() {
        let x = index as f32 * (page_width + PAGE_SPACING);
        let rect = graphene::Rect::new(x, top, page_width, page_height);
        append_page(widget, snapshot, rect, page, &color);
        let name = layout(widget, name, Some(page_width));
        let y = top + page_height + NAME_HEIGHT / 2.0;
        append_layout(snapshot, &name, x, y, 0.0, &foreground);
    }
}

enum Page {
    Note,
    File,
}

/// A page with a title and lines of text, or a file with its type, as
/// the miniatures of the app: lighter than what is around them.
fn append_page(
    widget: &gtk::Widget,
    snapshot: &gtk::Snapshot,
    rect: graphene::Rect,
    page: &Page,
    color: &gtk::gdk::RGBA,
) {
    let foreground = widget.color();
    fill_rounded(snapshot, rect, 4.0, &with_alpha(&foreground, 0.15));
    let paper = if adw::StyleManager::default().is_dark() {
        with_alpha(&gtk::gdk::RGBA::WHITE, 0.08)
    } else {
        gtk::gdk::RGBA::WHITE
    };
    fill_rounded(snapshot, rect.inset_r(1.0, 1.0), 3.0, &paper);
    let pad = rect.width() * 0.12;
    let line_width = rect.width() - 2.0 * pad;
    let bar = |y: f32, share: f32, alpha: f32| {
        let bar = graphene::Rect::new(rect.x() + pad, y, line_width * share, 3.0);
        fill_rounded(snapshot, bar, 1.5, &with_alpha(&foreground, alpha));
    };
    match page {
        Page::Note => {
            bar(rect.y() + pad, 0.6, 0.5);
            let mut y = rect.y() + pad + 10.0;
            for share in [1.0, 0.85, 0.95, 0.5, 0.9, 0.7] {
                bar(y, share, 0.18);
                y += 7.0;
            }
        }
        Page::File => {
            let label = layout(widget, "PDF", None);
            let (label_width, label_height) = label.pixel_size();
            let (center_x, center_y) = (
                rect.x() + rect.width() / 2.0,
                rect.y() + rect.height() / 2.0,
            );
            let badge = graphene::Rect::new(
                center_x - label_width as f32 / 2.0 - 6.0,
                center_y - label_height as f32 / 2.0 - 2.0,
                label_width as f32 + 12.0,
                label_height as f32 + 4.0,
            );
            fill_rounded(snapshot, badge, 4.0, color);
            append_layout(
                snapshot,
                &label,
                center_x,
                center_y,
                0.5,
                &gtk::gdk::RGBA::WHITE,
            );
        }
    }
}
