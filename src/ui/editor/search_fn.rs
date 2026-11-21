//! The *Functions* dialog (built-in and saved replacement functions) and the Create/Edit Function dialog with a
//! source editor and a test run on a sample (the current selection by default).

use super::search_bar::SearchAct;
use super::tools_ui::{set_dialog_content, toast_in_dialog};
use super::*;
use crate::editor_search::{Opts, Pattern};
use crate::editor_search_fn::{self as sfn, Function};
use crate::i18n::{tr, trf};

struct Manager {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    user: RefCell<Vec<Function>>,
    tx: relm4::Sender<Input>,
    ctx: Ctx,
    parent: gtk::Widget,
}

/// What the edit dialog needs from the editor.
#[derive(Clone)]
struct Ctx {
    sample: String,
    opts: Opts,
    env: sfn::Env,
}

fn window_of(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_downcast::<gtk::Window>()
}

impl Editor {
    fn fn_ctx(&self) -> Ctx {
        let buf = self.w.view.buffer();
        let sample = match buf.selection_bounds() {
            Some((a, b)) => buf.text(&a, &b, false).to_string(),
            None => buffer_text(&buf).chars().take(3000).collect(),
        };
        let st = self.settings();
        Ctx {
            sample,
            opts: Opts {
                regex: true,
                case_sensitive: st.case_sensitive,
                dot_all: st.dot_all,
                whole_word: st.whole_word,
            },
            env: self.fn_env(),
        }
    }

    pub(super) fn open_functions_dialog(&mut self) {
        let m = Rc::new(Manager {
            dialog: adw::Dialog::new(),
            list: gtk::ListBox::new(),
            user: RefCell::new(self.w.sx.state.borrow().user_fns.clone()),
            tx: self.sender.input_sender().clone(),
            ctx: self.fn_ctx(),
            parent: self.w.win.clone().upcast(),
        });
        m.dialog.set_title(&tr("Replacement Functions"));
        m.dialog.set_content_width(600);
        m.dialog.set_content_height(560);
        m.list.add_css_class("boxed-list");
        m.list.set_selection_mode(gtk::SelectionMode::None);
        crate::ui::audit::label(&m.list, &tr("Functions"));
        let hint = gtk::Label::new(Some(
            "A function decides what each match of the regular expression is replaced with. Choose Use to select it in the find bar's Function mode.",
        ));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&m.list));
        let new = gtk::Button::with_label(&tr("New Function"));
        new.add_css_class("suggested-action");
        let import = gtk::Button::from_icon_name("document-open-symbolic");
        import.set_tooltip_text(Some(&tr("Import functions from a JSON file")));
        let export = gtk::Button::from_icon_name("document-save-symbolic");
        export.set_tooltip_text(Some(&tr("Export your functions to a JSON file")));
        let hb = adw::HeaderBar::new();
        hb.pack_start(&import);
        hb.pack_start(&export);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
        col.set_margin_all(12);
        col.append(&hint);
        col.append(&scroll);
        new.set_halign(gtk::Align::End);
        col.append(&new);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&hb);
        tb.set_content(Some(&col));
        set_dialog_content(&m.dialog, &tb);
        {
            let m2 = m.clone();
            new.connect_clicked(move |_| edit_fn(&m2, None));
            let m2 = m.clone();
            import.connect_clicked(move |b| {
                let fd = gtk::FileDialog::new();
                fd.set_title(&tr("Import functions"));
                let m3 = m2.clone();
                fd.open(window_of(b).as_ref(), gio::Cancellable::NONE, move |res| {
                    let Some(path) = res.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    match std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|t| sfn::parse_json(&t))
                    {
                        Ok(list) => {
                            {
                                let mut u = m3.user.borrow_mut();
                                for f in list {
                                    u.retain(|x| x.name != f.name);
                                    u.push(f);
                                }
                            }
                            changed(&m3);
                        }
                        Err(e) => toast_in_dialog(
                            &m3.dialog,
                            &trf(
                                "Could not import: that file is not a list of functions ({})",
                                &[&e],
                            ),
                        ),
                    }
                });
            });
            let m2 = m.clone();
            export.connect_clicked(move |b| {
                let fd = gtk::FileDialog::new();
                fd.set_title(&tr("Export functions"));
                fd.set_initial_name(Some("search-functions.json"));
                let m3 = m2.clone();
                fd.save(window_of(b).as_ref(), gio::Cancellable::NONE, move |res| {
                    if let Some(path) = res.ok().and_then(|f| f.path()) {
                        let _ = std::fs::write(path, sfn::to_json(&m3.user.borrow()));
                    }
                });
            });
        }
        refresh(&m);
        m.dialog.present(Some(&self.w.win));
    }

    /// Opens the editor for the function called `name` (a new one when there is none).
    pub(super) fn edit_function_dialog(&mut self, name: &str) {
        let m = Rc::new(Manager {
            dialog: adw::Dialog::new(),
            list: gtk::ListBox::new(),
            user: RefCell::new(self.w.sx.state.borrow().user_fns.clone()),
            tx: self.sender.input_sender().clone(),
            ctx: self.fn_ctx(),
            parent: self.w.win.clone().upcast(),
        });
        let f = sfn::merged(&m.user.borrow())
            .into_iter()
            .find(|f| f.name == name);
        edit_fn(&m, f);
    }
}

