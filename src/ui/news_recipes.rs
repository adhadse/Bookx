//! The recipe editor of the *News Subscriptions* window (roadmap S4) and the site accounts for
//! paywalled news.
//!
//! * **Edit Feeds** (press a subscription): one expandable row per feed (section title, address,
//!   full articles or the feed text, own article limit, on or off), the filters (patterns for titles
//!   and addresses, CSS selectors to keep or remove), language, title of the issue and images. The
//!   schedule stays with *Edit* of the subscription. Logic and tests: `crate::news_recipe_form`.
//! * **Site accounts**: a user name and password for a site that needs a login, sent as HTTP Basic
//!   authentication to that exact host over https only; the password is a secret field (keyring), see
//!   `crate::news_sites` for the rules and the tests.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::news::Ctx;
use super::SharedRef;
use crate::news_recipe_form::{self as form, FeedForm, Mode, RecipeForm};
use crate::news_sites::{self as sites, SiteAccount};

thread_local! {
    static WINDOW: RefCell<Option<(Ctx, adw::Dialog)>> = const { RefCell::new(None) };
    static EDITOR: RefCell<Option<Rc<Editor>>> = const { RefCell::new(None) };
    static SITES: RefCell<Option<Rc<Sites>>> = const { RefCell::new(None) };
    static SITE_FORM: RefCell<Option<Rc<SiteForm>>> = const { RefCell::new(None) };
    static SITE_UNDO: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Remembers the News Subscriptions window for the developer hooks.
pub fn note_window(ctx: &Ctx, dialog: &adw::Dialog) {
    WINDOW.with(|w| *w.borrow_mut() = Some((ctx.clone(), dialog.clone())));
    dialog.connect_closed(|_| WINDOW.with(|w| *w.borrow_mut() = None));
}

fn text_card(title: &str, help: &str, text: &str) -> (adw::PreferencesGroup, gtk::TextView) {
    let g = adw::PreferencesGroup::new();
    g.set_title(&glib::markup_escape_text(title));
    g.set_description(Some(&glib::markup_escape_text(help)));
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(12);
    view.set_right_margin(12);
    view.buffer().set_text(text);
    super::audit::label(&view, title);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(72);
    scroll.set_child(Some(&view));
    scroll.add_css_class("card");
    g.add(&scroll);
    (g, view)
}

fn text_of(v: &gtk::TextView) -> String {
    let b = v.buffer();
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

// ---- the recipe editor ------------------------------------------------------------------------

struct FeedRow {
    row: adw::ExpanderRow,
    title: adw::EntryRow,
    url: adw::EntryRow,
    mode: adw::ComboRow,
    max: adw::SpinRow,
    on: adw::SwitchRow,
    remove: gtk::Button,
}

struct Editor {
    dialog: adw::Dialog,
    key: String,
    ctx: Ctx,
    done: Rc<dyn Fn()>,
    feeds: adw::PreferencesGroup,
    rows: RefCell<Vec<Rc<FeedRow>>>,
    include: gtk::TextView,
    exclude: gtk::TextView,
    keep: gtk::TextView,
    remove: gtk::TextView,
    language: adw::EntryRow,
    template: adw::EntryRow,
    images: adw::SwitchRow,
    banner: adw::Banner,
    save: gtk::Button,
    add: gtk::Button,
}

fn modes() -> gtk::StringList {
    gtk::StringList::new(&[
        &tr("Same as the subscription"),
        &tr("Full articles"),
        &tr("Feed text only"),
    ])
}

impl Editor {
    fn add_feed(self: &Rc<Self>, f: &FeedForm) -> Rc<FeedRow> {
        let row = adw::ExpanderRow::new();
        let title = adw::EntryRow::new();
        title.set_title(&tr("Section title"));
        title.set_text(&f.title);
        let url = adw::EntryRow::new();
        url.set_title(&tr("Address"));
        url.set_text(&f.url);
        let mode = adw::ComboRow::new();
        mode.set_title(&tr("Articles"));
        mode.set_model(Some(&modes()));
        mode.set_selected(f.mode.index());
        let max = adw::SpinRow::with_range(0.0, form::MAX_ARTICLES as f64, 1.0);
        max.set_title(&tr("Articles from this feed"));
        max.set_subtitle(&tr("0 uses the limit of the subscription"));
        max.set_value(f.max_articles as f64);
        let on = adw::SwitchRow::new();
        on.set_title(&tr("Download this feed"));
        on.set_active(f.enabled);
        for w in [
            title.upcast_ref::<gtk::Widget>(),
            url.upcast_ref(),
            mode.upcast_ref(),
            max.upcast_ref(),
            on.upcast_ref(),
        ] {
            row.add_row(w);
        }
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some(&tr("Remove this feed")));
        super::audit::label(&remove, &tr("Remove this feed"));
        row.add_suffix(&remove);
        let fr = Rc::new(FeedRow {
            row: row.clone(),
            title: title.clone(),
            url: url.clone(),
            mode,
            max,
            on,
            remove: remove.clone(),
        });
        let show = {
            let (row, title, url) = (row.clone(), title.clone(), url.clone());
            move || {
                let t = title.text();
                let u = url.text();
                row.set_title(&glib::markup_escape_text(if t.trim().is_empty() {
                    u.as_str()
                } else {
                    t.as_str()
                }));
                row.set_subtitle(&glib::markup_escape_text(u.as_str()));
            }
        };
        show();
        {
            let s = show.clone();
            title.connect_changed(move |_| s());
        }
        url.connect_changed(move |_| show());
        {
            let (ed, fr) = (self.clone(), fr.clone());
            remove.connect_clicked(move |_| {
                ed.feeds.remove(&fr.row);
                ed.rows.borrow_mut().retain(|r| !Rc::ptr_eq(r, &fr));
            });
        }
        self.feeds.add(&row);
        self.rows.borrow_mut().push(fr.clone());
        fr
    }

    fn form(&self) -> RecipeForm {
        RecipeForm {
            feeds: self
                .rows
                .borrow()
                .iter()
                .map(|r| FeedForm {
                    url: r.url.text().to_string(),
                    title: r.title.text().to_string(),
                    mode: Mode::from_index(r.mode.selected()),
                    max_articles: r.max.value() as usize,
                    enabled: r.on.is_active(),
                })
                .collect(),
            include: text_of(&self.include),
            exclude: text_of(&self.exclude),
            keep: text_of(&self.keep),
            remove: text_of(&self.remove),
            language: self.language.text().to_string(),
            title_template: self.template.text().to_string(),
            images: self.images.is_active(),
        }
    }

    fn problem(&self, text: &str) {
        self.banner.set_title(text);
        self.banner.set_revealed(true);
        LAST.with(|l| *l.borrow_mut() = text.to_string());
    }

    fn save(&self) {
        let Some(base) = self.ctx.store.borrow().get(&self.key).cloned() else {
            return self.problem(&tr("This subscription is gone."));
        };
        let result =
            form::apply(&self.form(), &base).and_then(|s| self.ctx.store.borrow_mut().update(s));
        match result {
            Ok(()) => {
                LAST.with(|l| l.borrow_mut().clear());
                self.dialog.close();
                (self.done)();
            }
            Err(e) => self.problem(&format!("{e:#}")),
        }
    }
}

/// Opens the editor for one subscription of the list. `done` redraws the list afterwards.
pub fn edit_feeds(parent: &adw::Dialog, ctx: &Ctx, key: &str, done: Rc<dyn Fn()>) {
    let Some(sub) = ctx.store.borrow().get(key).cloned() else {
        return;
    };
    let f = RecipeForm::from_subscription(&sub);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&trf("Edit Feeds of {}", &[&sub.recipe.title]));
    dialog.set_content_width(560);
    dialog.set_content_height(600);

    let page = adw::PreferencesPage::new();
    let feeds = adw::PreferencesGroup::new();
    feeds.set_title(&tr("Feeds"));
    feeds.set_description(Some(&tr(
        "Open a feed to change its section title, address and how its articles are downloaded.",
    )));
    let add = gtk::Button::with_label(&tr("Add Feed"));
    add.set_valign(gtk::Align::Center);
    feeds.set_header_suffix(Some(&add));
    page.add(&feeds);

    let (g_inc, include) = text_card(
        &tr("Keep only articles matching"),
        &tr("Patterns (regular expressions), one per line, tried on the title and the address. Empty keeps all."),
        &f.include,
    );
    let (g_exc, exclude) = text_card(
        &tr("Skip articles matching"),
        &tr("Patterns, one per line. An article that matches any of them is left out."),
        &f.exclude,
    );
    let (g_keep, keep) = text_card(
        &tr("Keep only these parts"),
        &tr("CSS selectors, one per line, for full articles. Empty keeps what Bookx finds."),
        &f.keep,
    );
    let (g_rm, remove) = text_card(
        &tr("Remove these parts"),
        &tr("CSS selectors, one per line, removed from full articles (ads, share bars)."),
        &f.remove,
    );
    for g in [&g_inc, &g_exc, &g_keep, &g_rm] {
        page.add(g);
    }

    let issue = adw::PreferencesGroup::new();
    issue.set_title(&tr("Issue"));
    let language = adw::EntryRow::new();
    language.set_title(&tr("Language code"));
    language.set_text(&f.language);
    let template = adw::EntryRow::new();
    template.set_title(&tr("Title of the issue"));
    template.set_text(&f.title_template);
    template.set_tooltip_text(Some(&tr(
        "{title} is the name of the subscription and {date} the day of the download",
    )));
    let images = adw::SwitchRow::new();
    images.set_title(&tr("Include images"));
    images.set_active(f.images);
    issue.add(&language);
    issue.add(&template);
    issue.add(&images);
    page.add(&issue);

    let when = adw::PreferencesGroup::new();
    when.set_title(&tr("Schedule"));
    when.add(&super::preferences::info_row(
        &crate::news::schedule_text(&sub, *chrono::Local::now().offset()),
        &tr("Change it with Edit in the list of subscriptions."),
    ));
    page.add(&when);

    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let banner = adw::Banner::new("");
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&banner);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&save));

    let ed = Rc::new(Editor {
        dialog: dialog.clone(),
        key: key.to_string(),
        ctx: ctx.clone(),
        done,
        feeds,
        rows: RefCell::new(Vec::new()),
        include,
        exclude,
        keep,
        remove,
        language,
        template,
        images,
        banner,
        save: save.clone(),
        add: add.clone(),
    });
    for feed in &f.feeds {
        ed.add_feed(feed);
    }
    {
        let ed = ed.clone();
        add.connect_clicked(move |_| {
            if ed.rows.borrow().len() >= form::MAX_FEEDS {
                return ed.problem(&trf("At most {} feeds.", &[&form::MAX_FEEDS]));
            }
            let r = ed.add_feed(&FeedForm::new(""));
            r.row.set_expanded(true);
            r.url.grab_focus();
        });
    }
    {
        let ed = ed.clone();
        save.connect_clicked(move |_| ed.save());
    }
    EDITOR.with(|e| *e.borrow_mut() = Some(ed));
    dialog.connect_closed(|_| EDITOR.with(|e| *e.borrow_mut() = None));
    dialog.present(Some(parent));
}

