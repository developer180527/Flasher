//! Two classic desktop-dialog widgets libgui does not have yet: notebook tabs
//! and a titled group box. Built only from libgui's public API, so they can
//! move into libgui unchanged.
//!
//! ```text
//!   ╭──────╮╭─────────╮╭─────────╮
//!   │ Flash ││ Restore ││ Options │       ← tab_bar
//! ──╯      ╰┴─────────┴┴─────────┴────╮
//! │  ╭─ Advanced settings ──────────╮ │  ← group_box
//! │  │ ☑ Verify after writing        │ │
//! │  ╰──────────────────────────────╯ │
//! ╰───────────────────────────────────╯  ← tab_page
//! ```

use libgui::*;

const TAB_PAD_X: f32 = 12.0;
const TAB_HEIGHT: f32 = 28.0;
/// Unselected tabs sit this much lower, so the selected one reads as in front.
const TAB_DROP: f32 = 3.0;
/// Where the first tab starts, from the page's left edge.
const TAB_INDENT: f32 = 6.0;
const RADIUS: f32 = 4.0;

/// The x of each tab relative to the page's left edge, and its width. A pure
/// function of the labels, so the page can cut its notch under the selected
/// tab in the same frame — no waiting for last frame's rects.
fn tab_geometry(ui: &Ui, labels: &[&str]) -> Vec<(f32, f32)> {
    let size = ui.theme.metrics.font_size;
    let mut x = TAB_INDENT;
    labels
        .iter()
        .map(|l| {
            let w = ui.fonts.measure(ui.font, size, l).x + 2.0 * TAB_PAD_X;
            let at = (x, w);
            x += w - 1.0; // neighbours share a border
            at
        })
        .collect()
}

/// A row of notebook tabs; clicking one sets `selected`. Returns each tab's
/// response, in order.
/// Follow it with [`tab_page`] for the page the tabs belong to.
pub fn tab_bar(ui: &mut Ui, key: &str, selected: &mut usize, labels: &[&str]) -> Vec<Response> {
    let geometry = tab_geometry(ui, labels);
    let pal = ui.theme.palette;
    let size = ui.theme.metrics.font_size;
    let mut responses = Vec::with_capacity(labels.len());

    ui.container_id(
        Id::new(("tab_bar", key)),
        Layout::row()
            .width(Size::Grow(1.0))
            .height(Size::Fixed(TAB_HEIGHT))
            .gap(-1.0)
            .padding(Insets {
                left: TAB_INDENT,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            }),
        Frame::none(),
        |ui| {
            for (i, label) in labels.iter().enumerate() {
                let id = Id::new(("tab", key, i));
                let r = ui.interact_focusable(id, FocusKind::Control);
                if r.hovered {
                    ui.cursor = Cursor::Pointer;
                }
                if r.clicked {
                    *selected = i;
                }
                let on = i == *selected;
                let hover = r.hovered && !on;
                let text = ui.frame_text(label);
                let w = geometry[i].1;
                ui.add_leaf(
                    id,
                    Layout::leaf(Size::Fixed(w), Size::Fixed(TAB_HEIGHT)),
                    Vec2::new(w, TAB_HEIGHT),
                    true,
                    move |p, r| {
                        let r = if on {
                            r
                        } else {
                            Rect::new(r.x, r.y + TAB_DROP, r.w, r.h - TAB_DROP)
                        };
                        let fill = if on {
                            pal.bg_panel
                        } else if hover {
                            pal.surface_hover
                        } else {
                            pal.bg_inset
                        };
                        // A rounded box, then square off and open its bottom:
                        // tabs are rounded only on top, and the selected one
                        // runs into its page with no line between them.
                        p.rect_bordered(r, fill, RADIUS, 1.0, pal.border);
                        p.rect(
                            Rect::new(r.x + 1.0, r.y + r.h - RADIUS - 1.0, r.w - 2.0, RADIUS + 1.0),
                            fill,
                            0.0,
                        );
                        p.line(
                            Vec2::new(r.x + 0.5, r.y + RADIUS),
                            Vec2::new(r.x + 0.5, r.y + r.h),
                            1.0,
                            pal.border,
                        );
                        p.line(
                            Vec2::new(r.x + r.w - 0.5, r.y + RADIUS),
                            Vec2::new(r.x + r.w - 0.5, r.y + r.h),
                            1.0,
                            pal.border,
                        );
                        let ink = if on { pal.text } else { pal.text_muted };
                        p.text_centered(r, size, ink, text);
                    },
                );
                responses.push(r);
            }
        },
    );
    responses
}

