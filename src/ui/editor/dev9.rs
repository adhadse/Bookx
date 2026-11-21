//! Dev hooks of the editor human-interface wave 9 (`editor:<command>`): they print what a scenario of
//! build-aux/ui-tests-editor8.sh checks (`editor_header:`, `editor_menu:`, `editor_layout:` ...).

use super::*;

/// Every widget below `w` (itself included) that passes `f`.
fn find_all(w: &gtk::Widget, f: &dyn Fn(&gtk::Widget) -> bool, out: &mut Vec<gtk::Widget>) {
    if f(w) {
        out.push(w.clone());
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        find_all(&x, f, out);
        c = x.next_sibling();
    }
}

/// The first widget below `w` with that widget name.
pub(super) fn find_named(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut v = Vec::new();
    find_all(w, &|x| x.widget_name() == name, &mut v);
    v.into_iter().next()
}

/// The labels of `menu` and of every submenu, depth first.
fn all_labels(menu: &gio::MenuModel, out: &mut Vec<String>) {
    for i in 0..menu.n_items() {
        if let Some(v) = menu.item_attribute_value(i, "label", Some(glib::VariantTy::STRING)) {
            out.push(v.str().unwrap_or_default().to_string());
        }
        if let Some(s) = menu.item_link(i, "section") {
            all_labels(&s, out);
        }
        if let Some(s) = menu.item_link(i, "submenu") {
            all_labels(&s, out);
        }
    }
}

fn duplicates(labels: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut dup = Vec::new();
    for l in labels {
        if !seen.insert(l.clone()) && !dup.contains(l) {
            dup.push(l.clone());
        }
    }
    dup
}

fn collect_model_buttons(w: &gtk::Widget, out: &mut Vec<(String, bool)>) {
    if w.type_().name() == "GtkModelButton" {
        let text: String = w.property("text");
        out.push((text, w.is_sensitive()));
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        collect_model_buttons(&x, out);
        c = x.next_sibling();
    }
}

impl Editor {
    /// The header's Main Menu button.
    fn main_menu_button(&self) -> Option<gtk::MenuButton> {
        let header = self.w.title.ancestor(adw::HeaderBar::static_type())?;
        let mut stack = vec![header];
        while let Some(w) = stack.pop() {
            if let Some(b) = w.downcast_ref::<gtk::MenuButton>() {
                if b.is_primary() {
                    return Some(b.clone());
                }
            }
            let mut c = w.first_child();
            while let Some(x) = c {
                stack.push(x.clone());
                c = x.next_sibling();
            }
        }
        None
    }

    fn print_menu(&self, name: &str, menu: &gio::MenuModel) {
        let top = window_features::top_level_labels(menu);
        let mut all = Vec::new();
        all_labels(menu, &mut all);
        let dup = duplicates(&all);
        eprintln!(
            "editor_menu: name={name} top={} labels={} dup={}",
            top.len(),
            top.join("|"),
            if dup.is_empty() {
                "-".to_string()
            } else {
                dup.join("|")
            }
        );
    }

