//! The Live CSS panel (CE-260): the rules that apply to the element at the cursor, where they are written,
//! what the element inherits, the resulting values, and which declarations lose. Read only.

use crate::i18n::{tr, trf};
use std::cell::RefCell;

use bookx_editing::live_css::{self, DeclInfo, LiveCss, MatchedRule};

use super::preview::PvMsg;
use super::*;

#[derive(Debug)]
pub enum CssMsg {
    Refresh,
    /// Show the rule written at this byte of that file.
    Jump(String, usize),
}

pub(super) struct CssUi {
    pub root: gtk::ScrolledWindow,
    body: gtk::Box,
    /// The last result, for the dev hooks.
    pub last: RefCell<Option<LiveCss>>,
}

fn markup(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

impl CssUi {
    pub(super) fn new(_sender: &ComponentSender<Editor>) -> CssUi {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_all(12);
        let root = gtk::ScrolledWindow::new();
        root.set_child(Some(&body));
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        crate::ui::audit::label(&root, &tr("Styles that apply to the element at the cursor"));
        let ui = CssUi {
            root,
            body,
            last: RefCell::new(None),
        };
        ui.message("Put the cursor in a chapter to see the styles that apply there.");
        ui
    }

    fn clear(&self) {
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
    }

    fn message(&self, text: &str) {
        self.clear();
        let l = gtk::Label::new(Some(text));
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.add_css_class("dim-label");
        self.body.append(&l);
    }

    fn decl_rows(expander: &adw::ExpanderRow, decls: &[DeclInfo]) {
        for d in decls {
            let row = adw::ActionRow::new();
            let name = markup(&d.name);
            row.set_title(&if d.overridden {
                format!("<s>{name}</s>")
            } else {
                name
            });
            let value = markup(&format!(
                "{}{}",
                d.value,
                if d.important { " !important" } else { "" }
            ));
            row.set_subtitle(&if d.overridden {
                format!("<s>{value}</s>")
            } else {
                value
            });
            row.set_subtitle_lines(2);
            if d.overridden {
                row.add_css_class("dim-label");
                row.set_tooltip_text(Some(&tr("Another rule wins for this property")));
            }
            expander.add_row(&row);
        }
    }

    fn rule_row(&self, r: &MatchedRule, tx: &relm4::Sender<Input>) -> adw::ExpanderRow {
        let row = adw::ExpanderRow::new();
        row.set_title(&markup(&r.selector));
        row.set_subtitle(&if r.inline {
            markup(&r.origin.label())
        } else {
            format!(
                "{} · specificity {}",
                markup(&r.origin.label()),
                spec_text(r.specificity)
            )
        });
        row.set_expanded(true);
        let jump =
            gtk::Button::with_label(&format!("{}:{}", file_name(&r.origin.file), r.origin.line));
        jump.set_valign(gtk::Align::Center);
        jump.add_css_class("flat");
        jump.set_tooltip_text(Some(&trf("Go to {}", &[&r.origin.label()])));
        crate::ui::audit::label(
            &jump,
            &trf("Go to the rule {} in {}", &[&r.selector, &r.origin.label()]),
        );
        let (file, off) = (r.origin.file.clone(), r.origin.offset);
        let tx = tx.clone();
        jump.connect_clicked(move |_| {
            tx.emit(Input::Pv(PvMsg::Css(CssMsg::Jump(file.clone(), off))))
        });
        row.add_suffix(&jump);
        Self::decl_rows(&row, &r.decls);
        row
    }

    fn show(&self, l: &LiveCss, tx: &relm4::Sender<Input>) {
        self.clear();
        let head = adw::PreferencesGroup::new();
        head.set_title(&markup(&l.element));
        head.set_description(Some(&markup(&format!("line {} · {}", l.line, l.path))));
        self.body.append(&head);

        let own = adw::PreferencesGroup::new();
        own.set_title(&tr("Applies to this element"));
        if l.rules.is_empty() {
            own.set_description(Some(&tr(
                "No rule of the book's styles matches this element.",
            )));
        }
        for r in &l.rules {
            own.add(&self.rule_row(r, tx));
        }
        self.body.append(&own);

        let mut from: Vec<&str> = Vec::new();
        for r in &l.inherited {
            let f = r.from.as_deref().unwrap_or("");
            if !from.contains(&f) {
                from.push(f);
            }
        }
        for f in from {
            let g = adw::PreferencesGroup::new();
            g.set_title(&trf("Inherited from {}", &[&markup(f)]));
            for r in l.inherited.iter().filter(|r| r.from.as_deref() == Some(f)) {
                g.add(&self.rule_row(r, tx));
            }
            self.body.append(&g);
        }

        let comp = adw::PreferencesGroup::new();
        comp.set_title(&tr("Resulting values"));
        comp.set_description(Some(&tr(
            "The value each property gets from the book's styles",
        )));
        for c in &l.computed {
            let row = adw::ActionRow::new();
            row.set_title(&markup(&c.name));
            row.set_subtitle(&markup(&format!(
                "{}  ·  {}{}",
                c.value,
                if c.inherited { "inherited, " } else { "" },
                c.origin.label()
            )));
            row.set_subtitle_lines(2);
            comp.add(&row);
        }
        if l.computed.is_empty() {
            comp.set_description(Some(&tr("The book's styles set nothing here")));
        }
        self.body.append(&comp);
    }
}

fn spec_text(s: u32) -> String {
    format!("{},{},{}", s >> 16, (s >> 8) & 0xff, s & 0xff)
}

impl Editor {
    pub(super) fn css_msg(&mut self, m: CssMsg) {
        match m {
            CssMsg::Refresh => self.css_refresh(),
            CssMsg::Jump(file, off) => self.goto_offset(&file, off),
        }
    }

    fn css_refresh(&mut self) {
        let Some(path) = self
            .current
            .clone()
            .filter(|c| bookx_editing::markup::is_markup_path(c))
        else {
            self.pv
                .css
                .message("Open a chapter to see the styles that apply to it.");
            *self.pv.css.last.borrow_mut() = None;
            return;
        };
        self.flush();
        let buf = self.buffer_for(&path);
        let text = buffer_text(&buf);
        let c = buf.iter_at_mark(&buf.get_insert()).offset() as usize;
        let off = super::search::char_to_byte_idx(&text, c);
        let res = live_css::inspect(&self.book.borrow().archive, &path, &text, off);
        match res {
            Ok(l) => {
                self.pv.css.show(&l, self.sender.input_sender());
                *self.pv.css.last.borrow_mut() = Some(l);
            }
            Err(e) => {
                self.pv
                    .css
                    .message(&trf("No styles to show here: {}", &[&format!("{:#}", e)]));
                *self.pv.css.last.borrow_mut() = None;
            }
        }
    }

    pub(super) fn css_dev(&mut self, c: &str, sender: &ComponentSender<Self>) -> bool {
        if c == "css-show" {
            self.w.sidebar_stack.set_visible_child_name("css");
            self.css_refresh();
        } else if c == "css-dump" {
            self.css_refresh();
            match self.pv.css.last.borrow().as_ref() {
                Some(l) => {
                    eprintln!(
                        "editor_css: element={} line={} path={:?} rules={} inherited={} computed={}",
                        l.element,
                        l.line,
                        l.path,
                        l.rules.len(),
                        l.inherited.len(),
                        l.computed.len()
                    );
                    for r in l.rules.iter().chain(l.inherited.iter()) {
                        eprintln!(
                            "editor_css_rule: {} | {} | from={} | {}",
                            r.selector,
                            r.origin.label(),
                            r.from.as_deref().unwrap_or("-"),
                            r.decls
                                .iter()
                                .map(|d| format!(
                                    "{}{}:{}",
                                    if d.overridden { "~" } else { "" },
                                    d.name,
                                    d.value
                                ))
                                .collect::<Vec<_>>()
                                .join(";")
                        );
                    }
                    eprintln!(
                        "editor_css_computed: {}",
                        l.computed
                            .iter()
                            .map(|c| format!("{}={}", c.name, c.value))
                            .collect::<Vec<_>>()
                            .join(";")
                    );
                }
                None => eprintln!("editor_css: none"),
            }
        } else if let Some(n) = c.strip_prefix("css-jump=") {
            let target = {
                let g = self.pv.css.last.borrow();
                g.as_ref().and_then(|l| {
                    l.rules
                        .iter()
                        .find(|r| r.selector == n)
                        .map(|r| (r.origin.file.clone(), r.origin.offset))
                })
            };
            if let Some((f, o)) = target {
                sender.input(Input::Pv(PvMsg::Css(CssMsg::Jump(f, o))));
            }
        } else {
            return false;
        }
        true
    }
}