// ---- site accounts ----------------------------------------------------------------------------

struct Sites {
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    buttons: RefCell<Vec<(String, gtk::Button, gtk::Button)>>,
    shared: SharedRef,
    add: gtk::Button,
}

/// The group of the News Subscriptions window that opens the site accounts.
pub fn sites_group(parent: &adw::Dialog, shared: &SharedRef) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Sites with a login"));
    let row = adw::ActionRow::new();
    row.set_title(&tr("Site accounts"));
    row.set_subtitle(&tr(
        "A user name and password for sites that need a login. Sent only to that site.",
    ));
    row.set_subtitle_lines(0);
    let manage = gtk::Button::with_label(&tr("Manage"));
    manage.set_valign(gtk::Align::Center);
    manage.set_tooltip_text(Some(&tr("Manage the site accounts for news downloads")));
    super::audit::label(&manage, &tr("Manage site accounts"));
    row.add_suffix(&manage);
    row.set_activatable_widget(Some(&manage));
    g.add(&row);
    let (parent, shared) = (parent.clone(), shared.clone());
    manage.connect_clicked(move |_| show_sites(&parent, &shared));
    g
}

fn show_sites(parent: &adw::Dialog, shared: &SharedRef) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Site accounts"));
    group.set_description(Some(&tr(
        "Bookx signs in with HTTP Basic authentication, only to the exact site you name and only over https. Passwords are kept in your keyring, never in the settings file.",
    )));
    let add = gtk::Button::with_label(&tr("Add Site"));
    add.add_css_class("suggested-action");
    add.set_valign(gtk::Align::Center);
    group.set_header_suffix(Some(&add));
    page.add(&group);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Site accounts"));
    dialog.set_content_width(480);
    dialog.set_content_height(480);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let toasts = super::dialog_toast::overlay(&view);
    dialog.set_child(Some(&toasts));
    let s = Rc::new(Sites {
        dialog: dialog.clone(),
        toasts,
        group,
        rows: RefCell::new(Vec::new()),
        buttons: RefCell::new(Vec::new()),
        shared: shared.clone(),
        add: add.clone(),
    });
    fill_sites(&s);
    {
        let s = s.clone();
        add.connect_clicked(move |_| site_form(&s, None));
    }
    SITES.with(|c| *c.borrow_mut() = Some(s));
    dialog.connect_closed(|_| SITES.with(|c| *c.borrow_mut() = None));
    dialog.present(Some(parent));
}

