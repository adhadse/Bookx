//! Source positions for the editor preview: which block of the document is under a point, and where on
//! the widget a block starts. The editor tags blocks with anchors (`bxsN`, see `bookx_editing::preview_map`);
//! this module reads them back from the laid out items. It only reads the canvas state.

use super::*;
use bookx_editing::preview_map::block_of_anchor;

impl BookCanvas {
    /// The block number of the item that holds, or starts last before, the widget point.
    pub fn block_at_point(&self, x: f64, y: f64) -> Option<usize> {
        let s = self.st.borrow();
        let (flow_y, _) = to_flow(&s, x, y)?;
        let idx = s.items.partition_point(|i| i.y <= flow_y).checked_sub(1)?;
        s.items[..=idx]
            .iter()
            .rev()
            .take(8)
            .find_map(|i| i.anchors.iter().rev().find_map(|a| block_of_anchor(a)))
    }

    /// Widget `y` of the top of block `n` (horizontal text only), `None` when it is not laid out.
    pub fn block_top(&self, n: usize) -> Option<f64> {
        let s = self.st.borrow();
        if is_vertical(&s) {
            return None;
        }
        let id = bookx_editing::preview_map::anchor_name(n);
        let item = s.items.iter().find(|i| i.anchors.contains(&id))?;
        if s.style.layout == LayoutMode::Scroll {
            Some(V_MARGIN + item.y - s.scroll_y)
        } else {
            let page = s.pages.get(s.cur)?;
            (item.y >= page.y0 && item.y < page.y1).then_some(V_MARGIN + item.y - page.y0)
        }
    }

    /// Height of the visible text area of the widget.
    pub fn visible_height(&self) -> f64 {
        let s = self.st.borrow();
        scroll_view(&s)
    }

    /// Scrolls so that block `n` is shown, a little below the top; does nothing when it already is in the
    /// upper two thirds of the view.
    pub fn reveal_block(&self, n: usize) -> bool {
        let top = self.block_top(n);
        let view = self.visible_height();
        match top {
            Some(t) if t >= V_MARGIN && t < view * 0.66 => true,
            _ => self.goto_anchor(&bookx_editing::preview_map::anchor_name(n)),
        }
    }

    /// `(block, y in the flow)` of every block that carries a source anchor, and the height of the flow.
    pub fn block_anchors(&self) -> (Vec<(usize, f64)>, f64) {
        let s = self.st.borrow();
        let v = s
            .items
            .iter()
            .filter_map(|i| {
                i.anchors
                    .iter()
                    .find_map(|a| block_of_anchor(a))
                    .map(|b| (b, i.y))
            })
            .collect();
        (v, s.flow_h)
    }

    /// Height of the whole laid out page.
    pub fn flow_height(&self) -> f64 {
        self.st.borrow().flow_h
    }

    /// Flow y at the top of the view (scrolling layout).
    pub fn scroll_top(&self) -> f64 {
        self.st.borrow().scroll_y
    }

    /// Scrolls the view to flow y without telling the position listeners (the editor's scroll sync).
    pub fn set_scroll_top(&self, y: f64) -> bool {
        {
            let mut s = self.st.borrow_mut();
            if s.style.layout != LayoutMode::Scroll || is_vertical(&s) {
                return false;
            }
            s.scroll_y = y.clamp(0.0, scroll_max(&s));
        }
        self.area.queue_draw();
        true
    }
}
