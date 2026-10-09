//! The figures atop the pages of the help: parts of the app drawn as plain
//! examples, in its style but simpler. Drawn rather than pictures, so that
//! they take the colours of the theme, their words are translated and
//! they do not age with the app.

mod day;
mod markdown;
mod projects;
mod tasks;
mod week;

use std::cell::Cell;

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};

use crate::colors::with_alpha;
use crate::week_chart::append_layout;

/// What a figure shows.
#[derive(Debug, Default, Clone, Copy)]
pub enum Kind {
    #[default]
    Day,
    Projects,
    Tasks,
    Week,
    Markdown,
}

impl Kind {
    fn height(self) -> f32 {
        match self {
            Self::Day => day::HEIGHT,
            Self::Projects => projects::HEIGHT,
            Self::Tasks => tasks::HEIGHT,
            Self::Week => week::HEIGHT,
            Self::Markdown => markdown::HEIGHT,
        }
    }

    /// What it shows, for screen readers.
    fn description(self) -> String {
        match self {
            Self::Day => gettext(
                "A morning with blocks of work and overhead, a gap without a block and a break; dots show text that doesn’t fit",
            ),
            Self::Projects => {
                gettext("The activity of a project over 30 days, with two of its notes and a file")
            }
            Self::Tasks => gettext("A task list with an overdue task and a finished one"),
            Self::Week => gettext("A week of working time per day and project, with the target"),
            Self::Markdown => gettext(
                "A note formatted as it is typed, with the markers shown only where the cursor is",
            ),
        }
    }
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Figure {
        pub kind: Cell<Kind>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Figure {
        const NAME: &'static str = "BitLogHelpFigure";
        type Type = super::Figure;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Figure {}

    impl WidgetImpl for Figure {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (minimum, natural) = match orientation {
                gtk::Orientation::Horizontal => (240, 420),
                _ => {
                    let height = self.kind.get().height().ceil() as i32;
                    (height, height)
                }
            };
            (minimum, natural, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let widget = widget.upcast_ref();
            match self.kind.get() {
                Kind::Day => day::snapshot(widget, snapshot),
                Kind::Projects => projects::snapshot(widget, snapshot),
                Kind::Tasks => tasks::snapshot(widget, snapshot),
                Kind::Week => week::snapshot(widget, snapshot),
                Kind::Markdown => markdown::snapshot(widget, snapshot),
            }
        }
    }
}

glib::wrapper! {
    /// A part of the app drawn as an example.
    pub struct Figure(ObjectSubclass<imp::Figure>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Figure {
    pub fn new(kind: Kind) -> Self {
        let figure: Self = glib::Object::new();
        figure.imp().kind.set(kind);
        figure.update_property(&[gtk::accessible::Property::Label(&kind.description())]);
        figure
    }
}

/// `text` in the font of `widget`, cut off with "…" beyond `width`, if
/// given.
fn layout(widget: &gtk::Widget, text: &str, width: Option<f32>) -> pango::Layout {
    let layout = widget.create_pango_layout(Some(text));
    if let Some(width) = width {
        layout.set_width((width.max(0.0) * pango::SCALE as f32) as i32);
        layout.set_ellipsize(pango::EllipsizeMode::End);
    }
    layout
}

/// Like `layout`, but `markup` in Pango markup.
fn markup_layout(widget: &gtk::Widget, markup: &str, width: Option<f32>) -> pango::Layout {
    let layout = layout(widget, "", width);
    layout.set_markup(markup);
    layout
}

fn fill_rounded(snapshot: &gtk::Snapshot, rect: graphene::Rect, radius: f32, color: &gdk::RGBA) {
    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, radius));
    snapshot.append_color(color, &rect);
    snapshot.pop();
}

/// A check box as GTK draws it: an outline, or filled in the accent colour
/// with a tick.
fn append_check_box(
    snapshot: &gtk::Snapshot,
    x: f32,
    y: f32,
    checked: bool,
    foreground: &gdk::RGBA,
) {
    const SIZE: f32 = 16.0;
    let rect = graphene::Rect::new(x, y - SIZE / 2.0, SIZE, SIZE);
    let rounded = gsk::RoundedRect::from_rect(rect, 4.0);
    if checked {
        let accent = adw::StyleManager::default().accent_color_rgba();
        fill_rounded(snapshot, rect, 4.0, &accent);
        let tick = gsk::PathBuilder::new();
        tick.move_to(x + 4.0, y);
        tick.line_to(x + 7.0, y + 3.0);
        tick.line_to(x + 12.0, y - 3.5);
        snapshot.append_stroke(&tick.to_path(), &gsk::Stroke::new(2.0), &gdk::RGBA::WHITE);
    } else {
        let color = with_alpha(foreground, 0.4);
        snapshot.append_border(&rounded, &[2.0; 4], &[color; 4]);
    }
}
