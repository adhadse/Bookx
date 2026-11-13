//! Completion popup of the code view: one GtkSourceView `CompletionProvider` that asks
//! `editor_complete` what fits at the cursor (tag and attribute names, CSS properties, internal
//! files and anchors, words of the file) and offers the snippets whose trigger matches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::gtk::{self, gio, glib, prelude::*, subclass::prelude::*};
use sourceview5 as sv;
use sv::prelude::*;
use sv::subclass::prelude::*;

use crate::core::book::OpenBook;
use crate::editor_complete::{self as ec, Candidate, Context};
use crate::editor_snippets::{self as snip, Snippet};

/// What the provider needs to know about the book; the editor keeps it up to date.
pub struct Data {
    pub current: String,
    pub files: Vec<String>,
    pub classes: Vec<String>,
    pub words: Vec<String>,
    /// Ids of files whose text differs from the archive (the buffers being edited).
    pub edited_ids: HashMap<String, Vec<String>>,
    pub book: Rc<RefCell<OpenBook>>,
    pub snippets: Vec<Snippet>,
    pub region: Rc<SnippetRegion>,
    /// How often the popup asked for proposals and how many it got the last time (dev hook).
    pub populated: (usize, usize),
}

pub type SharedData = Rc<RefCell<Data>>;

impl Data {
    pub fn is_css(&self) -> bool {
        crate::core::util::extension(&self.current) == "css"
    }

    fn ids_of(&self, path: &str) -> Vec<String> {
        if let Some(v) = self.edited_ids.get(path) {
            return v.clone();
        }
        self.book
            .borrow()
            .archive
            .get_string(path)
            .map(|t| bookx_editing::markup::defined_ids(&t))
            .unwrap_or_default()
    }
}

/// Where the last snippet was expanded, so Tab keeps moving between its tab stops while the caret
/// is inside it.
#[derive(Default)]
pub struct SnippetRegion {
    marks: RefCell<Option<(gtk::TextMark, gtk::TextMark)>>,
}

impl SnippetRegion {
    pub fn active(&self, buf: &gtk::TextBuffer) -> bool {
        let m = self.marks.borrow();
        let Some((a, b)) = m.as_ref() else {
            return false;
        };
        if a.is_deleted() || b.is_deleted() {
            return false;
        }
        let cur = buf.iter_at_mark(&buf.get_insert()).offset();
        cur >= buf.iter_at_mark(a).offset() && cur < buf.iter_at_mark(b).offset()
    }

    fn set(&self, buf: &gtk::TextBuffer, start: gtk::TextMark, end: gtk::TextMark) {
        if let Some((a, b)) = self.marks.borrow_mut().take() {
            if !a.is_deleted() {
                buf.delete_mark(&a);
            }
            if !b.is_deleted() {
                buf.delete_mark(&b);
            }
        }
        *self.marks.borrow_mut() = Some((start, end));
    }
}

/// Inserts the snippet at the caret, replacing the `trigger_len` characters before it.
pub fn expand(view: &sv::View, region: &SnippetRegion, s: &Snippet, trigger_len: i32) -> bool {
    let buf = view.buffer();
    let Ok(snippet) = sv::Snippet::new_parsed(&s.body) else {
        return false;
    };
    snippet.set_name(&s.name);
    buf.begin_user_action();
    let mut end = buf.iter_at_mark(&buf.get_insert());
    let mut start = end;
    start.backward_chars(trigger_len);
    if trigger_len > 0 {
        buf.delete(&mut start, &mut end);
    }
    let mut at = buf.iter_at_mark(&buf.get_insert());
    let m_start = gtk::TextMark::new(None, true);
    let m_end = gtk::TextMark::new(None, false);
    buf.add_mark(&m_start, &at);
    buf.add_mark(&m_end, &at);
    view.push_snippet(&snippet, Some(&mut at));
    buf.end_user_action();
    region.set(&buf, m_start, m_end);
    true
}