/// The bordered page under a [`tab_bar`], with the border cut open beneath
/// the selected tab so the two read as one surface.
pub fn tab_page<R>(
    ui: &mut Ui,
    key: &str,
    selected: usize,
    labels: &[&str],
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let pal = ui.theme.palette;
    let notch = tab_geometry(ui, labels).get(selected).copied();
    let frame = Frame {
        fill: pal.bg_panel,
        border: pal.border,
        border_width: 1.0,
        radius: RADIUS,
        shadow: false,
        clip: true,
    };
    ui.container_id(
        Id::new(("tab_page", key)),
        Layout::column()
            .width(Size::Grow(1.0))
            .height(Size::Grow(1.0)),
        frame,
        |ui| {
            // Paints over the page's top border under the selected tab. It is
            // the page's first child, so it draws after the border and before
            // anything else.
            ui.add_leaf(
                Id::new(("tab_notch", key)),
                Layout::leaf(Size::Grow(1.0), Size::Fixed(0.0)),
                Vec2::ZERO,
                false,
                move |p, r| {
                    if let Some((x, w)) = notch {
                        p.rect(
                            Rect::new(r.x + x + 1.0, r.y, w - 2.0, 1.5),
                            pal.bg_panel,
                            0.0,
                        );
                    }
                },
            );
            ui.container_id(
                Id::new(("tab_body", key, selected)),
                Layout::column()
                    .width(Size::Grow(1.0))
                    .height(Size::Grow(1.0))
                    .padding(Insets::all(16.0))
                    .gap(10.0),
                Frame::none(),
                body,
            )
        },
    )
}

/// A bordered group with its title set into the top border, as in the
/// "Advanced settings" box of a classic settings dialog.
pub fn group_box<R>(ui: &mut Ui, key: &str, title: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = ui.theme.palette;
    let size = ui.theme.metrics.font_size;
    let line_h = ui.fonts.line_height(ui.font, size);
    let title_w = ui.fonts.measure(ui.font, size, title).x;
    let text = ui.frame_text(title);
    // The background the title is knocked out of: the page it sits on.
    let behind = pal.bg_panel;

    // Room above the box for the half of the title that pokes over its border.
    let outer = Layout::column()
        .width(Size::Grow(1.0))
        .height(Size::Fit)
        .padding(Insets {
            left: 0.0,
            top: line_h / 2.0,
            right: 0.0,
            bottom: 0.0,
        });
    ui.container_id(Id::new(("group_outer", key)), outer, Frame::none(), |ui| {
        let frame = Frame {
            fill: Color::TRANSPARENT,
            border: pal.border,
            border_width: 1.0,
            radius: RADIUS,
            shadow: false,
            clip: false,
        };
        let layout = Layout::column()
            .width(Size::Grow(1.0))
            .height(Size::Fit)
            .gap(8.0)
            .padding(Insets {
                left: 14.0,
                top: 0.0,
                right: 14.0,
                bottom: 12.0,
            });
        ui.container_id(Id::new(("group", key)), layout, frame, |ui| {
            ui.add_leaf(
                Id::new(("group_title", key)),
                Layout::leaf(Size::Grow(1.0), Size::Fixed(line_h / 2.0 + 4.0)),
                Vec2::new(title_w, 0.0),
                false,
                move |p, r| {
                    // Centred on the border line, which is this leaf's top edge.
                    let t = Rect::new(r.x - 4.0, r.y - line_h / 2.0, title_w + 8.0, line_h);
                    p.rect(t, behind, 0.0);
                    p.text_left(
                        Rect::new(t.x + 4.0, t.y, title_w + 4.0, t.h),
                        size,
                        pal.text_muted,
                        text,
                    );
                },
            );
            body(ui)
        })
    })
}

/// One line of text that fits its width by cutting the *middle* when it is
/// too long: `/Volumes/Long Name/…/image.img`. For paths, where the start says
/// where and the end says what, and the middle matters least.
pub fn label_middle_ellipsis(ui: &mut Ui, key: &str, text: &str, color: Color) {
    let size = ui.theme.metrics.font_size;
    let h = ui.fonts.line_height(ui.font, size);
    let text = text.to_string();
    ui.add_leaf(
        Id::new(("ellipsis", key)),
        Layout::leaf(Size::Grow(1.0), Size::Fixed(h)),
        Vec2::new(0.0, h),
        false,
        move |p, r| {
            let fits = |s: &str| p.measure(size, s).x <= r.w;
            let shown = middle_ellipsis(&text, fits);
            p.text_left(r, size, color, shown.as_str());
        },
    );
}

/// The longest `head…tail` of `text` that `fits` accepts, keeping a little
/// more of the end than the start (a file name outweighs its folders).
pub fn middle_ellipsis(text: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| {
        let tail = keep.div_ceil(2) + keep / 6;
        let tail = tail.min(keep);
        let head = keep - tail;
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - tail..]);
        s
    };
    // Binary search on how many characters survive.
    let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

#[cfg(test)]
mod tests {
    use super::middle_ellipsis;

    #[test]
    fn keeps_both_ends_and_fits() {
        let path = "/var/folders/dc/g6d23k_541lf8c41zxkxt/T/flasher_headless/stick.disk";
        let fits = |s: &str| s.chars().count() <= 30;
        let out = middle_ellipsis(path, fits);
        assert_eq!(out.chars().count(), 30, "{out}");
        assert!(
            out.starts_with("/var/") && out.ends_with("stick.disk") && out.contains('…'),
            "{out}"
        );
        assert_eq!(middle_ellipsis("/dev/disk4", fits), "/dev/disk4");
    }
}