fn fill_sites(s: &Rc<Sites>) {
    for r in s.rows.borrow_mut().drain(..) {
        s.group.remove(&r);
    }
    s.buttons.borrow_mut().clear();
    let list = s.shared.cfg.borrow().news_sites.clone();
    if list.is_empty() {
        let r = super::preferences::info_row(
            &tr("No site accounts yet"),
            &tr("Press Add Site to save a login for a news site."),
        );
        s.group.add(&r);
        s.rows.borrow_mut().push(r);
    }
    for a in list {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&a.host));
        row.set_subtitle(&glib::markup_escape_text(&sites::summary(&a)));
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        let rm = gtk::Button::from_icon_name("user-trash-symbolic");
        for (b, text) in [
            (&edit, trf("Edit the login for {}", &[&a.host])),
            (&rm, trf("Remove the login for {}", &[&a.host])),
        ] {
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&text));
            super::audit::label(b, &text);
            row.add_suffix(b);
        }
        {
            let (s, a) = (s.clone(), a.clone());
            edit.connect_clicked(move |_| site_form(&s, Some(a.clone())));
        }
        {
            let (s, host) = (s.clone(), a.host.clone());
            rm.connect_clicked(move |_| remove_site(&s, &host));
        }
        s.buttons.borrow_mut().push((a.host.clone(), edit, rm));
        s.group.add(&row);
        s.rows.borrow_mut().push(row);
    }
}

