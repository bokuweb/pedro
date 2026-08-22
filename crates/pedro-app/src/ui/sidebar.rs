//! Everything to the left of the page: the window controls, where in the
//! application you are, what that place holds, and which agent is answering.
//!
//! There is no icon rail beside this. A column of icons whose labels are only
//! ever a tooltip is a second navigation for the same six places this one
//! already names, and it costs 64 points of every window to say less.
//!
//! Rows are separated by space rather than by hairlines, and the one you are on
//! is a rounded fill inset from both edges. A list of documents is read by
//! scanning titles, and rules between them turn that into a table.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex, v_flex};

use pedro_core::store::MAX_SHELF_DEPTH;

use crate::app::Pedro;
use crate::palette;
use crate::state::{AgentStatus, Entry, RailItem, Section, Status};
use crate::ui::icon;

pub(crate) const SIDEBAR_WIDTH: f32 = 300.;

/// How far the rows are inset from the panel edges. The fill on the active row
/// stops here, which is what makes it read as a card rather than a band.
const INSET: f32 = 8.;

/// Room for the macOS traffic lights, which live in this panel now that there
/// is no title bar and no rail for them to live in.
const LIGHTS: f32 = 84.;

/// Names the row a remove button hides inside until the row is hovered.
const GROUP: &str = "row";

/// The same, for the header of a shelf: the delete affordance hides in it.
const HEADER: &str = "section-header";

/// How far a shelf standing on another one is stepped in, and its books with
/// it. Wide enough to read as one level at a glance, narrow enough that three
/// of them still leave a title room in a 300 point panel.
const STEP: f32 = 14.;

impl Pedro {
    pub(crate) fn render_sidebar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.sidebar.is_visible() {
            return None;
        }

