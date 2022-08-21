use crate::application::{Action, BookxApplication};
use crate::config;
// TODO: database section
// TODO: BookxLibraryContentBox
// use crate::database::{SwLibrary, SwLibraryStatus};
// use crate::ui::BookxLibraryContentBox;

use adw::subclass::prelude::*;
use glib::{clone, subclass, Sender};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use once_cell::unsync::OnceCell;

mod imp {
    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/com/anuragdhadse/Bookx/ui/library.ui")]
    pub struct BookxLibraryPage {
        #[template_child]
        pub library_status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub library_stack: TemplateChild<gtk::Stack>,
        // TODO: uncomment this
        // #[template_child]
        // pub content_box: TemplateChild<BookxLibraryContentBox>,

        // pub library: BookxLibrary,
        pub sender: OnceCell<Sender<Action>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BookxLibraryPage {
        const NAME: &'static str = "BookxLibraryPage";
        type ParentType = adw::Bin;
        type Type = super::BookxLibraryPage;

        fn new() -> Self {
            let library_status_page = TemplateChild::default();
            let library_stack = TemplateChild::default();
            // TODO: uncomment this
            // let content_box = TemplateChild::default();

            let app = gio::Application::default()
                .unwrap()
                .downcast::<BookxApplication>()
                .unwrap();
            // let library = app.library();

            let sender = OnceCell::default();

            Self {
                library_status_page,
                library_stack,
                // content_box,
                // library,
                sender,
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BookxLibraryPage {}
    impl WidgetImpl for BookxLibraryPage {}
    impl BinImpl for BookxLibraryPage {}
}

glib::wrapper! {
    pub struct BookxLibraryPage(ObjectSubclass<imp::BookxLibraryPage>)
        @extends gtk::Widget, adw::Bin;
}

impl BookxLibraryPage {
    pub fn init(&self, sender: Sender<Action>) {
        self.imp().sender.set(sender).unwrap();

        self.setup_widgets();
        // self.setup_signals();
    }

    // pub fn set_sorting(&self, sorting: BookxSorting, descending: bool) {
    //     self.imp().content_box.get().set_sorting(sorting, descending);
    // }

    fn setup_widgets(&self) {
        let imp = self.imp();

        // Setup empty status page
        imp.library_status_page.set_icon_name(Some(config::APP_ID));

        // TODO: this isn't supposed to be here
        imp.library_stack.set_visible_child_name("empty");

        // Library content box
        // imp.content_box.
        //     init(imp.library.model(), imp.sender.get().unwrap().clone());

        // TODO: uncomment this
        // self.update_stack_page();
    }

    // fn setup_signal(&self) {
    //     self.imp().library.connect_notify_local(
    //         Some("status"),
    //         clone!(@weak self as this => move |_, _|this.update_stack_page()),
    //     )
    // }

    // fn update_stack_page(&self) {
    //     let imp = self.imp();

    //     match imp.library.status() {
    //         BookxLibraryStatus::Loading => imp.library_stack.set_visible_child_nae("loading"),
    //         BookxLibraryStatus::Empty => imp.library_stack.set_visible_child("empty"),
    //         BookxLibraryStatus::Content => imp.library_stack.set_visible_child("content"),
    //         _ => (),
    //     }
    // }
}