fn changed(m: &Rc<Manager>) {
    m.tx.emit(Input::Search(SearchAct::FunctionsChanged(
        m.user.borrow().clone(),
    )));
    refresh(m);
}

fn refresh(m: &Rc<Manager>) {
    while let Some(c) = m.list.first_child() {
        m.list.remove(&c);
    }
    for f in sfn::merged(&m.user.borrow()) {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&f.name))
            .subtitle(glib::markup_escape_text(if f.description.is_empty() {
                &f.find
            } else {
                &f.description
            }))
            .build();
        row.set_subtitle_lines(2);
        if f.builtin {
            let l = gtk::Label::new(Some(&tr("Built-in")));
            l.add_css_class("dim-label");
            row.add_prefix(&l);
        }
        let use_b = gtk::Button::with_label(&tr("Use"));
        use_b.set_valign(gtk::Align::Center);
        use_b.set_tooltip_text(Some(&tr("Select this function in the find bar")));
        let edit_b = gtk::Button::with_label(&(if f.builtin { tr("View") } else { tr("Edit") }));
        edit_b.set_valign(gtk::Align::Center);
        row.add_suffix(&use_b);
        row.add_suffix(&edit_b);
        {
            let (m2, name) = (m.clone(), f.name.clone());
            use_b.connect_clicked(move |_| {
                m2.tx
                    .emit(Input::Search(SearchAct::UseFunction(name.clone())));
                m2.dialog.close();
            });
            let (m2, f2) = (m.clone(), f.clone());
            edit_b.connect_clicked(move |_| edit_fn(&m2, Some(f2.clone())));
        }
        if !f.builtin {
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.set_valign(gtk::Align::Center);
            del.set_tooltip_text(Some(&tr("Delete this function")));
            del.add_css_class("flat");
            crate::ui::audit::label(&del, &tr("Delete function"));
            let (m2, name) = (m.clone(), f.name.clone());
            del.connect_clicked(move |_| {
                m2.user.borrow_mut().retain(|x| x.name != name);
                changed(&m2);
            });
            row.add_suffix(&del);
        }
        m.list.append(&row);
    }
}

const NEW_SOURCE: &str = "// Called for every match.\n// m.group(1), m.text, m.start, m.end; number counts matches; data keeps values for the run.\n// Return the replacement text, or () to leave the match alone.\nfn replace(m, number, file, meta, data) {\n    m.text\n}\n";