        Some(
            v_flex()
                .w(px(self.sidebar.width))
                // Not hidden: the shelf menu is drawn over the panel and a
                // menu clipped to the panel is a menu with items missing.
                .relative()
                .h_full()
                .flex_shrink_0()
                .bg(palette::sidebar())
                .border_r_1()
                .border_color(palette::border())
                .children(self.render_shelf_menu(cx))
                .child(self.render_window_row(window, cx))
                .child(self.render_navigation(cx))
                .children(self.render_drive_field())
                .child(self.render_search())
                .children(self.render_notice())
                .children(self.render_new_shelf_button(cx))
                .child(self.render_sections(cx))
                .child(self.render_agent_footer()),
        )
    }

    /// The row the window controls sit in, with the panel's own actions beside
    /// them. Dragging it moves the window, the way a title bar would.
    fn render_window_row(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let hint = self.active_rail.hint();
        let indent = if window.is_fullscreen() {
            INSET
        } else {
            LIGHTS
        };

        let row = h_flex()
            .id("window-row")
            .h(px(46.))
            .pl(px(indent))
            .pr(px(INSET))
            .gap(px(2.))
            .items_center()
            .child(self.render_add_button(cx))
            .child(self.render_drive_button(cx))
            .child(
                div()
                    .id("panel-hint")
                    .size(px(24.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|this| this.bg(palette::row_hover()))
                    .child(icon(IconName::Info, px(14.), palette::text_faint()))
                    .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx)),
            )
            .child(div().flex_1());

        self.draggable(row, cx)
    }

    /// Where in the application the reader is.
    ///
    /// The four places a reader goes are always listed; the two that are
    /// settings rather than reading are behind "More", which is where a thing
    /// you touch twice a month belongs.
    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let primary: Vec<_> = RailItem::PRIMARY
            .into_iter()
            .map(|item| self.render_navigation_row(item, cx))
            .collect();
        let secondary: Vec<_> = if self.show_secondary {
            RailItem::SECONDARY
                .into_iter()
                .map(|item| self.render_navigation_row(item, cx))
                .collect()
        } else {
            Vec::new()
        };

        let chevron = if self.show_secondary {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        v_flex()
            .flex_shrink_0()
            .pb(px(6.))
            .children(primary)
            .child(
                h_flex()
                    .id("more")
                    .h(px(30.))
                    .mx(px(INSET))
                    .px(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|this| this.bg(palette::row_hover()))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_secondary(cx)))
                    .child(icon(chevron, px(13.), palette::text_faint()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::text_faint())
                            .child("More"),
                    ),
            )
            .children(secondary)
    }

    fn render_navigation_row(
        &self,
        item: RailItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.active_rail == item;

        h_flex()
            .id(("navigate", item as usize))
            .h(px(32.))
            .mx(px(INSET))
            .px(px(8.))
            .gap(px(9.))
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(active, |this| this.bg(palette::row_active()))
            .when(!active, |this| {
                this.hover(|this| this.bg(palette::row_hover()))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select_rail(item, cx)))
            .child(icon(
                item.icon(),
                px(15.),
                if active {
                    palette::text()
                } else {
                    palette::text_muted()
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(if active {
                        palette::text()
                    } else {
                        palette::text_muted()
                    })
                    .child(item.title()),
            )
    }

    /// The last thing that went wrong, where the reader was looking when it
    /// did. A file that could not be added is about the list it is missing
    /// from, so it belongs here rather than in a notification that floats away.
    fn render_notice(&self) -> Option<impl IntoElement + use<>> {
        let notice = self.notice.clone()?;

        Some(
            div()
                .mx(px(INSET))
                .mb(px(6.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .bg(palette::danger().opacity(0.14))
                .text_size(px(11.))
                .text_color(palette::danger())
                .child(notice),
        )
    }

    /// The plus in the panel header.
    fn render_add_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("panel-add")
            .size(px(24.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(IconName::Plus, px(15.), palette::text_muted()))
            .tooltip(move |window, cx| Tooltip::new("Add a document").build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.pick_documents(cx);
            }))
    }

    /// The other way a book gets in: out of Drive rather than off the disk.
    fn render_drive_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("panel-drive")
            .size(px(24.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            // Held open while the field below it is, so the button reads as the
            // thing that opened it rather than as one that did nothing.
            .when(self.drive_open, |this| this.bg(palette::row_hover()))
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(IconName::ExternalLink, px(14.), palette::text_muted()))
            .tooltip(move |window, cx| Tooltip::new("Add from Google Drive").build(window, cx))
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.toggle_drive(window, cx);
            }))
    }

    /// Makes a shelf, directly on top of the shelves it will join.
    ///
    /// This was an icon in the window row, four rows and a search box above the
    /// list it changes, beside the buttons that add a document. But a reader
    /// makes a shelf while looking at the books they mean to group, and the row
    /// they are looking at is where the affordance has to be. It is a labelled
    /// row rather than an icon for the same reason the navigation is: a shelf
    /// and a folder are not the same thing, and only the word says which.
    ///
    /// It is also where a shelf comes back to the top level from: dragging one
    /// onto it takes it off whatever it was standing on, which is the only
    /// place in the list that means "not inside anything".
    ///
    /// A search replaces the list with passages, which are not shelved, so it
    /// takes this with it.
    fn render_new_shelf_button(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.active_rail != RailItem::Library || !self.search_query(cx).is_empty() {
            return None;
        }

        Some(
            h_flex()
                .id("panel-new-shelf")
                .h(px(30.))
                .mx(px(INSET))
                .px(px(8.))
                .gap(px(8.))
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|this| this.bg(palette::row_hover()))
                .drag_over::<DraggedShelf>(|this, _, _, _| this.bg(palette::accent().opacity(0.26)))
                .on_drop(cx.listener(|this, moved: &DraggedShelf, _, cx| {
                    this.move_shelf(&moved.0, None, cx);
                }))
                .child(icon(IconName::FolderOpen, px(14.), palette::text_muted()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(palette::text_muted())
                        .child("New shelf"),
                )
                .tooltip(move |window, cx| {
                    Tooltip::new("A shelf is asked as one — every book on it at once")
                        .build(window, cx)
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.create_shelf(None, window, cx);
                })),
        )
    }

    /// Where a Drive link is pasted, when that has been asked for.
    fn render_drive_field(&self) -> Option<impl IntoElement + use<>> {
        if !self.drive_open {
            return None;
        }

        Some(
            div().px(px(INSET)).pb(px(6.)).child(
                Input::new(&self.drive)
                    .h(px(34.))
                    .prefix(icon(IconName::ExternalLink, px(15.), palette::text_faint()))
                    .suffix(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(match self.drive_busy {
                                true => "…",
                                false => "⏎",
                            }),
                    ),
            ),
        )
    }

    fn render_search(&self) -> impl IntoElement + use<> {
        div().px(px(INSET)).pb(px(6.)).child(
            Input::new(&self.search)
                .h(px(34.))
                .prefix(icon(IconName::Search, px(15.), palette::text_faint()))
                .suffix(
                    div()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child("⌘K"),
                ),
        )
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let query = self.search_query(cx);
        let panel = self.panel();

        // A search hides sections that have nothing left in them; with no
        // search every section stays visible, collapsed or not.
        let visible: Vec<(usize, &Section, Vec<&Entry>)> = panel
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| (index, section, section.matching(&query)))
            .filter(|(_, _, entries)| query.is_empty() || !entries.is_empty())
            .collect();

        let empty_message: SharedString = if query.is_empty() {
            panel.empty_message.clone()
        } else {
            format!("Nothing matches “{query}”.").into()
        };

        let is_empty = visible.is_empty();

        // Rendered up front so that only one borrow of `cx` is live at a time;
        // builder temporaries would otherwise all survive to the end of the
        // statement.
        let mut rendered = Vec::with_capacity(visible.len());
        for (index, section, entries) in visible {
            // While searching, matches are always worth showing even if the
            // section was shut by hand.
            let expanded = self.is_expanded(index) || !query.is_empty();
            let header = self.render_section_header(index, section, expanded, cx);

            let rows: Vec<_> = if expanded {
                entries
                    .into_iter()
                    .map(|entry| self.render_entry(entry, section.depth, cx))
                    .collect()
            } else {
                Vec::new()
            };

            rendered.push(v_flex().child(header).children(rows));
        }

        v_flex()
            .id("sections")
            .flex_1()
            .min_h_0()
            .pb(px(8.))
            .overflow_y_scroll()
            .when(is_empty, |this| {
                this.child(
                    div()
                        .px(px(16.))
                        .py(px(24.))
                        .text_size(px(13.))
                        .text_color(palette::text_faint())
                        .child(empty_message),
                )
            })
            .children(rendered)
    }

    /// A quiet label with the chevron on the trailing edge, so the eye lands on
    /// the rows rather than on the headers between them.
    fn render_section_header(
        &self,
        index: usize,
        section: &Section,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let chevron = if expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        // A shelf header is a thing rather than a grouping: it opens, it takes
        // a book or another shelf dropped on it, and it can be thrown away. The
        // count sits beside the name so an empty shelf reads as empty rather
        // than as broken.
        let shelf = section.shelf.clone();
        let open = shelf.clone();
        let dropped = shelf.clone();
        let nested = shelf.clone();
        let dragged = shelf.clone();
        let name = section.title.clone();
        // Every level in steps the row, which is the only thing that says what
        // is standing on what.
        let step = section.depth.saturating_sub(1) as f32 * STEP;
        // A shelf being asked whether it should go has its own question drawn
        // over this, so the count stands down for as long as that is up.
        let asked = shelf
            .as_ref()
            .map(|shelf| SharedString::from(format!("shelf:{shelf}")))
            .is_some_and(|id| self.is_confirming(&id));
        // What a question to it would read, rather than how many rows are
        // under the header: a shelf of shelves has books it answers from and
        // does not list.
        let count = section
            .book_count
            .filter(|_| !asked)
            .map(|count| SharedString::from(count.to_string()));
        let tab_id = shelf.as_ref().map(|id| format!("shelf:{id}"));
        let active = tab_id.is_some_and(|id| self.active_tab().is_some_and(|tab| tab.id == id));

        h_flex()
            .id(("section", index))
            .group(HEADER)
            // The delete affordance is laid over the count rather than put
            // beside it: a name that shortens the moment the pointer crosses
            // the header is a name that moves while it is being read.
            .relative()
            .h(px(32.))
            .ml(px(INSET + step))
            .mr(px(INSET))
            .px(px(8.))
            .mt(px(10.))
            .gap(px(8.))
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(active, |this| this.bg(palette::row_active()))
            .when(!active, |this| {
                this.hover(|this| this.bg(palette::row_hover()))
            })
            // A book or a shelf being dragged over a shelf lights it, which is
            // the only thing that says the drop will land rather than be
            // ignored.
            .drag_over::<DraggedBook>(|this, _, _, _| this.bg(palette::accent().opacity(0.26)))
            .drag_over::<DraggedShelf>(|this, _, _, _| this.bg(palette::accent().opacity(0.26)))
            .on_drop(cx.listener(move |this, book: &DraggedBook, _, cx| {
                if let Some(shelf) = &dropped {
                    this.shelve_book(&book.0, Some(shelf.as_ref()), cx);
                }
            }))
            .on_drop(cx.listener(move |this, moved: &DraggedShelf, _, cx| {
                if let Some(onto) = &nested {
                    this.move_shelf(&moved.0, Some(onto.as_ref()), cx);
                }
            }))
            // A shelf is dragged the way a book is: onto another shelf to stand
            // on it, or onto the New shelf row to come back to the top level.
            .when_some(dragged, |this, id| {
                this.on_drag(DraggedShelf(id), move |_, _, _, cx| {
                    cx.new(|_| DragGhost(name.clone()))
                })
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                // Pressing the header rather than its delete button is an
                // answer to the question that button asked.
                this.cancel_confirmation(cx);

                match &open {
                    // Clicking a shelf opens it; clicking a grouping folds it.
                    Some(shelf) => {
                        let name = shelf.to_string();
                        let title = this
                            .library
                            .shelves()
                            .iter()
                            .find(|folder| folder.id == name)
                            .map(|folder| folder.name.clone())
                            .unwrap_or_default();

                        this.open_shelf(&name, &title, window, cx);
                    }
                    None => this.toggle_section(index, cx),
                }
            }))
            .children(shelf.is_some().then(|| {
                icon(
                    IconName::Folder,
                    px(13.),
                    if active {
                        palette::text()
                    } else {
                        palette::text_faint()
                    },
                )
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if active {
                        palette::text()
                    } else {
                        palette::text_faint()
                    })
                    .child(section.title.clone()),
            )
            .children(count.map(|count| {
                div()
                    .text_size(px(11.))
                    .text_color(palette::text_faint())
                    // The delete affordance is drawn over this, so the count
                    // gets out of the way rather than showing through it.
                    .group_hover(HEADER, |this| this.invisible())
                    .child(count)
            }))
            .child(
                div()
                    .id(("fold", index))
                    .p(px(2.))
                    .rounded(px(5.))
                    .hover(|this| this.bg(palette::row_hover()))
                    .child(icon(chevron, px(13.), palette::text_faint()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_section(index, cx);
                    })),
            )
            .children(
                shelf.map(|shelf| self.render_shelf_controls(&shelf, section.depth, asked, cx)),
            )
    }

    /// What can be done to a shelf from the list: put another shelf inside it,
    /// or throw it away.
    ///
    /// Laid over the count rather than put beside it. A name that shortens the
    /// moment the pointer crosses the header is a name that moves while it is
    /// being read, and at three levels in there is not much of it left to move.
    fn render_shelf_controls(
        &self,
        shelf: &SharedString,
        depth: usize,
        confirming: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // The bottom level has nothing to put a shelf on, so it does not offer.
        // Saying so afterwards, in a notice, would be a button that lies.
        let room = depth < MAX_SHELF_DEPTH && !confirming;

        h_flex()
            .absolute()
            .top(px(6.))
            .right(px(28.))
            .gap(px(2.))
            .items_center()
            .children(room.then(|| self.render_shelf_add_button(shelf, cx)))
            .child(self.render_shelf_delete_button(shelf, cx))
    }

    /// Makes a shelf inside this one.
    fn render_shelf_add_button(
        &self,
        shelf: &SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let parent = shelf.to_string();

        div()
            .id(SharedString::from(format!("add-shelf:{shelf}")))
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .invisible()
            .group_hover(HEADER, |this| this.visible())
            .bg(palette::surface())
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(IconName::Plus, px(12.), palette::text_muted()))
            .tooltip(move |window, cx| Tooltip::new("A shelf inside this one").build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.create_shelf(Some(parent.clone()), window, cx);
            }))
    }

    /// Throws a shelf away from the list it is in, asking first.
    ///
    /// The shelf view has this too, but reaching it means opening the shelf to
    /// delete it — which is a long way round for a shelf made by accident, and
    /// no way at all for one the reader can see is empty. The books stay
    /// either way: a shelf is a grouping, not a place the file lives.
    fn render_shelf_delete_button(
        &self,
        shelf: &SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // The same id the shelf view confirms under, so a shelf cannot be
        // half-asked about in two places at once.
        let asked = SharedString::from(format!("shelf:{shelf}"));
        let confirming = self.is_confirming(&asked);
        let shelf_id = shelf.to_string();

        div()
            .id(SharedString::from(format!("delete-shelf:{shelf}")))
            .px(px(7.))
            .h(px(20.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .when(!confirming, |this| {
                this.invisible()
                    .group_hover(HEADER, |this| this.visible())
                    .bg(palette::surface())
            })
            .when(confirming, |this| this.bg(palette::danger().opacity(0.24)))
            .hover(|this| this.bg(palette::danger().opacity(0.36)))
            .text_size(px(10.))
            .text_color(if confirming {
                palette::danger()
            } else {
                palette::text_muted()
            })
            .child(if confirming {
                "Delete? The books stay."
            } else {
                "Delete"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                match this.is_confirming(&asked) {
                    true => this.delete_shelf(&shelf_id, cx),
                    false => this.confirm(asked.clone(), cx),
                }
            }))
    }

    fn render_entry(
        &self,
        entry: &Entry,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = entry.current || self.active_tab().is_some_and(|tab| tab.id == entry.id);
        let clickable = entry.openable;
        let on_open = entry.clone();

        // Only a book has a shelf to be put on, and only a book can be dragged
        // onto one.
        let book_id = entry
            .id
            .strip_prefix("book:")
            .map(|id| SharedString::from(id.to_owned()));
        let dragged = book_id.clone();
        let menu_for = book_id.clone();
        let label = entry.label.clone();

        let row = div()
            .id(entry.id.clone())
            .group(GROUP)
            .relative()
            .ml(px(INSET + depth.saturating_sub(1) as f32 * STEP))
            .mr(px(INSET))
            .mt(px(2.))
            .rounded(px(10.))
            .bg(if active {
                palette::row_active()
            } else {
                palette::row()
            })
            .when(clickable, |this| {
                this.cursor_pointer()
                    .when(!active, |this| {
                        this.hover(|this| this.bg(palette::row_hover()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.open_entry(&on_open, cx)))
            })
            .when_some(dragged, |this, id| {
                // The ghost is the title alone: dragging a three-line card
                // across the panel hides the row it is aimed at.
                this.on_drag(DraggedBook(id), move |_, _, _, cx| {
                    cx.new(|_| DragGhost(label.clone()))
                })
            })
            .when_some(menu_for, |this, id| {
                this.on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_shelf_menu(&id, event.position, cx);
                    }),
                )
            });

        let row = if entry.is_compact() {
            row.child(render_compact(entry, active))
        } else {
            row.child(render_full(entry))
        };

        match entry.removable {
            true => row.child(self.render_remove_button(entry, cx)),
            false => row,
        }
    }

    /// The remove affordance on a row, which asks before it does anything.
    ///
    /// Removing a book takes its marks and its conversations with it, so the
    /// first press turns the button into the question and the second answers
    /// it. A press anywhere else changes its mind.
    fn render_remove_button(
        &self,
        entry: &Entry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let confirming = self.is_confirming(&entry.id);
        let on_remove = entry.clone();

        div()
            .id(SharedString::from(format!("remove:{}", entry.id)))
            .absolute()
            .top(px(6.))
            .right(px(6.))
            .px(px(7.))
            .h(px(20.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .when(!confirming, |this| {
                this.invisible()
                    .group_hover(GROUP, |this| this.visible())
                    .bg(palette::surface())
            })
            .when(confirming, |this| this.bg(palette::danger().opacity(0.24)))
            .hover(|this| this.bg(palette::danger().opacity(0.36)))
            .text_size(px(10.))
            .text_color(if confirming {
                palette::danger()
            } else {
                palette::text_muted()
            })
            .child(if confirming { "Remove?" } else { "Remove" })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.remove_entry(&on_remove, cx);
            }))
    }

    /// The menu of shelves a book can be put on.
    ///
    /// Drawn over the whole panel rather than inside the row, so a library with
    /// eight shelves is not clipped to the height of one row.
    pub(crate) fn render_shelf_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let menu = self.shelf_menu.clone()?;
        let book = self
            .library
            .books()
            .iter()
            .find(|book| book.id == menu.book_id.as_ref())?;

        let on_a_shelf = book.folder_id.clone();
        let shelves = self.library.shelves();

        let mut items: Vec<gpui::AnyElement> = shelves
            .iter()
            .map(|shelf| {
                let here = on_a_shelf.as_deref() == Some(shelf.id.as_str());
                let id = SharedString::from(shelf.id.clone());
                let book_id = menu.book_id.clone();

                self.render_menu_item(
                    SharedString::from(format!("shelf-menu:{}", shelf.id)),
                    self.library.path_of(&shelf.id),
                    here,
                    cx.listener(
                        move |this: &mut Pedro,
                              _: &gpui::ClickEvent,
                              _: &mut Window,
                              cx: &mut Context<Pedro>| {
                            cx.stop_propagation();
                            this.shelve_book(&book_id, Some(id.as_ref()), cx);
                            this.close_shelf_menu(cx);
                        },
                    ),
                )
                .into_any_element()
            })
            .collect();

        if on_a_shelf.is_some() {
            let book_id = menu.book_id.clone();
            items.push(
                self.render_menu_item(
                    "shelf-menu:none".into(),
                    "Take off the shelf".into(),
                    false,
                    cx.listener(
                        move |this: &mut Pedro,
                              _: &gpui::ClickEvent,
                              _: &mut Window,
                              cx: &mut Context<Pedro>| {
                            cx.stop_propagation();
                            this.shelve_book(&book_id, None, cx);
                            this.close_shelf_menu(cx);
                        },
                    ),
                )
                .into_any_element(),
            );
        }

        if shelves.is_empty() {
            items.push(
                self.render_menu_item(
                    "shelf-menu:new".into(),
                    "New shelf".into(),
                    false,
                    cx.listener(
                        |this: &mut Pedro,
                         _: &gpui::ClickEvent,
                         window: &mut Window,
                         cx: &mut Context<Pedro>| {
                            cx.stop_propagation();
                            this.create_shelf(None, window, cx);
                            this.close_shelf_menu(cx);
                        },
                    ),
                )
                .into_any_element(),
            );
        }

        Some(
            div()
                .id("shelf-menu")
                .absolute()
                .left(menu.at.x)
                .top(menu.at.y)
                .w(px(200.))
                .p(px(4.))
                .rounded(px(10.))
                .bg(palette::surface())
                .border_1()
                .border_color(palette::border())
                .shadow_lg()
                .child(
                    div()
                        .px(px(8.))
                        .py(px(4.))
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child("Put on a shelf"),
                )
                .children(items),
        )
    }

    fn render_menu_item(
        &self,
        id: SharedString,
        label: SharedString,
        checked: bool,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        h_flex()
            .id(id)
            .h(px(28.))
            .px(px(8.))
            .gap(px(6.))
            .items_center()
            .rounded(px(7.))
            .cursor_pointer()
            .hover(|this| this.bg(palette::row_hover()))
            .child(
                div()
                    .w(px(14.))
                    .children(checked.then(|| icon(IconName::Check, px(12.), palette::accent()))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(palette::text())
                    .child(label),
            )
            .on_click(on_click)
    }

    /// Which agent is answering, at the foot of the panel.
    ///
    /// The reference design puts the signed-in account here. Pedro has no
    /// account; the equivalent fact — whose credentials are about to answer a
    /// question — is the CLI it found.
    fn render_agent_footer(&self) -> impl IntoElement + use<> {
        let (name, detail): (SharedString, SharedString) =
            match (&self.agent_status, self.answering_agent()) {
                (AgentStatus::Detecting, _) => ("Looking…".into(), "for an agent CLI".into()),
                (_, Some(agent)) => (
                    agent.kind.display_name().into(),
                    agent
                        .version
                        .clone()
                        .unwrap_or_else(|| "installed".to_owned())
                        .into(),
                ),
                (_, None) => ("No agent".into(), "install claude or codex".into()),
            };

        // The first letter, the way the reference design badges an account.
        let initial: SharedString = name.chars().next().unwrap_or('?').to_string().into();

        h_flex()
            .h(px(58.))
            .px(px(14.))
            .gap(px(10.))
            .items_center()
            .flex_shrink_0()
            .border_t_1()
            .border_color(palette::separator())
            .child(
                div()
                    .size(px(30.))
                    .rounded_full()
                    .bg(palette::accent())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(palette::text())
                    .child(initial),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(palette::text())
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(detail),
                    ),
            )
    }
}

