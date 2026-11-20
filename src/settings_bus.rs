//! A small change notification for settings that several windows show at once (the footer statistics
//! are edited in Preferences, in the reader's own popover and in its main menu, and every open
//! reader must follow at once). Subscribers are plain callbacks; dropping the [`Subscription`] ends
//! it. Not thread safe on purpose: GTK is single threaded.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Default)]
pub struct Bus {
    subs: RefCell<Vec<(u64, Rc<dyn Fn()>)>>,
    next: Cell<u64>,
}

/// Keeps a callback subscribed while it lives.
pub struct Subscription {
    id: u64,
    bus: Weak<Bus>,
}

impl Bus {
    pub fn subscribe(self: &Rc<Self>, f: impl Fn() + 'static) -> Subscription {
        let id = self.next.get();
        self.next.set(id + 1);
        self.subs.borrow_mut().push((id, Rc::new(f)));
        Subscription {
            id,
            bus: Rc::downgrade(self),
        }
    }

    /// Calls every subscriber (a callback may subscribe or unsubscribe meanwhile).
    pub fn notify(&self) {
        let now: Vec<Rc<dyn Fn()>> = self.subs.borrow().iter().map(|(_, f)| f.clone()).collect();
        for f in now {
            f();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.subs.borrow().len()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(b) = self.bus.upgrade() {
            b.subs.borrow_mut().retain(|(i, _)| *i != self.id);
        }
    }
}

thread_local! {
    static FOOTER: Rc<Bus> = Rc::new(Bus::default());
    static EDITOR: Rc<Bus> = Rc::new(Bus::default());
    static SYNC: Rc<Bus> = Rc::new(Bus::default());
}

/// A sync setting or the library server changed (Preferences → Sync and the Sync window show the
/// same state and follow each other through this).
pub fn sync() -> Rc<Bus> {
    SYNC.with(|b| b.clone())
}

/// The editor settings (`editor_prefs`) were changed; every open editor window applies them.
pub fn editor() -> Rc<Bus> {
    EDITOR.with(|b| b.clone())
}

/// The footer statistics were changed (in the common settings or in a book's own).
pub fn footer() -> Rc<Bus> {
    FOOTER.with(|b| b.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subscriber_hears_a_change() {
        let bus = Rc::new(Bus::default());
        let n = Rc::new(Cell::new(0));
        let (a, b) = (n.clone(), n.clone());
        let _s1 = bus.subscribe(move || a.set(a.get() + 1));
        let _s2 = bus.subscribe(move || b.set(b.get() + 10));
        bus.notify();
        assert_eq!(n.get(), 11);
        bus.notify();
        assert_eq!(n.get(), 22);
    }

    #[test]
    fn a_dropped_subscription_hears_nothing() {
        let bus = Rc::new(Bus::default());
        let n = Rc::new(Cell::new(0));
        let a = n.clone();
        let s = bus.subscribe(move || a.set(a.get() + 1));
        bus.notify();
        drop(s);
        assert_eq!(bus.len(), 0);
        bus.notify();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn a_callback_may_end_its_own_subscription() {
        let bus = Rc::new(Bus::default());
        let slot: Rc<RefCell<Option<Subscription>>> = Rc::new(RefCell::new(None));
        let (s2, n) = (slot.clone(), Rc::new(Cell::new(0)));
        let n2 = n.clone();
        *slot.borrow_mut() = Some(bus.subscribe(move || {
            n2.set(n2.get() + 1);
            s2.borrow_mut().take();
        }));
        bus.notify();
        bus.notify();
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn the_footer_bus_is_shared_by_the_whole_app() {
        let n = Rc::new(Cell::new(0));
        let a = n.clone();
        let _s = footer().subscribe(move || a.set(a.get() + 1));
        footer().notify();
        assert_eq!(n.get(), 1);
    }
}