fn remove_site(s: &Rc<Sites>, host: &str) {
    let removed = {
        let mut c = s.shared.cfg.borrow_mut();
        c.news_sites
            .iter()
            .position(|a| a.host == host)
            .map(|i| (i, c.news_sites.remove(i)))
    };
    let Some((index, acc)) = removed else { return };
    crate::secrets::vault().forget(&sites::secret_key(host));
    s.shared.cfg.borrow().save();
    fill_sites(s);
    let stash = Rc::new(RefCell::new(Some((index, acc))));
    let undo: Rc<dyn Fn()> = {
        let (s, stash) = (s.clone(), stash.clone());
        Rc::new(move || {
            if let Some((i, a)) = stash.borrow_mut().take() {
                {
                    let mut c = s.shared.cfg.borrow_mut();
                    let i = i.min(c.news_sites.len());
                    c.news_sites.insert(i, a);
                }
                s.shared.cfg.borrow().save();
                fill_sites(&s);
            }
        })
    };
    SITE_UNDO.with(|u| *u.borrow_mut() = Some(undo.clone()));
    super::dialog_toast::show_undo(
        &s.toasts,
        &trf("Login for {} removed", &[&host]),
        move || undo(),
    );
}

struct SiteForm {
    dialog: adw::Dialog,
    host: adw::EntryRow,
    user: adw::EntryRow,
    password: adw::PasswordEntryRow,
    save: gtk::Button,
    banner: adw::Banner,
}