    /// Returns true when `c` was one of the hooks of this file.
    pub(super) fn dev9(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        match c {
            "header-info" => {
                // `editor_header: buttons=N inside=yes|no collapsed=… win=WxH sidebar_btn=… preview_tip=…`
                let Some(header) = self.w.title.ancestor(adw::HeaderBar::static_type()) else {
                    return true;
                };
                let (ww, wh) = (self.w.win.width(), self.w.win.height());
                let mut btns = Vec::new();
                find_all(
                    &header,
                    &|x| {
                        (x.is::<gtk::MenuButton>() || x.is::<gtk::Button>())
                            && x.is_mapped()
                            && x.ancestor(gtk::WindowControls::static_type()).is_none()
                            // a MenuButton holds a toggle button of its own: count the outer widget only
                            && x.parent().is_none_or(|p| p.ancestor(gtk::MenuButton::static_type()).is_none())
                    },
                    &mut btns,
                );
                let inside = btns.iter().all(|b| {
                    b.compute_bounds(&self.w.win)
                        .is_some_and(|r| r.x() >= -1.0 && r.x() + r.width() <= ww as f32 + 1.0)
                });
                eprintln!(
                    "editor_header: buttons={} inside={} win={ww}x{wh} sidebar_btn={} preview_tip={:?}",
                    btns.len(),
                    if inside { "yes" } else { "no" },
                    self.w.sidebar_btn.is_mapped(),
                    self.w.preview_btn.tooltip_text().unwrap_or_default()
                );
                true
            }
            "tools-menu" => {
                if let Some((_, m)) = self.w.menus.iter().find(|(n, _)| n == "Tools") {
                    self.print_menu("tools", m);
                }
                true
            }
            "book-menu" => {
                self.print_menu("book", window_features::book_menu().upcast_ref());
                true
            }
            "book-menu-state" => {
                // `editor_book_menu: Preferences=on Keyboard Shortcuts=on … all_app=yes|no`: opens the real Main Menu
                // popover and reads the sensitivity of each entry, which is what the user sees greyed or not
                let Some(btn) = self.main_menu_button() else {
                    eprintln!("editor_book_menu: no main menu button");
                    return true;
                };
                btn.popup();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    let mut entries: Vec<(String, bool)> = Vec::new();
                    if let Some(pop) = btn.popover() {
                        collect_model_buttons(pop.upcast_ref(), &mut entries);
                    }
                    btn.popdown();
                    let on = |l: &str| entries.iter().any(|(n, e)| n == l && *e);
                    let all = [
                        "Preferences",
                        "Keyboard Shortcuts",
                        "Help",
                        "About Bookx",
                        "Command Palette",
                    ]
                    .iter()
                    .all(|l| on(l));
                    eprintln!(
                        "editor_book_menu: {} all_app={}",
                        entries
                            .iter()
                            .map(|(n, e)| format!("{n}={}", if *e { "on" } else { "off" }))
                            .collect::<Vec<_>>()
                            .join(" | "),
                        if all { "yes" } else { "no" }
                    );
                });
                true
            }
            "book-menu-activate=Preferences"
            | "book-menu-activate=Keyboard Shortcuts"
            | "book-menu-activate=About Bookx"
            | "book-menu-activate=Command Palette" => {
                // runs the action of the entry from the editor window and reports whether a dialog opened over it
                let label = c.trim_start_matches("book-menu-activate=").to_string();
                let (action, target) = match label.as_str() {
                    "Preferences" => ("app.preferences", None),
                    "Keyboard Shortcuts" => ("app.shortcuts", None),
                    "About Bookx" => ("app.about", None),
                    _ => ("ext.win", Some("palette".to_variant())),
                };
                let ok =
                    gtk::prelude::WidgetExt::activate_action(&self.w.win, action, target.as_ref())
                        .is_ok();
                let win = self.w.win.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                    let d = win.visible_dialog();
                    eprintln!(
                        "editor_book_menu_activate: {label} activated={ok} dialog_over_editor={} dialog={}",
                        d.is_some(),
                        d.as_ref().map(|d| d.type_().name()).unwrap_or("-")
                    );
                    if let Some(d) = d {
                        d.close();
                    }
                });
                true
            }
            "revert-state" => {
                let state = |id: &str| {
                    self.ext
                        .group
                        .borrow()
                        .as_ref()
                        .and_then(|g| g.lookup_action(id))
                        .map(|a| a.is_enabled())
                        .unwrap_or(false)
                };
                eprintln!(
                    "editor_revert: undo={} redo={}",
                    state("undo-checkpoint"),
                    state("redo-checkpoint")
                );
                true
            }
            _ => self.dev9_more(c, sender),
        }
    }

    /// The dialog on top of the editor window, if any.
    fn top_dialog(&self) -> Option<adw::Dialog> {
        let mut v = Vec::new();
        find_all(
            self.w.win.upcast_ref::<gtk::Widget>(),
            &|x| x.is::<adw::Dialog>(),
            &mut v,
        );
        v.pop().and_downcast::<adw::Dialog>()
    }

    fn dev9_more(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        let root = self.w.win.clone().upcast::<gtk::Widget>();
        if let Some(r) = c.strip_prefix("resize=") {
            if let Some((a, b)) = r.split_once('x') {
                self.w
                    .win
                    .set_default_size(a.parse().unwrap_or(900), b.parse().unwrap_or(600));
            }
            return true;
        }
        match c {
            "layout" => {
                // `editor_layout: win=WxH code_w=N collapsed=… minimap=… preview_w=N vertical=… sidebar_btn=…`
                eprintln!(
                    "editor_layout: win={}x{} code_w={} collapsed={} minimap={} preview_w={} vertical={} sidebar_btn={}",
                    self.w.win.width(),
                    self.w.win.height(),
                    self.w.center.width(),
                    self.w.split.is_collapsed(),
                    self.w.map.is_mapped(),
                    self.w.preview_pane.width(),
                    self.w.paned.orientation() == gtk::Orientation::Vertical,
                    self.w.sidebar_btn.is_mapped(),
                );
                // the buttons of the preview header (the scenario wants four at most)
                let mut pb = Vec::new();
                find_all(
                    self.w.preview_pane.upcast_ref::<gtk::Widget>(),
                    &|x| {
                        (x.is::<gtk::MenuButton>() || x.is::<gtk::Button>())
                            && x.is_visible()
                            && !x.parent().is_some_and(|p| {
                                p.ancestor(gtk::MenuButton::static_type()).is_some()
                            })
                            && x.ancestor(gtk::Popover::static_type()).is_none()
                            && x.ancestor(gtk::Revealer::static_type()).is_none()
                    },
                    &mut pb,
                );
                eprintln!(
                    "editor_preview_head: buttons={} preview_visible={}",
                    pb.len(),
                    self.w.preview_pane.is_visible()
                );
            }
            "find-open" => sender.input(Input::ToggleFind),
            "replace-open" => sender.input(Input::ToggleReplace),
            "find-info" => {
                eprintln!(
                    "editor_find: height={} open={} replace={}",
                    self.w.find_rev.height(),
                    self.w.find_rev.reveals_child(),
                    self.w.replace_rev.reveals_child()
                );
            }
            "switcher-info" => {
                // titles that are ellipsized in the sidebar switcher (none: it shows icons)
                let mut labels = Vec::new();
                find_all(
                    &root,
                    &|x| {
                        x.is::<gtk::Label>()
                            && x.is_mapped()
                            && x.ancestor(adw::ViewSwitcher::static_type()).is_some()
                    },
                    &mut labels,
                );
                let cut = labels
                    .iter()
                    .filter_map(|l| l.downcast_ref::<gtk::Label>())
                    .filter(|l| l.layout().is_ellipsized())
                    .count();
                eprintln!("editor_switcher: labels={} ellipsized={cut}", labels.len());
            }
            "banner-info" => eprintln!(
                "editor_banner: revealed={} title={:?} button={}",
                self.w.banner.is_revealed(),
                self.w.banner.title(),
                self.w
                    .banner
                    .button_label()
                    .map_or("-".to_string(), |l| l.to_string())
            ),
            "banner-action" => {
                if self.w.banner.is_revealed() && self.w.banner.button_label().is_some() {
                    self.w.banner.emit_by_name::<()>("button-clicked", &[]);
                }
            }
            "foreign-banner" => self.show_foreign_banner(),
            "file-labels" => {
                // `editor_file_labels: <label>=<tooltip>|...` of the file list rows
                self.rebuild_files();
                let mut rows = Vec::new();
                find_all(
                    self.w.files.upcast_ref::<gtk::Widget>(),
                    &|x| x.is::<gtk::Label>() && x.is_mapped() && x.has_css_class("file-name"),
                    &mut rows,
                );
                let out: Vec<String> = rows
                    .iter()
                    .filter_map(|l| l.downcast_ref::<gtk::Label>())
                    .map(|l| format!("{}={}", l.text(), l.tooltip_text().unwrap_or_default()))
                    .collect();
                eprintln!("editor_file_labels: {}", out.join("|"));
            }
            _ => {
                if let Some(why) = c.strip_prefix("save-failed=") {
                    self.save_failed(why);
                } else if c == "fonts-state" {
                    let dialog = self.ext.structure.fonts_dialog.borrow().clone();
                    let in_dialog = |n: &str| {
                        dialog
                            .as_ref()
                            .and_then(|d| d.child())
                            .and_then(|w| find_named(&w, n))
                            .is_some()
                    };
                    eprintln!(
                        "editor_fonts: empty_page={} sample_group={}",
                        in_dialog("fonts-empty"),
                        in_dialog("fonts-sample")
                    );
                } else if c == "dialog-close" {
                    if let Some(d) = self.top_dialog() {
                        d.force_close();
                    }
                } else if c == "dialog-info" {
                    self.print_dialog_info();
                } else if c == "dialog-buttons" {
                    let Some(d) = self.top_dialog() else {
                        return true;
                    };
                    let mut b = Vec::new();
                    find_all(
                        d.upcast_ref::<gtk::Widget>(),
                        &|x| x.is::<gtk::Button>() && x.is_visible() && !x.is::<gtk::MenuButton>(),
                        &mut b,
                    );
                    let names: Vec<String> = b
                        .iter()
                        .filter_map(|x| x.downcast_ref::<gtk::Button>())
                        .filter_map(|x| x.label())
                        .map(|l| l.to_string())
                        .collect();
                    eprintln!("editor_dialog_buttons: {}", names.join("|"));
                } else if let Some(r) = c.strip_prefix("suff-toggle=") {
                    // `suff-toggle=<set>:<mode>` flips a box of the accessibility form; `suff-text` prints the text
                    if let Some((n, m)) = r.split_once(':') {
                        let found = self
                            .top_dialog()
                            .and_then(|d| find_named(d.upcast_ref(), &format!("suff-{n}-{m}")))
                            .and_downcast::<gtk::CheckButton>();
                        match found {
                            Some(b) => b.set_active(!b.is_active()),
                            None => eprintln!("editor_suff: missing {n}-{m}"),
                        }
                    }
                } else if c == "suff-add" {
                    if let Some(b) = self
                        .top_dialog()
                        .and_then(|d| {
                            let mut v = Vec::new();
                            find_all(
                                d.upcast_ref(),
                                &|x| {
                                    x.downcast_ref::<gtk::Button>().is_some_and(|b| {
                                        b.label().as_deref() == Some(&tr("Add Alternative Set"))
                                    })
                                },
                                &mut v,
                            );
                            v.pop()
                        })
                        .and_downcast::<gtk::Button>()
                    {
                        b.emit_clicked();
                    }
                } else if c == "suff-text" {
                    let t = self
                        .top_dialog()
                        .and_then(|d| find_named(d.upcast_ref(), "suff-entry"))
                        .and_downcast::<adw::EntryRow>()
                        .map(|e| e.text().to_string())
                        .unwrap_or_default();
                    let mut v = Vec::new();
                    if let Some(d) = self.top_dialog() {
                        find_all(
                            d.upcast_ref(),
                            &|x| x.widget_name().starts_with("suff-") && x.is::<gtk::CheckButton>(),
                            &mut v,
                        );
                    }
                    eprintln!("editor_suff: text={t:?} boxes={}", v.len());
                } else if c == "links-ui" {
                    let d = self.top_dialog();
                    let find = |n: &str| d.as_ref().and_then(|d| find_named(d.upcast_ref(), n));
                    eprintln!(
                        "editor_links: progress={} cancel={}",
                        find("links-progress").is_some_and(|x| x.is_visible()),
                        find("links-cancel").is_some_and(|x| x.is_visible())
                    );
                } else if c == "links-cancel" {
                    if let Some(b) = self
                        .top_dialog()
                        .and_then(|d| find_named(d.upcast_ref(), "links-cancel"))
                        .and_downcast::<gtk::Button>()
                    {
                        b.emit_clicked();
                    }
                } else if c == "narration-ui" {
                    let d = self.top_dialog();
                    let find = |n: &str| d.as_ref().and_then(|d| find_named(d.upcast_ref(), n));
                    eprintln!(
                        "editor_narration: dialog={} background={} cancel={}",
                        d.is_some(),
                        find("narration-background").is_some(),
                        find("narration-cancel").is_some()
                    );
                } else if c == "narration-background" {
                    if let Some(b) = self
                        .top_dialog()
                        .and_then(|d| find_named(d.upcast_ref(), "narration-background"))
                        .and_downcast::<gtk::Button>()
                    {
                        b.emit_clicked();
                    }
                } else {
                    return false;
                }
            }
        }
        true
    }

    /// `editor_dialog: card_open=… first_row=<title|-> visible=yes|no ellipsized=N` for the top dialog.
    fn print_dialog_info(&self) {
        let Some(d) = self.top_dialog() else {
            eprintln!("editor_dialog: none");
            return;
        };
        // a dialog that was just presented is laid out a frame or two later; labels measured before that are
        // allocated no width and look cut off
        let ctx = glib::MainContext::default();
        let start = std::time::Instant::now();
        while d.child().is_none_or(|c| c.width() <= 0 || c.height() <= 0)
            && start.elapsed() < std::time::Duration::from_secs(3)
        {
            ctx.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let w: gtk::Widget = d.clone().upcast();
        let card = find_named(&w, "tool-change-card")
            .and_downcast::<gtk::Revealer>()
            .is_some_and(|r| r.reveals_child());
        let mut scrolls = Vec::new();
        find_all(
            &w,
            &|x| x.is::<gtk::ScrolledWindow>() && x.is_visible(),
            &mut scrolls,
        );
        let scroll = scrolls.first().cloned();
        let mut rows = Vec::new();
        find_all(
            &w,
            &|x| x.is::<adw::PreferencesRow>() && x.is_visible() && x.height() > 8,
            &mut rows,
        );
        let first = rows.first();
        let visible = match (first, &scroll) {
            (Some(r), Some(s)) => r
                .compute_bounds(s)
                .is_some_and(|b| b.y() >= 0.0 && b.y() + b.height() <= s.height() as f32 + 1.0),
            _ => false,
        };
        let mut labels = Vec::new();
        find_all(
            &w,
            &|x| {
                x.is::<gtk::Label>()
                    && x.is_visible()
                    && x.ancestor(adw::ComboRow::static_type()).is_some()
            },
            &mut labels,
        );
        let cut_labels: Vec<String> = labels
            .iter()
            .filter_map(|l| l.downcast_ref::<gtk::Label>())
            .filter(|l| l.layout().is_ellipsized())
            .map(|l| l.text().to_string())
            .collect();
        let cut = cut_labels.len();
        if cut > 0 {
            eprintln!("editor_dialog_debug: ellipsized={cut_labels:?}");
        }
        if let (Some(r), Some(sc)) = (first, &scroll) {
            eprintln!(
                "editor_dialog_debug: row_bounds={:?} scroll={}x{} scrolls={}",
                r.compute_bounds(sc).map(|b| (b.y(), b.height())),
                sc.width(),
                sc.height(),
                scrolls.len()
            );
        }
        eprintln!(
            "editor_dialog: card_open={card} first_row={} visible={} ellipsized={cut} rows={}",
            first
                .and_then(|r| r.downcast_ref::<adw::PreferencesRow>())
                .map_or("-".to_string(), |r| r.title().to_string()),
            if visible { "yes" } else { "no" },
            rows.len()
        );
    }
}
