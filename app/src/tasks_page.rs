use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use knotbook_core::Vault;

use crate::task_list_view::TaskListView;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/tasks_page.ui")]
    pub struct TasksPage {
        #[template_child]
        pub list: TemplateChild<TaskListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TasksPage {
        const NAME: &'static str = "KnotbookTasksPage";
        type Type = super::TasksPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            TaskListView::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for TasksPage {}
    impl WidgetImpl for TasksPage {}
    impl NavigationPageImpl for TasksPage {}
}

glib::wrapper! {
    /// All tasks of the vault, to add, edit, sort and archive.
    pub struct TasksPage(ObjectSubclass<imp::TasksPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TasksPage {
    /// Shows the tasks of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().list.set_vault(vault);
    }

    /// Reads the task list and shows it.
    pub fn reload(&self) {
        self.imp().list.reload();
    }
}