fn site_form(s: &Rc<Sites>, existing: Option<SiteAccount>) {
    let editing = existing.is_some();
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&if editing {
        tr("Edit Site Account")
    } else {
        tr("Add Site Account")
    });
    dialog.set_content_width(440);
    dialog.set_content_height(360);
    let g = adw::PreferencesGroup::new();
    let host = adw::EntryRow::new();
    host.set_title(&tr("Site address"));
    host.set_input_purpose(gtk::InputPurpose::Url);
    let user = adw::EntryRow::new();
    user.set_title(&tr("User name"));
    let password = adw::PasswordEntryRow::new();
    password.set_title(&if editing {
        tr("New password, empty keeps the old one")
    } else {
        tr("Password")
    });
    g.set_description(Some(&tr(
        "Type the address of the site as in the feed, for example news.example.com.",
    )));
    for w in [&host as &dyn AsRef<gtk::Widget>, &user, &password] {
        g.add(w.as_ref());
    }
    if let Some(a) = &existing {
        host.set_text(&a.host);
        host.set_editable(false);
        user.set_text(&a.user);
    }
    let page = adw::PreferencesPage::new();
    page.add(&g);
    let save = gtk::Button::with_label(&if editing { tr("Save") } else { tr("Add") });
    save.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let banner = adw::Banner::new("");
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&banner);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&save));
    let f = Rc::new(SiteForm {
        dialog: dialog.clone(),
        host,
        user,
        password,
        save: save.clone(),
        banner,
    });
    SITE_FORM.with(|c| *c.borrow_mut() = Some(f.clone()));
    {
        let (s, f) = (s.clone(), f.clone());
        let old = existing.clone();
        save.connect_clicked(move |_| submit_site(&s, &f, old.clone()));
    }
    dialog.connect_closed(|_| SITE_FORM.with(|c| *c.borrow_mut() = None));
    dialog.present(Some(&s.dialog));
}

fn submit_site(s: &Rc<Sites>, f: &Rc<SiteForm>, old: Option<SiteAccount>) {
    let list = s.shared.cfg.borrow().news_sites.clone();
    let typed = f.password.text().to_string();
    // editing without a new password keeps the saved one
    let password = if typed.is_empty() {
        old.as_ref().map(|a| a.password.clone()).unwrap_or_default()
    } else {
        typed
    };
    let editing = old.as_ref().map(|a| a.host.as_str());
    match sites::check(
        f.host.text().as_str(),
        f.user.text().as_str(),
        &password,
        &list,
        editing,
    ) {
        Err(e) => {
            f.banner.set_title(&e);
            f.banner.set_revealed(true);
            LAST.with(|l| *l.borrow_mut() = e);
        }
        Ok(acc) => {
            f.password.set_text("");
            {
                let mut c = s.shared.cfg.borrow_mut();
                match c
                    .news_sites
                    .iter()
                    .position(|a| Some(a.host.as_str()) == editing)
                {
                    Some(i) => c.news_sites[i] = acc,
                    None => c.news_sites.push(acc),
                }
            }
            s.shared.cfg.borrow().save();
            LAST.with(|l| l.borrow_mut().clear());
            f.dialog.close();
            fill_sites(s);
            super::dialog_toast::show_plain(&s.toasts, &tr("Login saved"));
        }
    }
}