/// The Create/Edit Function dialog.
fn edit_fn(m: &Rc<Manager>, f: Option<Function>) {
    let builtin = f.as_ref().is_some_and(|f| f.builtin);
    let old_name = f.as_ref().map(|f| f.name.clone());
    let f = f.unwrap_or_else(|| Function::new("New function", "", NEW_SOURCE));
    let dialog = adw::Dialog::new();
    dialog.set_title(
        &(if builtin {
            tr("Built-in Function")
        } else {
            tr("Function")
        }),
    );
    dialog.set_content_width(720);
    dialog.set_content_height(680);

    let name = adw::EntryRow::new();
    name.set_title(&tr("Name"));
    name.set_text(&f.name);
    let find = adw::EntryRow::new();
    find.set_title(&tr("Suggested regular expression"));
    find.set_text(&f.find);
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&find);

    let buf = sv::Buffer::new(None::<&gtk::TextTagTable>);
    buf.set_text(&f.source);
    buf.set_language(sv::LanguageManager::default().language("rust").as_ref());
    codeview::apply_scheme(&buf, system_is_dark(), crate::eink::is_active());
    let view = codeview::new_view();
    view.set_buffer(Some(&buf));
    view.set_bottom_margin(8);
    crate::ui::audit::label(&view, &tr("Function source"));
    let src_scroll = gtk::ScrolledWindow::new();
    src_scroll.set_min_content_height(230);
    src_scroll.set_vexpand(true);
    src_scroll.set_child(Some(&view));
    src_scroll.add_css_class("card");

    let sample_buf = gtk::TextBuffer::new(None);
    sample_buf.set_text(&m.ctx.sample);
    let sample = gtk::TextView::with_buffer(&sample_buf);
    sample.set_monospace(true);
    sample.set_wrap_mode(gtk::WrapMode::WordChar);
    crate::ui::audit::label(&sample, &tr("Sample text to test on"));
    let sample_scroll = gtk::ScrolledWindow::new();
    sample_scroll.set_min_content_height(80);
    sample_scroll.set_child(Some(&sample));
    sample_scroll.add_css_class("card");

    let out = gtk::TextView::new();
    out.set_editable(false);
    out.set_monospace(true);
    out.set_wrap_mode(gtk::WrapMode::WordChar);
    crate::ui::audit::label(&out, &tr("Test results"));
    let out_scroll = gtk::ScrolledWindow::new();
    out_scroll.set_min_content_height(110);
    out_scroll.set_child(Some(&out));
    out_scroll.add_css_class("card");

    let test = gtk::Button::with_label(&tr("Test on Sample"));
    test.set_tooltip_text(Some(
        &tr("Run the function on the sample text with the suggested expression. Nothing in the book changes."),
    ));
    let caption = |t: &str| {
        let l = gtk::Label::new(Some(t));
        l.set_xalign(0.0);
        l.add_css_class("heading");
        l
    };

    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.set_margin_all(12);
    col.append(&group);
    col.append(&caption("Source (Rhai)"));
    col.append(&src_scroll);
    col.append(&caption("Sample text"));
    col.append(&sample_scroll);
    col.append(&test);
    col.append(&caption("Result"));
    col.append(&out_scroll);
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&(if builtin { tr("Close") } else { tr("Cancel") }));
    let hb = adw::HeaderBar::new();
    hb.set_show_end_title_buttons(false);
    hb.set_show_start_title_buttons(false);
    hb.pack_start(&cancel);
    hb.pack_end(&save);
    if builtin {
        save.set_label(&tr("Save as My Version"));
        save.set_tooltip_text(Some(&tr(
            "A saved function with the same name replaces the built-in one",
        )));
    }
    let tb = adw::ToolbarView::new();
    tb.add_top_bar(&hb);
    tb.set_content(Some(&col));
    dialog.set_child(Some(&tb));

    let jump_to_error = {
        let (buf, view) = (buf.clone(), view.clone());
        move |line: Option<usize>| {
            if let Some(l) = line.filter(|l| *l > 0) {
                if let Some(mut it) = buf.iter_at_line(l as i32 - 1) {
                    buf.place_cursor(&it);
                    view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
                }
            }
        }
    };
    let source_of = {
        let buf = buf.clone();
        move || buffer_text(&buf)
    };
    let ctx = m.ctx.clone();
    {
        let (out, name, find, jump, source_of, sample_buf) = (
            out.clone(),
            name.clone(),
            find.clone(),
            jump_to_error.clone(),
            source_of.clone(),
            sample_buf.clone(),
        );
        test.connect_clicked(move |_| {
            let func = Function::new(&name.text(), &find.text(), &source_of());
            let result = match Pattern::new(&func.find, ctx.opts) {
                Ok(p) => {
                    let text = buffer_text(&sample_buf);
                    let t = sfn::test_run(&func, &ctx.env, &p, &text, "");
                    jump(t.error.as_ref().and_then(|e| e.line));
                    let mut d = sfn::describe_test(&t);
                    if t.error.is_none() {
                        d.push_str(&format!("\nResult:\n{}", t.text));
                    }
                    d
                }
                Err(_) => "Error: the suggested regular expression is empty or not valid\n".into(),
            };
            out.buffer().set_text(&result);
        });
    }
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let (m2, d, out2) = (m.clone(), dialog.clone(), out.clone());
    save.connect_clicked(move |_| {
        let source = source_of();
        if let Err(e) = sfn::check(&source) {
            out2.buffer().set_text(&trf("Error: {}\n", &[&e]));
            jump_to_error(e.line);
            return;
        }
        let nm = name.text().trim().to_string();
        if nm.is_empty() {
            out2.buffer()
                .set_text(&tr("Error: give the function a name\n"));
            return;
        }
        let mut func = Function::new(&nm, &find.text(), &source);
        func.description = f.description.clone();
        {
            let mut u = m2.user.borrow_mut();
            if let Some(old) = old_name.as_ref().filter(|o| **o != nm && !builtin) {
                u.retain(|x| &x.name != old);
            }
            u.retain(|x| x.name != nm);
            u.push(func);
        }
        changed(&m2);
        d.close();
    });
    dialog.present(Some(&m.parent));
}