/// Expands the snippet whose trigger stands before the caret. `false` when none does.
pub fn expand_at_caret(view: &sv::View, data: &SharedData) -> bool {
    let buf = view.buffer();
    if buf.selection_bounds().is_some() {
        return false;
    }
    let d = data.borrow();
    if d.region.active(&buf) {
        return false;
    }
    let cur = buf.iter_at_mark(&buf.get_insert());
    let mut from = cur;
    from.backward_chars(40);
    let before = buf.text(&from, &cur, false);
    let word = snip::trailing_trigger(&before);
    if word.is_empty() {
        return false;
    }
    let Some(s) = snip::find(&d.snippets, word, d.is_css()).cloned() else {
        return false;
    };
    let region = d.region.clone();
    drop(d);
    expand(view, &region, &s, word.chars().count() as i32)
}

// ---- proposal ------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct Item {
    pub label: String,
    pub insert: String,
    pub back: usize,
    pub kind: &'static str,
    pub replace_len: usize,
    pub snippet: Option<Snippet>,
}

mod proposal_imp {
    use super::*;

    #[derive(Default)]
    pub struct Proposal {
        pub item: RefCell<Item>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Proposal {
        const NAME: &'static str = "BookxCompletionProposal";
        type Type = super::Proposal;
        type ParentType = glib::Object;
        type Interfaces = (sv::CompletionProposal,);
    }

    impl ObjectImpl for Proposal {}
    impl CompletionProposalImpl for Proposal {}
}

glib::wrapper! {
    pub struct Proposal(ObjectSubclass<proposal_imp::Proposal>)
        @implements sv::CompletionProposal;
}

impl Proposal {
    fn new(item: Item) -> Proposal {
        let p: Proposal = glib::Object::new();
        *p.imp().item.borrow_mut() = item;
        p
    }
    fn item(&self) -> Item {
        self.imp().item.borrow().clone()
    }
}

// ---- provider --------------------------------------------------------------------

mod provider_imp {
    use super::*;

    #[derive(Default)]
    pub struct Provider {
        pub data: RefCell<Option<SharedData>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Provider {
        const NAME: &'static str = "BookxCompletionProvider";
        type Type = super::Provider;
        type ParentType = glib::Object;
        type Interfaces = (sv::CompletionProvider,);
    }

    impl ObjectImpl for Provider {}

    impl CompletionProviderImpl for Provider {
        fn title(&self) -> Option<glib::GString> {
            Some("Book".into())
        }

        fn priority(&self, _context: &sv::CompletionContext) -> i32 {
            10
        }

        fn is_trigger(&self, _iter: &gtk::TextIter, c: char) -> bool {
            matches!(c, '<' | ' ' | '#' | '/' | '"' | '\'' | '.')
        }

        fn populate(&self, context: &sv::CompletionContext) -> Result<gio::ListModel, glib::Error> {
            let store = gio::ListStore::new::<Proposal>();
            self.fill(context, &store);
            Ok(store.upcast())
        }

        fn refilter(&self, context: &sv::CompletionContext, model: &gio::ListModel) {
            if let Some(store) = model.downcast_ref::<gio::ListStore>() {
                store.remove_all();
                self.fill(context, store);
            }
        }

        fn display(
            &self,
            _context: &sv::CompletionContext,
            proposal: &sv::CompletionProposal,
            cell: &sv::CompletionCell,
        ) {
            let Some(p) = proposal.downcast_ref::<Proposal>() else {
                return;
            };
            let it = p.item();
            match cell.column() {
                sv::CompletionColumn::TypedText => cell.set_text(Some(&it.label)),
                sv::CompletionColumn::Comment => cell.set_text(Some(it.kind)),
                sv::CompletionColumn::Icon => cell.set_icon_name(match it.kind {
                    "tag" => "text-html-symbolic",
                    "file" => "text-x-generic-symbolic",
                    "anchor" => "insert-link-symbolic",
                    "snippet" => "insert-text-symbolic",
                    _ => "format-text-rich-symbolic",
                }),
                _ => cell.set_text(None),
            }
        }