// ---- developer hooks --------------------------------------------------------------------------

/// `news:recipes=…` of the developer script; arguments are separated by `;`. A password is never
/// printed.
///
/// * `open;KEY` opens the editor of a subscription, `save`, `close`
/// * `feed;N;TITLE|ADDRESS|MODE|MAX|ON` sets feed N (N = the number of feeds adds one); MODE 0 same, 1 full, 2 text
/// * `remove-feed;N`
/// * `filters;INCLUDE|EXCLUDE|KEEP|REMOVE` (`~` inside a box stands for a new line)
/// * `issue;LANGUAGE|TEMPLATE|IMAGES`
/// * `sites` opens the site accounts, `site-new`, `site-fill;HOST;USER;PASSWORD`, `site-save`,
///   `site-edit;HOST`, `site-remove;HOST`, `site-undo`
/// * `dump` prints `news_recipes: open=B feeds=N modes=… language=… error="…" ...`;
///   `sites-dump` prints `news_sites: open=B hosts=a,b users=x,y form=B last="…"`
pub fn dev(arg: &str) {
    let Some(arg) = arg.strip_prefix("recipes=") else {
        return;
    };
    let parts: Vec<&str> = arg.split(';').collect();
    let ed = EDITOR.with(|e| e.borrow().clone());
    let sites_w = SITES.with(|s| s.borrow().clone());
    let site_form_w = SITE_FORM.with(|s| s.borrow().clone());
    match parts[0] {
        "open" => {
            if let Some((ctx, dialog)) = WINDOW.with(|w| w.borrow().clone()) {
                let d2 = dialog.clone();
                let refresh = ctx.slot.clone();
                edit_feeds(
                    &d2,
                    &ctx,
                    parts.get(1).copied().unwrap_or(""),
                    Rc::new(move || {
                        if let Some(f) = refresh.borrow().clone() {
                            f();
                        }
                    }),
                );
            } else {
                eprintln!("news_recipes: no news window");
            }
        }
        "save" => {
            if let Some(e) = ed {
                e.save.emit_clicked();
            }
        }
        "close" => {
            if let Some(e) = ed {
                e.dialog.close();
            }
        }
        "feed" => {
            let Some(e) = ed else { return };
            let n: usize = parts.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
            if n == e.rows.borrow().len() {
                e.add.emit_clicked();
            }
            let row = e.rows.borrow().get(n).cloned();
            if let (Some(r), Some(v)) = (row, parts.get(2)) {
                let f: Vec<&str> = v.split('|').collect();
                r.title.set_text(f.first().copied().unwrap_or(""));
                r.url.set_text(f.get(1).copied().unwrap_or(""));
                r.mode
                    .set_selected(f.get(2).and_then(|m| m.parse().ok()).unwrap_or(0));
                r.max
                    .set_value(f.get(3).and_then(|m| m.parse().ok()).unwrap_or(0.0));
                r.on.set_active(f.get(4).is_none_or(|v| *v != "0"));
            }
        }
        "remove-feed" => {
            if let Some(e) = ed {
                let n: usize = parts.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
                let b = e.rows.borrow().get(n).map(|r| r.remove.clone());
                if let Some(b) = b {
                    b.emit_clicked();
                }
            }
        }
        "filters" => {
            if let (Some(e), Some(v)) = (ed, parts.get(1)) {
                let f: Vec<&str> = v.split('|').collect();
                for (view, i) in [
                    (&e.include, 0),
                    (&e.exclude, 1),
                    (&e.keep, 2),
                    (&e.remove, 3),
                ] {
                    view.buffer()
                        .set_text(&f.get(i).copied().unwrap_or("").replace('~', "\n"));
                }
            }
        }
        "issue" => {
            if let (Some(e), Some(v)) = (ed, parts.get(1)) {
                let f: Vec<&str> = v.split('|').collect();
                e.language.set_text(f.first().copied().unwrap_or(""));
                e.template.set_text(f.get(1).copied().unwrap_or(""));
                e.images.set_active(f.get(2).is_none_or(|v| *v != "0"));
            }
        }
        "dump" => match ed {
            Some(e) => {
                let f = e.form();
                eprintln!(
                    "news_recipes: open=true feeds={} modes={} on={} max={} language={} template={:?} images={} include={} exclude={} keep={} remove={} banner={} error={:?}",
                    f.feeds.len(),
                    f.feeds.iter().map(|x| x.mode.index().to_string()).collect::<Vec<_>>().join("+"),
                    f.feeds.iter().map(|x| u8::from(x.enabled).to_string()).collect::<Vec<_>>().join("+"),
                    f.feeds.iter().map(|x| x.max_articles.to_string()).collect::<Vec<_>>().join("+"),
                    f.language,
                    f.title_template,
                    f.images,
                    form::lines(&f.include).len(),
                    form::lines(&f.exclude).len(),
                    form::lines(&f.keep).len(),
                    form::lines(&f.remove).len(),
                    e.banner.is_revealed(),
                    LAST.with(|l| l.borrow().clone()),
                );
            }
            None => eprintln!("news_recipes: open=false"),
        },
        "sites" => {
            if let Some((ctx, dialog)) = WINDOW.with(|w| w.borrow().clone()) {
                show_sites(&dialog, &ctx.shared);
            }
        }
        "site-new" => {
            if let Some(s) = sites_w {
                s.add.emit_clicked();
            }
        }
        "site-fill" => {
            if let Some(f) = site_form_w {
                if f.host.is_editable() {
                    f.host.set_text(parts.get(1).copied().unwrap_or(""));
                }
                f.user.set_text(parts.get(2).copied().unwrap_or(""));
                f.password.set_text(parts.get(3).copied().unwrap_or(""));
            }
        }
        "site-save" => {
            if let Some(f) = site_form_w {
                f.save.emit_clicked();
            }
        }
        "site-edit" | "site-remove" => {
            if let Some(s) = sites_w {
                let host = parts.get(1).copied().unwrap_or("");
                let b = s
                    .buttons
                    .borrow()
                    .iter()
                    .find(|(h, _, _)| h == host)
                    .map(|(_, e, r)| {
                        if parts[0] == "site-edit" {
                            e.clone()
                        } else {
                            r.clone()
                        }
                    });
                if let Some(b) = b {
                    b.emit_clicked();
                }
            }
        }
        "site-undo" => {
            if let Some(u) = SITE_UNDO.with(|u| u.borrow_mut().take()) {
                u();
            }
        }
        "sites-dump" => {
            let (open, shared) = (
                sites_w.is_some(),
                sites_w.map(|s| s.shared.clone()).or_else(|| {
                    WINDOW.with(|w| w.borrow().as_ref().map(|(c, _)| c.shared.clone()))
                }),
            );
            let list = shared
                .map(|s| s.cfg.borrow().news_sites.clone())
                .unwrap_or_default();
            eprintln!(
                "news_sites: open={open} count={} hosts={} users={} form={} last={:?}",
                list.len(),
                list.iter()
                    .map(|a| a.host.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                list.iter()
                    .map(|a| a.user.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                site_form_w.is_some(),
                LAST.with(|l| l.borrow().clone()),
            );
        }
        _ => {}
    }
}