/// The three-line shape: where it came from, what it is, what file it is in.
/// `use<>` because the row is stored as an `AnyElement`, which is `'static`:
/// everything here is cloned out of the entry rather than borrowed from it,
/// and edition 2024 would otherwise capture the lifetime anyway.
fn render_full(entry: &Entry) -> impl IntoElement + use<> {
    v_flex()
        .px(px(10.))
        .py(px(8.))
        .gap(px(3.))
        .child(
            h_flex()
                .gap(px(8.))
                .items_center()
                .children(entry.meta.clone().map(|meta| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child(meta)
                }))
                .child(render_trailing(entry)),
        )
        .child(
            div()
                .truncate()
                .text_size(px(14.))
                .text_color(palette::text())
                .child(entry.label.clone()),
        )
        .children(entry.detail.clone().map(|detail| {
            h_flex()
                .gap(px(6.))
                .items_center()
                .child(icon(entry.icon.clone(), px(12.), palette::text_faint()))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child(detail),
                )
        }))
}

/// The one-line shape, for lists long enough that three lines each would be a
/// wall: an archive, a table of contents, the agents that were found.
fn render_compact(entry: &Entry, active: bool) -> impl IntoElement + use<> {
    h_flex()
        .h(px(34.))
        .px(px(10.))
        .gap(px(9.))
        .items_center()
        .child(icon(
            entry.icon.clone(),
            px(14.),
            if active {
                palette::text()
            } else {
                palette::text_muted()
            },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .text_color(palette::text())
                .child(entry.label.clone()),
        )
        .child(render_trailing(entry))
}

/// The right-hand end of a row: a status if it has one, an age or a page if it
/// does not, and nothing at all otherwise.
fn render_trailing(entry: &Entry) -> impl IntoElement + use<> {
    let status = entry.status;
    let trailing = entry.trailing.clone();

    h_flex()
        .flex_shrink_0()
        .gap(px(6.))
        .items_center()
        .children(status.map(|status| {
            div()
                .size(px(6.))
                .rounded_full()
                .bg(status_color(status))
                .into_any_element()
        }))
        .children(status.map(|status| {
            div()
                .text_size(px(11.))
                .text_color(status_color(status))
                .child(status.label())
                .into_any_element()
        }))
        .children(trailing.filter(|_| status.is_none()).map(|trailing| {
            div()
                .text_size(px(11.))
                .text_color(palette::text_faint())
                .child(trailing)
                .into_any_element()
        }))
}

fn status_color(status: Status) -> Hsla {
    match status {
        Status::Working => palette::working(),
        Status::Failed => palette::danger(),
    }
}

/// A book on its way to a shelf.
#[derive(Clone)]
pub(crate) struct DraggedBook(pub SharedString);

/// A shelf on its way onto another one, or back to the top level.
#[derive(Clone)]
pub(crate) struct DraggedShelf(pub SharedString);

/// What follows the pointer while a book is being dragged.
struct DragGhost(SharedString);

impl gpui::Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(6.))
            .max_w(px(240.))
            .truncate()
            .rounded(px(8.))
            .bg(palette::surface())
            .border_1()
            .border_color(palette::border())
            .shadow_lg()
            .text_size(px(12.))
            .text_color(palette::text())
            .child(self.0.clone())
    }
}