        fn activate(&self, context: &sv::CompletionContext, proposal: &sv::CompletionProposal) {
            let Some(p) = proposal.downcast_ref::<Proposal>() else {
                return;
            };
            let it = p.item();
            let (Some(buf), Some(view)) = (context.buffer(), context.view()) else {
                return;
            };
            if let Some(s) = &it.snippet {
                let region = self
                    .data
                    .borrow()
                    .as_ref()
                    .map(|d| d.borrow().region.clone())
                    .unwrap_or_default();
                expand(&view, &region, s, it.replace_len as i32);
                return;
            }
            buf.begin_user_action();
            let mut end = buf.iter_at_mark(&buf.get_insert());
            let mut start = end;
            start.backward_chars(it.replace_len as i32);
            buf.delete(&mut start, &mut end);
            buf.insert(&mut end, &it.insert);
            if it.back > 0 {
                end.backward_chars(it.back as i32);
                buf.place_cursor(&end);
            }
            buf.end_user_action();
        }
    }

    impl Provider {
        fn fill(&self, context: &sv::CompletionContext, store: &gio::ListStore) {
            let Some(data) = self.data.borrow().clone() else {
                return;
            };
            // a fresh iterator: the ones of the context go stale when the text changed meanwhile
            let Some(buf) = context.buffer() else {
                return;
            };
            let cursor = buf.iter_at_mark(&buf.get_insert());
            let mut from = cursor;
            from.backward_chars(6000);
            let before = buf.text(&from, &cursor, false).to_string();
            let items = proposals(&data.borrow(), &before);
            {
                let mut d = data.borrow_mut();
                d.populated = (d.populated.0 + 1, items.len());
            }
            for it in items {
                store.append(&Proposal::new(it));
            }
        }
    }
}

/// What the popup offers when `before` (the text up to the caret) has just been typed.
pub fn proposals(d: &Data, before: &str) -> Vec<Item> {
    let ids = |p: &str| d.ids_of(p);
    let cd = ec::Data {
        current: &d.current,
        files: &d.files,
        ids_of: &ids,
        classes: &d.classes,
        words: &d.words,
    };
    let ctx = ec::context(before, d.is_css());
    let replace_len = ec::prefix_len(&ctx);
    let mut items: Vec<Item> = Vec::new();
    // snippets first, when a trigger is being typed in plain text
    let typed = match &ctx {
        Context::Word { prefix } | Context::CssProperty { prefix } => Some(prefix.as_str()),
        _ => None,
    };
    if let Some(w) = typed.filter(|w| !w.is_empty()) {
        for s in d.snippets.iter().filter(|s| {
            s.applies_to_file(d.is_css()) && !s.trigger.is_empty() && s.trigger.starts_with(w)
        }) {
            items.push(Item {
                label: format!("{}  —  {}", s.trigger, s.name),
                kind: "snippet",
                replace_len: w.chars().count(),
                snippet: Some(s.clone()),
                ..Default::default()
            });
        }
    }
    for c in ec::candidates(&ctx, &cd) {
        items.push(item_of(&c, replace_len));
    }
    items
}

fn item_of(c: &Candidate, replace_len: usize) -> Item {
    Item {
        label: c.label.clone(),
        insert: c.insert.clone(),
        back: c.back,
        kind: c.kind.label(),
        replace_len,
        snippet: None,
    }
}

glib::wrapper! {
    pub struct Provider(ObjectSubclass<provider_imp::Provider>)
        @implements sv::CompletionProvider;
}

impl Provider {
    pub fn new(data: SharedData) -> Provider {
        let p: Provider = glib::Object::new();
        *p.imp().data.borrow_mut() = Some(data);
        p
    }
}

/// Registers the provider on the view's completion popup.
pub fn install(view: &sv::View, data: SharedData) {
    let completion = view.completion();
    completion.set_select_on_show(true);
    completion.add_provider(&Provider::new(data));
}
