//! The two notices at the top of the library: the one-time beta notice (the first start of each beta
//! version) and "Bookx closed unexpectedly last time". Both are dismissible, non-modal, offer
//! Report a Problem, and never send anything.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::report_dialog::Prefill;
use super::SharedRef;
use crate::i18n::tr;

struct Notice {
    revealer: gtk::Revealer,
    report: gtk::Button,
    dismiss: gtk::Button,
    wanted: std::cell::Cell<bool>,
}

impl Notice {
    fn new(text: &str, name: &str) -> Rc<Notice> {
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let report = gtk::Button::with_label(&tr("Report a Problem"));
        let dismiss = gtk::Button::with_label(&tr("Dismiss"));
        super::audit::label(&report, &tr("Report a Problem"));
        super::audit::label(&dismiss, &tr("Dismiss"));
        let buttons = adw::WrapBox::new();
        buttons.set_child_spacing(8);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&dismiss);
        buttons.append(&report);
        report.add_css_class("suggested-action");
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.add_css_class("report-notice");
        inner.append(&label);
        inner.append(&buttons);
        inner.set_accessible_role(gtk::AccessibleRole::Group);
        super::audit::label(&inner, name);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&inner));
        revealer.set_reveal_child(false);
        Rc::new(Notice {
            revealer,
            report,
            dismiss,
            wanted: std::cell::Cell::new(false),
        })
    }
}

pub struct Notices {
    shared: SharedRef,
    beta: Rc<Notice>,
    crash: Rc<Notice>,
    nav: adw::NavigationView,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Notices>>> = const { RefCell::new(None) };
}

impl Notices {
    fn at_root(&self) -> bool {
        self.nav.navigation_stack().n_items() <= 1
    }

    fn apply(&self) {
        let root = self.at_root();
        self.beta
            .revealer
            .set_reveal_child(self.beta.wanted.get() && root);
        self.crash
            .revealer
            .set_reveal_child(self.crash.wanted.get() && root);
    }

    /// Looks at the settings and the crash files again.
    pub fn recheck(&self) {
        let version = crate::beta::version();
        let seen = self.shared.cfg.borrow().beta_notice_seen.clone();
        let dev = std::env::var_os("BOOKX_DEV_SCRIPT").is_some();
        let tv = std::env::var_os("BOOKX_TEST_VERSION").is_some();
        self.beta
            .wanted
            .set(crate::beta::notice_allowed(dev, tv) && crate::beta::notice_due(&version, &seen));
        self.crash
            .wanted
            .set(crate::applog::file().is_some_and(|f| f.unseen_crash().is_some()));
        self.apply();
    }

    fn dismiss_beta(&self) {
        self.beta.wanted.set(false);
        {
            let mut c = self.shared.cfg.borrow_mut();
            c.beta_notice_seen = crate::beta::version();
            c.save();
        }
        self.apply();
    }

    fn dismiss_crash(&self) {
        if let Some(f) = crate::applog::file() {
            f.dismiss_crashes();
        }
        self.crash.wanted.set(false);
        self.apply();
    }
}

/// The notices above `nav`: returns the box that holds them and the navigation view, to be put
/// where the navigation view was. `open` shows the dialog.
pub fn install(
    shared: &SharedRef,
    nav: &adw::NavigationView,
    open: Rc<dyn Fn(Prefill)>,
) -> gtk::Box {
    let beta = Notice::new(
        &tr("Thanks for trying the Bookx beta. It can have bugs, so keep a backup of anything you care about. Reporting problems helps a lot."),
        &tr("Beta notice"),
    );
    let crash = Notice::new(
        &tr("Bookx closed unexpectedly last time. Report the problem?"),
        &tr("Crash notice"),
    );
    let n = Rc::new(Notices {
        shared: shared.clone(),
        beta,
        crash,
        nav: nav.clone(),
    });
    {
        let (nn, open_beta) = (n.clone(), open.clone());
        n.beta.report.connect_clicked(move |_| {
            nn.dismiss_beta();
            open_beta(Prefill::None);
        });
        let nn = n.clone();
        n.beta.dismiss.connect_clicked(move |_| nn.dismiss_beta());
        let (nn, open) = (n.clone(), open);
        n.crash.report.connect_clicked(move |_| {
            // the dialog still finds the crash text: the file is only marked as seen
            nn.dismiss_crash();
            open(Prefill::Crash);
        });
        let nn = n.clone();
        n.crash.dismiss.connect_clicked(move |_| nn.dismiss_crash());
        let nn = n.clone();
        nav.connect_visible_page_notify(move |_| nn.apply());
    }
    n.recheck();
    CURRENT.with(|c| *c.borrow_mut() = Some(n.clone()));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.append(&n.crash.revealer);
    b.append(&n.beta.revealer);
    nav.set_vexpand(true);
    b.append(nav);
    b
}

/// `report:<command>` of a dev script that is about the notices.
pub fn dev_command(arg: &str) -> bool {
    let Some(n) = CURRENT.with(|c| c.borrow().clone()) else {
        return false;
    };
    match arg {
        "notices" => eprintln!(
            "report_notices: beta={} crash={} seen={:?} due={}",
            n.beta.revealer.reveals_child(),
            n.crash.revealer.reveals_child(),
            n.shared.cfg.borrow().beta_notice_seen,
            crate::beta::notice_due(
                &crate::beta::version(),
                &n.shared.cfg.borrow().beta_notice_seen
            )
        ),
        "recheck" => {
            n.recheck();
            eprintln!("report_recheck: ok");
        }
        "dismiss-beta" => n.beta.dismiss.emit_clicked(),
        "dismiss-crash" => n.crash.dismiss.emit_clicked(),
        "report-from-beta" => n.beta.report.emit_clicked(),
        "report-from-crash" => n.crash.report.emit_clicked(),
        _ => return false,
    }
    true
}
