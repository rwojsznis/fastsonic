//! Playlist, album, and Liked Songs pages: a hero, actions, and a track table.

use std::sync::Arc;

use egui::{Align, Layout, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::{Album, PlayableItem, Playlist, pick_image};
use crate::app::App;
use crate::model::{
    Action, Dialog, DragTrack, Loadable, Page, PagedList, RowContext, RowPick, SortColumn,
    TableItem, TableRowsCache, TableSort,
};
use crate::theme::{self, Icon, Palette};
use crate::util;

use super::widgets::{self, TrackRow};

pub struct Hero<'a> {
    pub image: Option<&'a str>,
    pub liked: bool,
    pub kind: &'a str,
    pub title: &'a str,
    pub description: Option<String>,
    pub byline: Vec<(String, Option<Page>)>,
    pub round: bool,
}

pub fn hero(app: &mut App, ui: &mut egui::Ui, hero: Hero<'_>) {
    let palette = app.palette;
    ui.add_space(12.0);
    let cover_size = if ui.available_width() > 720.0 {
        212.0
    } else {
        160.0
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(cover_size), Sense::hover());
        let radius = if hero.round { cover_size / 2.0 } else { 6.0 };
        widgets::paint_shadow(ui, &palette, rect, radius);
        if hero.liked {
            super::sidebar::liked_cover(ui, rect, radius);
        } else {
            widgets::paint_cover(
                ui,
                &palette,
                hero.image,
                rect,
                radius,
                if hero.round { Icon::User } else { Icon::Music },
                Some(app.backend.art()),
            );
        }
        ui.vertical(|ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(cover_size * 0.08);
            theme::text(ui, hero.kind, theme::medium(12.5), palette.text);
            let mut size = if cover_size > 200.0 { 56.0 } else { 40.0 };
            // Measured on the display text: the same glyphs, in the order
            // they are drawn.
            let display_title = crate::bidi::display_text(hero.title);
            loop {
                let galley = ui.painter().layout_no_wrap(
                    display_title.to_string(),
                    theme::bold(size),
                    palette.text,
                );
                if galley.size().x <= width || size <= 22.0 {
                    break;
                }
                size -= 6.0;
            }
            theme::text(ui, hero.title, theme::bold(size), palette.text);
            if let Some(description) = &hero.description
                && !description.is_empty()
            {
                theme::text(
                    ui,
                    description.as_str(),
                    theme::regular(13.5),
                    palette.secondary,
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, (text, page)) in hero.byline.iter().enumerate() {
                    if index > 0 {
                        theme::text(ui, "•", theme::regular(13.5), palette.secondary);
                    }
                    match page {
                        Some(page) => {
                            if theme::link(ui, text, theme::semibold(13.5), palette.text).clicked()
                            {
                                app.actions.push(Action::Open(page.clone()));
                            }
                        }
                        None => {
                            theme::text(ui, text, theme::regular(13.5), palette.secondary);
                        }
                    }
                }
            });
        });
    });
    ui.add_space(20.0);
}

pub struct Actions<'a> {
    pub play_uri: Option<String>,
    /// A sorted or filtered view: the exact list on screen, which the big
    /// button plays instead of the context's own order.
    pub view: Option<Arc<[String]>>,
    pub saved: Option<(String, bool)>,
    pub saved_icons: (Icon, Icon),
    pub saved_tooltips: (&'a str, &'a str),
    pub owned_playlist: Option<Playlist>,
    /// A playlist page can be refreshed from its More menu, and says
    /// whether it is loading.
    pub reload: Option<(Page, bool)>,
    pub name: &'a str,
}

/// The big play button and its neighbours; returns the filter text if a
/// filter field was shown.
pub fn actions_row(
    app: &mut App,
    ui: &mut egui::Ui,
    actions: Actions<'_>,
    filter: Option<&mut String>,
) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        if let Some(uri) = &actions.play_uri {
            let now_playing_here = app.playing_context_uri().as_deref() == Some(uri.as_str())
                && app.believed_playing();
            let icon = if now_playing_here {
                Icon::PauseFilled
            } else {
                Icon::PlayFilled
            };
            // A filter that matches nothing has nothing to play, and must
            // not fall back to playing everything it hid.
            let can_start = actions.view.as_ref().is_none_or(|uris| !uris.is_empty());
            if app.play_pending(uri) {
                theme::circle_spinner(ui, 56.0, palette.accent, palette.on_accent, "Starting…");
            } else if ui
                .add_enabled_ui(now_playing_here || can_start, |ui| {
                    theme::circle_button(
                        ui,
                        icon,
                        56.0,
                        palette.accent,
                        palette.accent_hover,
                        palette.on_accent,
                        if now_playing_here { "Pause" } else { "Play" },
                    )
                })
                .inner
                .on_disabled_hover_text("No songs in this view")
                .clicked()
            {
                let is_filtered = filter.as_ref().is_some_and(|f| !f.trim().is_empty());
                if now_playing_here {
                    app.actions.push(Action::TogglePlay);
                } else if let Some(uris) = actions.view.clone()
                    && should_play_view(app.playing_context_shuffle(), is_filtered)
                {
                    app.actions.push(Action::PlayFromRow {
                        context: RowContext::View {
                            uris: Arc::clone(&uris),
                            context_uri: uri.clone(),
                            // Header playback needs no edit rights; row
                            // menus carry theirs via the table conversion.
                            editable_playlist: None,
                        },
                        uri: String::new(),
                        index: 0,
                    });
                } else {
                    app.actions.push(Action::PlayContext {
                        uri: uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    });
                }
            }
            // The mode, not a way to start this collection: chosen here it
            // applies to whatever plays, and to the next Play if nothing does.
            let shuffle = app.playing_context_shuffle();
            if theme::icon_button(
                ui,
                Icon::Shuffle,
                26.0,
                if shuffle {
                    palette.accent
                } else {
                    palette.secondary
                },
                palette.text,
                if shuffle { "Shuffle off" } else { "Shuffle" },
            )
            .clicked()
            {
                app.actions.push(Action::SetShuffle(!shuffle));
            }
        }
        if let Some((uri, saved)) = &actions.saved {
            let (icon, tooltip, color) = if *saved {
                (
                    actions.saved_icons.1,
                    actions.saved_tooltips.1,
                    palette.accent,
                )
            } else {
                (
                    actions.saved_icons.0,
                    actions.saved_tooltips.0,
                    palette.secondary,
                )
            };
            if theme::icon_button(ui, icon, 26.0, color, palette.text, tooltip).clicked() {
                app.actions.push(Action::ToggleSaved(uri.clone()));
            }
        }
        if let Some(uri) = &actions.play_uri {
            let more = theme::icon_button(
                ui,
                Icon::Ellipsis,
                26.0,
                palette.secondary,
                palette.text,
                "More",
            );
            egui::Popup::menu(&more)
                .frame(widgets::menu_frame(&palette))
                .show(|ui| {
                    widgets::context_menu_items(
                        ui,
                        app,
                        uri,
                        actions.name,
                        actions.owned_playlist.as_ref(),
                    );
                    if let Some((page, loading)) = &actions.reload {
                        widgets::menu_separator(ui, &palette);
                        let clicked = ui
                            .add_enabled_ui(!loading, |ui| {
                                widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Refresh),
                                    if *loading { "Refreshing…" } else { "Refresh" },
                                )
                            })
                            .inner;
                        if clicked {
                            app.actions.push(Action::Reload(page.clone()));
                        }
                    }
                });
        }
        if let Some(filter) = filter {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::search_field(
                    ui,
                    &palette,
                    egui::Id::new(("collection-filter", actions.name)),
                    filter,
                    "Filter",
                    220.0,
                );
            });
        }
    });
    ui.add_space(14.0);
}

/// A filtered collection has no server-side equivalent, so shuffle must use
/// its visible rows. A merely sorted collection lets the engine shuffle the
/// whole context instead of freezing the displayed order into a list.
fn should_play_view(shuffling: bool, filtered: bool) -> bool {
    !shuffling || filtered
}

/// A track table with virtualised rows and paging.
pub struct Table<'a> {
    pub items: &'a [TableItem],
    pub context: RowContext,
    pub show_album: bool,
    pub show_cover: bool,
    pub show_added: bool,
    pub show_added_by: bool,
    pub page: Page,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub can_load_more: bool,
    pub filter: &'a str,
    pub items_revision: u64,
}

#[derive(Clone)]
pub struct TableCache {
    pub sort: Option<TableSort>,
    pub needle: String,
    pub items_revision: u64,
    pub user_names_revision: u64,
    pub visible: Arc<[usize]>,
    pub view_uris: Option<Arc<[String]>>,
}

pub fn table_items_hit(
    app: &App,
    page: &Page,
    generation: u64,
    items_revision: u64,
    user_names_revision: u64,
) -> Option<Arc<[TableItem]>> {
    app.table_rows.get(page).and_then(|cached| {
        (cached.generation == generation
            && cached.items_revision == items_revision
            && cached.user_names_revision == user_names_revision)
            .then(|| Arc::clone(&cached.items))
    })
}

pub fn remember_table_items(
    app: &mut App,
    page: Page,
    generation: u64,
    items_revision: u64,
    user_names_revision: u64,
    items: Vec<TableItem>,
) -> Arc<[TableItem]> {
    let items: Arc<[TableItem]> = items.into();
    app.table_rows.insert(
        page.clone(),
        TableRowsCache {
            generation,
            items_revision,
            user_names_revision,
            items: Arc::clone(&items),
        },
    );
    app.retain_table_rows(&page);
    items
}

/// Cached table rows for one page. Rebuilt only when the source list,
/// contributor names, or page generation change, not every frame.
///
/// The cache lives on `App`, not in egui temp data. It is dropped when the
/// page is evicted, when the account resets, and when more than two tables
/// would be retained. Recreated pages get a new generation, so an old
/// revision number cannot resurrect stale rows.
pub fn cached_table_items(
    app: &mut App,
    page: Page,
    generation: u64,
    items_revision: u64,
    user_names_revision: u64,
    build: impl FnOnce() -> Vec<TableItem>,
) -> Arc<[TableItem]> {
    if let Some(items) =
        table_items_hit(app, &page, generation, items_revision, user_names_revision)
    {
        app.retain_table_rows(&page);
        return items;
    }
    remember_table_items(
        app,
        page,
        generation,
        items_revision,
        user_names_revision,
        build(),
    )
}

pub fn prepare_table_view(
    ui: &mut egui::Ui,
    app: &App,
    page: &Page,
    items: &[TableItem],
    needle: &str,
    sort: Option<TableSort>,
    items_revision: u64,
) -> Arc<TableCache> {
    let cache_id = egui::Id::new("table-view-cache").with(page);
    let cached = ui.data(|d| d.get_temp::<Arc<TableCache>>(cache_id));

    let is_valid = cached.as_ref().is_some_and(|c| {
        c.sort == sort
            && c.needle == needle
            && c.items_revision == items_revision
            && c.user_names_revision == app.user_names_revision
    });

    if let Some(entry) = cached.filter(|_| is_valid) {
        entry
    } else {
        let visible = view_indices(items, needle, sort);
        let view_uris = view_uris(items, &visible, needle, sort);
        let entry = Arc::new(TableCache {
            sort,
            needle: needle.to_string(),
            items_revision,
            user_names_revision: app.user_names_revision,
            visible: visible.into(),
            view_uris,
        });
        ui.data_mut(|d| d.insert_temp(cache_id, Arc::clone(&entry)));
        entry
    }
}

/// The songs a sorted or filtered view plays, in the order it shows them.
/// A filter is a view too: playing it plays the matching songs, not the
/// whole collection they were found in.
fn view_uris(
    items: &[TableItem],
    visible: &[usize],
    needle: &str,
    sort: Option<TableSort>,
) -> Option<Arc<[String]>> {
    (sort.is_some() || !needle.is_empty()).then(|| {
        visible
            .iter()
            .map(|&index| items[index].0.uri().to_string())
            .collect()
    })
}

/// Select all, Cut, Copy, Paste and Delete on a song list. Cut copies the
/// picked songs and removes them from a playlist the account can edit,
/// Delete only removes them, and Paste adds copied songs to one. A focused text field keeps these keys for its
/// own text, and an open dialog keeps them from the list behind it.
fn list_shortcuts(
    ui: &egui::Ui,
    app: &mut App,
    table: &Table<'_>,
    view: &str,
    visible: &[usize],
    picked_songs: &[(String, String)],
) {
    if ui.ctx().text_edit_focused() || app.dialog.is_some() {
        return;
    }
    let editable = match &table.context {
        RowContext::Context {
            editable_playlist: Some((id, _)),
            ..
        }
        | RowContext::View {
            editable_playlist: Some((id, _)),
            ..
        } => Some(id.clone()),
        _ => None,
    };
    let can_delete = editable.is_some()
        && app.picked_rows(&table.page).is_some()
        && !egui::Popup::is_any_open(ui.ctx());
    let (select_all, cut, copy, pasted, delete) = ui.input_mut(|input| {
        // The platform's Cut, Copy and Paste keys arrive as these events,
        // not as key presses.
        let cut = editable.is_some()
            && !picked_songs.is_empty()
            && input.events.contains(&egui::Event::Cut);
        let copy = !picked_songs.is_empty() && input.events.contains(&egui::Event::Copy);
        let pasted = editable.as_ref().and_then(|_| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
        });
        input.events.retain(|event| match event {
            egui::Event::Cut => !cut,
            egui::Event::Copy => !copy,
            egui::Event::Paste(_) => pasted.is_none(),
            _ => true,
        });
        (
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::A),
            cut,
            copy,
            pasted,
            can_delete
                && (input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || (cfg!(target_os = "macos")
                        && input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))),
        )
    });
    if select_all {
        let all = (0..visible.len())
            .filter(|row| {
                table
                    .items
                    .get(visible[*row])
                    .is_some_and(|(item, _, _)| !item.uri().is_empty())
            })
            .collect();
        app.pick_rows(&table.page, view, all);
    }
    let uris = || picked_songs.iter().map(|(uri, _)| uri.clone()).collect();
    if let (true, Some(playlist_id)) = (delete, &editable) {
        app.actions.push(Action::RemoveFromPlaylist {
            playlist_id: playlist_id.clone(),
            uris: uris(),
        });
    }
    if let (true, Some(playlist_id)) = (cut, &editable) {
        app.actions.push(Action::CopySongs(uris()));
        app.actions.push(Action::RemoveFromPlaylist {
            playlist_id: playlist_id.clone(),
            uris: uris(),
        });
    } else if copy {
        app.actions.push(Action::CopySongs(uris()));
    }
    if let (Some(playlist_id), Some(text)) = (editable, pasted) {
        app.actions.push(Action::PasteSongs { playlist_id, text });
    }
}

/// Arrow keys follow display order, independent of the positions of the
/// artist links, heart buttons and other controls inside each song row.
/// Answers the rows it moved from and to, and whether Shift was held to
/// extend the selection.
fn navigate_song_rows(
    ui: &egui::Ui,
    rows: &[(usize, egui::Response)],
) -> Option<(usize, usize, bool)> {
    if egui::Popup::is_any_open(ui.ctx()) {
        return None;
    }
    let current = rows.iter().position(|(_, response)| response.has_focus())?;
    let (down, up, extend) = ui.input_mut(|input| {
        // Consume Shift first: egui's plain-key matcher also accepts Shift.
        let down = input.count_and_consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
        let up = input.count_and_consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
        let extend = down + up > 0;
        (
            down + input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            up + input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            extend,
        )
    });
    if down + up == 0 {
        return None;
    }
    let movement = down as isize - up as isize;
    let next = current.saturating_add_signed(movement).min(rows.len() - 1);
    // Cancel egui's spatial search at the end of this pass, including on the
    // first key after gaining focus. Tab still reaches child controls.
    ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    rows[next].1.request_focus();
    rows[next].1.scroll_to_me(None);
    ui.ctx().request_repaint();
    Some((rows[current].0, rows[next].0, extend))
}

fn view_context(base: &RowContext, view_uris: Option<&Arc<[String]>>) -> RowContext {
    if let Some(uris) = view_uris {
        match base {
            RowContext::Context {
                uri,
                editable_playlist,
            } => RowContext::View {
                uris: Arc::clone(uris),
                context_uri: uri.clone(),
                editable_playlist: editable_playlist.clone(),
            },
            _ => RowContext::Uris(Arc::clone(uris)),
        }
    } else {
        base.clone()
    }
}

pub fn table(app: &mut App, ui: &mut egui::Ui, table: Table<'_>) {
    let palette = app.palette;
    let needle = table.filter.trim().to_lowercase();
    let sort = app.table_sorts.get(&table.page).copied();
    let entry = prepare_table_view(
        ui,
        app,
        &table.page,
        table.items,
        &needle,
        sort,
        table.items_revision,
    );
    let thin = app.settings.tracklist_compact;
    let show_cover = !thin && table.show_cover;
    let row_height = if thin {
        theme::THIN_ROW_HEIGHT
    } else {
        theme::ROW_HEIGHT
    };

    if !table.items.is_empty()
        && let Some(column) = widgets::table_header(
            ui,
            &palette,
            table.show_album,
            table.show_added,
            table.show_added_by,
            show_cover,
            sort,
        )
    {
        // Ascending, descending, back to the list's own order.
        let next = match sort {
            Some(sort) if sort.column == column && sort.ascending => Some(TableSort {
                column,
                ascending: false,
            }),
            Some(sort) if sort.column == column => None,
            // The # stands for the list's own order: from any other sort
            // it returns there rather than layering a sort of its own.
            Some(_) if column == SortColumn::Index => None,
            // Ascending by # is the list's own order, a click that would
            // change nothing; the first click on # reverses instead.
            _ => Some(TableSort {
                column,
                ascending: column != SortColumn::Index,
            }),
        };
        match next {
            Some(sort) => {
                app.table_sorts.insert(table.page.clone(), sort);
                app.note_session_change();
                // A sort covers the whole list, so the rest must load.
                app.actions.push(Action::LoadMore(table.page.clone()));
            }
            None => {
                app.table_sorts.remove(&table.page);
                app.note_session_change();
            }
        }
    }
    // What is displayed is what plays: a sorted view plays in its own
    // order, as a plain list of tracks, and its rows cannot edit server
    // positions that no longer match the screen.
    let context = view_context(&table.context, entry.view_uris.as_ref());
    let sorted = sort.is_some();
    // Positional playlist edits need the rows on screen to match the
    // server's order.
    let move_playlist = (sort.is_none() && needle.is_empty())
        .then(|| match &table.context {
            RowContext::Context {
                editable_playlist: Some((id, _)),
                ..
            } => Some(id.clone()),
            _ => None,
        })
        .flatten();
    if move_playlist.is_some() && egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx()) {
        widgets::scroll_during_drag(ui);
    }
    // Calculate the nearest drop slot from fixed row height because virtualized
    // rows are not all available during drawing.
    let list_top = ui.cursor().top();
    let move_slot = move_playlist.as_ref().and_then(|_| {
        egui::DragAndDrop::payload::<DragTrack>(ui.ctx())?;
        let pos = ui
            .ctx()
            .pointer_latest_pos()
            .filter(|pos| ui.clip_rect().contains(*pos))?;
        let row = (pos.y - list_top) / row_height;
        // The blank space after the last row takes an append, which is
        // also how an empty playlist gets its first song.
        (row >= 0.0).then(|| (row.round() as usize).min(entry.visible.len()))
    });
    // Selection uses display indices. Clear it when the view or items change,
    // including a refresh that replaces songs without changing the row count.
    let view = format!(
        "{sort:?}|{needle}|{}|{}",
        entry.visible.len(),
        table.items_revision
    );
    app.keep_picked_rows_for(&table.page, &view);
    let picked: std::collections::BTreeSet<usize> =
        app.picked_rows(&table.page).cloned().unwrap_or_default();
    // Keep names with URIs for immediate optimistic queue rows.
    let picked_songs: Vec<(String, String)> = picked
        .iter()
        .filter_map(|row| entry.visible.get(*row))
        .filter_map(|index| table.items.get(*index))
        .map(|(item, _, _)| (item.uri().to_string(), item.name().to_string()))
        .collect();
    let rows = entry.visible.len();
    let mut pick = None;
    let mut row_responses = Vec::new();
    widgets::virtual_rows(ui, entry.visible.len(), row_height, |ui, row| {
        let index = entry.visible[row];
        let (item, added_at, added_by) = &table.items[index];
        // Shift neighboring rows around the current drop slot.
        let shift = ui.ctx().animate_value_with_time(
            ui.id().with(("table-move-shift", row)),
            match move_slot {
                Some(slot) if row < slot => -4.0,
                Some(_) => 4.0,
                None => 0.0,
            },
            0.12,
        );
        let (response, asked) = widgets::track_row_response(
            ui,
            app,
            TrackRow {
                index: if entry.view_uris.is_some() {
                    row
                } else {
                    index
                },
                number: Some(if sorted { row + 1 } else { index + 1 }),
                item,
                context: &context,
                show_cover,
                show_album: table.show_album,
                added_at: added_at.as_deref(),
                added_by: added_by.as_deref(),
                show_added_by: table.show_added_by,
                compact: false,
                thin,
                shift,
                picked: picked.contains(&row),
                picked_songs: &picked_songs,
            },
        );
        row_responses.push((row, response));
        if let Some(asked) = asked {
            pick = Some((row, asked));
        }
    });
    if app.dialog.is_none()
        && let Some((current, next, extend)) = navigate_song_rows(ui, &row_responses)
    {
        if extend {
            if app.picked_rows(&table.page).is_none() {
                app.pick_rows(&table.page, &view, [current].into_iter().collect());
            }
            app.pick_row(&table.page, &view, next, RowPick::Range, rows);
        } else {
            app.pick_rows(&table.page, &view, [next].into_iter().collect());
        }
    }
    if let Some((row, asked)) = pick {
        app.pick_row(&table.page, &view, row, asked, rows);
    }
    list_shortcuts(ui, app, &table, &view, &entry.visible, &picked_songs);
    // Escape clears the current selection.
    if !picked.is_empty() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.clear_picked_rows();
    }
    if let Some(slot) = move_slot {
        // Draw the destination line between shifted rows.
        let y = list_top + slot as f32 * row_height;
        ui.painter().hline(
            ui.max_rect().x_range().shrink(8.0),
            y,
            egui::Stroke::new(2.0, palette.accent),
        );
        if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
            && let Some(track) = egui::DragAndDrop::take_payload::<DragTrack>(ui.ctx())
            && let Some(playlist_id) = move_playlist
        {
            let to = slot as u32;
            match &track.from {
                // A row of this playlist moves. The slot is an
                // insert-before position, exactly what the action's handler
                // sends; a row dropped back on its own edges moves nothing.
                Some((origin, from)) if *origin == playlist_id => {
                    if to != *from && to != from.saturating_add(1) {
                        app.actions.push(Action::MoveInPlaylist {
                            playlist_id,
                            from: *from,
                            to,
                        });
                    }
                }
                // Anything else is a copy, and the source keeps its song.
                _ => app.actions.push(Action::InsertInPlaylist {
                    playlist_id,
                    position: to,
                    items: track.items.clone(),
                }),
            }
        }
    }
    if table.loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette);
    }
    if let Some(error) = table.error {
        ui.add_space(8.0);
        widgets::error_row(ui, app, error, Some(table.page.clone()));
    }
    if table.items.is_empty() && !table.loading && table.error.is_none() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Music,
            "Nothing here yet",
            "Added songs appear here.",
        );
    } else if entry.visible.is_empty()
        && !needle.is_empty()
        && table.can_load_more
        && !table.loading
    {
        // Filtering a partially loaded list: keep fetching so matches appear.
        app.actions.push(Action::LoadMore(table.page));
    } else {
        widgets::load_more_when_near_end(
            ui,
            app,
            table.page,
            table.can_load_more && !table.loading,
        );
    }
}

fn sort_by_text_key(visible: &mut [usize], ascending: bool, key: impl Fn(usize) -> String) {
    if ascending {
        visible.sort_by_cached_key(|&index| key(index));
    } else {
        visible.sort_by_cached_key(|&index| std::cmp::Reverse(key(index)));
    }
}

/// The indices of `items` as a view presents them: filtered by `needle`
/// (already lowercased), then ordered by `sort`.
fn view_indices(items: &[TableItem], needle: &str, sort: Option<TableSort>) -> Vec<usize> {
    let mut visible: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, (item, _, _))| {
            if needle.is_empty() {
                return true;
            }
            let haystack = match item {
                PlayableItem::Track(track) => format!(
                    "{} {} {}",
                    track.name,
                    track.artist_names(),
                    track
                        .album
                        .as_ref()
                        .map(|album| album.name.as_str())
                        .unwrap_or("")
                ),
            };
            haystack.to_lowercase().contains(needle)
        })
        .map(|(index, _)| index)
        .collect();
    if let Some(sort) = sort {
        match sort.column {
            SortColumn::Title => sort_by_text_key(&mut visible, sort.ascending, |index| {
                items[index].0.name().to_lowercase()
            }),
            SortColumn::Album => sort_by_text_key(&mut visible, sort.ascending, |index| {
                let PlayableItem::Track(track) = &items[index].0;
                track
                    .album
                    .as_ref()
                    .map(|album| album.name.to_lowercase())
                    .unwrap_or_default()
            }),
            SortColumn::AddedBy => sort_by_text_key(&mut visible, sort.ascending, |index| {
                items[index].2.as_deref().unwrap_or_default().to_lowercase()
            }),
            SortColumn::Added | SortColumn::Index | SortColumn::Duration => {
                visible.sort_by(|a, b| {
                    let ordering = match sort.column {
                        SortColumn::Added => items[*a].1.cmp(&items[*b].1),
                        SortColumn::Index => a.cmp(b),
                        SortColumn::Duration => {
                            items[*a].0.duration_ms().cmp(&items[*b].0.duration_ms())
                        }
                        _ => unreachable!("text columns are handled above"),
                    };
                    if sort.ascending {
                        ordering
                    } else {
                        ordering.reverse()
                    }
                });
            }
        }
    }
    visible
}

fn total_duration(items: &[TableItem]) -> u64 {
    items
        .iter()
        .map(|(item, _, _)| item.duration_ms() as u64)
        .sum()
}

fn items_of(
    list: &PagedList<crate::api::models::PlaylistItem>,
    owner_id: Option<&str>,
    owner_name: &str,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Vec<TableItem> {
    list.items
        .iter()
        .filter_map(|item| {
            let playable = item.playable().cloned()?;
            let adder = item
                .added_by
                .as_ref()
                .and_then(|user| user.id.as_deref())
                .map(|id| {
                    if Some(id) == owner_id {
                        owner_name.to_string()
                    } else {
                        names
                            .get(id)
                            .and_then(|name| name.clone())
                            .unwrap_or_else(|| id.to_string())
                    }
                });
            Some((playable, item.added_at.clone(), adder))
        })
        .collect()
}

/// A complete, ranked view of the listener's current top tracks.
pub fn top_songs(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(12.0);
    theme::text(ui, "Your top songs", theme::bold(30.0), palette.text);
    ui.add_space(4.0);
    theme::text(
        ui,
        "Your most-played tracks from the last four weeks.",
        theme::regular(13.5),
        palette.secondary,
    );
    ui.add_space(18.0);

    let tracks = match &app.home.top_songs {
        Loadable::Loaded(tracks) => tracks,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::loading_row(ui, &palette);
            return;
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::TopSongs));
            return;
        }
    };
    let generation = app.home.top_songs_generation;
    let names = app.user_names_revision;
    let items =
        if let Some(items) = table_items_hit(app, &Page::TopSongs, generation, generation, names) {
            items
        } else {
            let rows = tracks
                .iter()
                .cloned()
                .map(|track| (PlayableItem::Track(track), None, None))
                .collect();
            remember_table_items(app, Page::TopSongs, generation, generation, names, rows)
        };
    let uris: Arc<[String]> = items
        .iter()
        .map(|(item, _, _)| item.uri().to_string())
        .collect::<Vec<_>>()
        .into();
    table(
        app,
        ui,
        Table {
            items: &items,
            context: RowContext::Uris(Arc::clone(&uris)),
            show_album: true,
            show_cover: true,
            show_added: false,
            show_added_by: false,
            page: Page::TopSongs,
            loading: app.home.top_songs_loading,
            error: None,
            can_load_more: false,
            filter: "",
            items_revision: app.home.top_songs_generation,
        },
    );
}

pub fn playlist(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let Some(mut page) = app.playlist_pages.remove(id) else {
        app.ensure_loaded(Page::Playlist(id.to_string()));
        return;
    };
    let palette = app.palette;
    let user_id = app.user_id().unwrap_or("").to_string();
    match &page.playlist {
        Loadable::Loaded(playlist) => {
            let generation = page.generation;
            let revision = page.items.revision;
            let names = app.user_names_revision;
            let key = Page::Playlist(id.to_string());
            let items = if let Some(items) = table_items_hit(app, &key, generation, revision, names)
            {
                items
            } else {
                let rows = items_of(
                    &page.items,
                    playlist.owner.id.as_deref(),
                    playlist.owner_name(),
                    &app.user_names,
                );
                remember_table_items(app, key, generation, revision, names, rows)
            };
            let count = playlist.track_total().max(items.len() as u32);
            let owner_id = playlist.owner.id.as_deref();
            let others = page
                .contributors
                .iter()
                .filter(|id| !id.is_empty() && Some(id.as_str()) != owner_id)
                .count();
            let made_together = playlist.collaborative || others > 0;
            let mut byline = vec![(playlist.owner_name().to_string(), None)];
            if others > 0 {
                let named: Vec<String> = page
                    .contributors
                    .iter()
                    .filter(|id| Some(id.as_str()) != owner_id)
                    .filter_map(|id| app.user_names.get(id)?.clone())
                    .collect();
                byline.push((
                    if named.len() == others && others <= 2 {
                        format!("with {}", named.join(" and "))
                    } else if others == 1 {
                        "and 1 other".to_string()
                    } else {
                        format!("and {others} others")
                    },
                    None,
                ));
            }
            let count_text = if page.items.is_complete() {
                format!(
                    "{} songs, {}",
                    util::format_count(count as u64),
                    util::format_total_ms(total_duration(&items))
                )
            } else {
                format!("{} songs", util::format_count(count as u64))
            };
            byline.push((count_text, None));
            hero(
                app,
                ui,
                Hero {
                    image: pick_image(&playlist.images, 300),
                    liked: false,
                    kind: if made_together {
                        "Collaborative Playlist"
                    } else if playlist.public == Some(true) {
                        "Public Playlist"
                    } else {
                        "Playlist"
                    },
                    title: &playlist.name,
                    description: playlist.description.as_deref().map(util::strip_html),
                    byline,
                    round: false,
                },
            );
            let owned = playlist.owned_by(&user_id);
            let needle = page.filter.trim().to_lowercase();
            let sort = app
                .table_sorts
                .get(&Page::Playlist(id.to_string()))
                .copied();
            let table_view = prepare_table_view(
                ui,
                app,
                &Page::Playlist(id.to_string()),
                &items,
                &needle,
                sort,
                page.items.revision,
            );
            let view_play = table_view.view_uris.as_ref().map(Arc::clone);
            let playlist_clone = playlist.clone();
            actions_row(
                app,
                ui,
                Actions {
                    play_uri: Some(playlist.uri.clone()),
                    view: view_play,
                    // A playlist is yours or it is public; there is no
                    // following it and nothing to star (`01-api-mapping.md`).
                    saved: None,
                    saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                    saved_tooltips: ("Add to Your Library", "Remove from Your Library"),
                    owned_playlist: owned.then_some(playlist_clone),
                    reload: Some((Page::Playlist(id.to_string()), page.items.loading)),
                    name: &playlist.name,
                },
                Some(&mut page.filter),
            );
            let editable = (owned || playlist.collaborative)
                .then(|| (playlist.id.clone(), playlist.snapshot_id.clone()));
            table(
                app,
                ui,
                Table {
                    items: &items,
                    context: RowContext::Context {
                        uri: playlist.uri.clone(),
                        editable_playlist: editable,
                    },
                    show_album: true,
                    show_cover: true,
                    show_added: false,
                    show_added_by: made_together,
                    page: Page::Playlist(id.to_string()),
                    loading: page.items.loading,
                    error: page.items.error.as_deref(),
                    can_load_more: page.items.can_load_more(),
                    filter: &page.filter,
                    items_revision: page.items.revision,
                },
            );
        }
        Loadable::Loading | Loadable::NotLoaded => {
            ui.add_space(40.0);
            widgets::loading_row(ui, &palette);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            ui.add_space(40.0);
            widgets::error_row(ui, app, &error, Some(Page::Playlist(id.to_string())));
        }
    }
    app.playlist_pages.insert(id.to_string(), page);
}

pub fn album(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let Some(page) = app.album_pages.remove(id) else {
        app.ensure_loaded(Page::Album(id.to_string()));
        return;
    };
    let palette = app.palette;
    match &page.album {
        Loadable::Loaded(album) => {
            album_hero(app, ui, album, &page.tracks);
            let generation = page.generation;
            let revision = page.tracks.revision;
            let names = app.user_names_revision;
            let key = Page::Album(id.to_string());
            let items = if let Some(items) = table_items_hit(app, &key, generation, revision, names)
            {
                items
            } else {
                let rows = page
                    .tracks
                    .items
                    .iter()
                    .cloned()
                    .map(|mut track| {
                        if track.album.is_none() {
                            track.album = Some(Album {
                                id: album.id.clone(),
                                name: album.name.clone(),
                                uri: album.uri.clone(),
                                images: album.images.clone(),
                                ..Album::default()
                            });
                        }
                        (PlayableItem::Track(track), None, None)
                    })
                    .collect();
                remember_table_items(app, key, generation, revision, names, rows)
            };
            let saved = app.is_saved(&album.uri).unwrap_or(false);
            let sort = app.table_sorts.get(&Page::Album(id.to_string())).copied();
            let table_view = prepare_table_view(
                ui,
                app,
                &Page::Album(id.to_string()),
                &items,
                "",
                sort,
                page.tracks.revision,
            );
            let album_view = table_view.view_uris.as_ref().map(Arc::clone);
            actions_row(
                app,
                ui,
                Actions {
                    play_uri: Some(album.uri.clone()),
                    view: album_view,
                    saved: Some((album.uri.clone(), saved)),
                    saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                    saved_tooltips: ("Save to Your Library", "Remove from Your Library"),
                    owned_playlist: None,
                    reload: None,
                    name: &album.name,
                },
                None,
            );
            table(
                app,
                ui,
                Table {
                    items: &items,
                    context: RowContext::Context {
                        uri: album.uri.clone(),
                        editable_playlist: None,
                    },
                    show_album: false,
                    show_cover: false,
                    show_added: false,
                    show_added_by: false,
                    page: Page::Album(id.to_string()),
                    loading: page.tracks.loading,
                    error: page.tracks.error.as_deref(),
                    can_load_more: page.tracks.can_load_more(),
                    filter: "",
                    items_revision: page.tracks.revision,
                },
            );
            ui.add_space(24.0);
            if let Some(date) = &album.release_date {
                theme::text(
                    ui,
                    util::format_date(date),
                    theme::regular(12.5),
                    palette.secondary,
                );
            }
            // Labels file the same line under both kinds of copyright;
            // one line wearing both marks reads better than the line twice.
            let mut credits: Vec<(String, Vec<&str>)> = Vec::new();
            for copyright in &album.copyrights {
                let core = copyright
                    .text
                    .trim_start_matches(['©', '℗'])
                    .trim_start_matches("(C)")
                    .trim_start_matches("(P)")
                    .trim()
                    .to_string();
                let mark = if copyright.kind == "P" { "℗" } else { "©" };
                match credits.iter_mut().find(|(held, _)| *held == core) {
                    Some((_, marks)) => {
                        if !marks.contains(&mark) {
                            marks.push(mark);
                        }
                    }
                    None => credits.push((core, vec![mark])),
                }
            }
            for (core, marks) in credits {
                theme::text(
                    ui,
                    format!("{} {core}", marks.join(" ")),
                    theme::regular(11.5),
                    palette.dim,
                );
            }
        }
        Loadable::Loading | Loadable::NotLoaded => {
            ui.add_space(40.0);
            widgets::loading_row(ui, &palette);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            ui.add_space(40.0);
            widgets::error_row(ui, app, &error, Some(Page::Album(id.to_string())));
        }
    }
    app.album_pages.insert(id.to_string(), page);
}

fn album_hero(
    app: &mut App,
    ui: &mut egui::Ui,
    album: &Album,
    tracks: &PagedList<crate::api::models::Track>,
) {
    let mut byline: Vec<(String, Option<Page>)> = album
        .artists
        .iter()
        .map(|artist| (artist.name.clone(), artist.id.clone().map(Page::Artist)))
        .collect();
    if let Some(year) = album.year() {
        byline.push((year.to_string(), None));
    }
    let count = album.total_tracks.unwrap_or(tracks.items.len() as u32);
    let duration: u64 = tracks
        .items
        .iter()
        .map(|track| track.duration_ms as u64)
        .sum();
    let count_text = if tracks.is_complete() {
        format!("{count} songs, {}", util::format_total_ms(duration))
    } else {
        format!("{count} songs")
    };
    byline.push((count_text, None));
    hero(
        app,
        ui,
        Hero {
            image: pick_image(&album.images, 300),
            liked: false,
            kind: album.kind_label(),
            title: &album.name,
            description: None,
            byline,
            round: false,
        },
    );
}

pub fn liked(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let revision = app.library.liked.revision;
    let names = app.user_names_revision;
    let items =
        if let Some(items) = table_items_hit(app, &Page::LikedSongs, revision, revision, names) {
            items
        } else {
            let rows = app
                .library
                .liked
                .items
                .iter()
                .map(|saved| {
                    (
                        PlayableItem::Track(saved.track.clone()),
                        saved.added_at.clone(),
                        None,
                    )
                })
                .collect();
            remember_table_items(app, Page::LikedSongs, revision, revision, names, rows)
        };
    let total = app.library.liked.total.unwrap_or(items.len() as u32);
    let user = app
        .user
        .as_ref()
        .map(|user| user.name().to_string())
        .unwrap_or_default();
    let count_text = if app.library.liked.is_complete() {
        format!(
            "{} songs, {}",
            util::format_count(total as u64),
            util::format_total_ms(total_duration(&items))
        )
    } else {
        format!("{} songs", util::format_count(total as u64))
    };
    hero(
        app,
        ui,
        Hero {
            image: None,
            liked: true,
            kind: "Playlist",
            title: "Liked Songs",
            description: None,
            byline: vec![(user, None), (count_text, None)],
            round: false,
        },
    );
    let collection_uri = Some(crate::api::subsonic::convert::COLLECTION_URI.to_string());
    let filter_id = egui::Id::new("liked-filter");
    let mut filter = ui
        .data(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    let needle = filter.trim().to_lowercase();
    let sort = app.table_sorts.get(&Page::LikedSongs).copied();
    let table_view = prepare_table_view(
        ui,
        app,
        &Page::LikedSongs,
        &items,
        &needle,
        sort,
        app.library.liked.revision,
    );
    let liked_view = table_view.view_uris.as_ref().map(Arc::clone);
    actions_row(
        app,
        ui,
        Actions {
            play_uri: collection_uri.clone(),
            view: liked_view,
            saved: None,
            saved_icons: (Icon::Heart, Icon::HeartFilled),
            saved_tooltips: ("", ""),
            owned_playlist: None,
            reload: None,
            name: "Liked Songs",
        },
        Some(&mut filter),
    );
    ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
    let uris: Arc<[String]> = items
        .iter()
        .map(|(item, _, _)| item.uri().to_string())
        .collect::<Vec<_>>()
        .into();
    let context = match collection_uri {
        Some(uri) if app.library.liked.is_complete() => RowContext::Context {
            uri,
            editable_playlist: None,
        },
        _ => RowContext::Uris(uris),
    };
    let loading = app.library.liked.loading;
    let error = app.library.liked.error.clone();
    let can_load_more = app.library.liked.can_load_more();
    let _ = &palette;
    table(
        app,
        ui,
        Table {
            items: &items,
            context,
            show_album: true,
            show_cover: true,
            // Unlike a playlist entry, a starred song knows when it was
            // starred, so this column has something in it.
            show_added: true,
            show_added_by: false,
            page: Page::LikedSongs,
            loading,
            error: error.as_deref(),
            can_load_more,
            filter: &filter,
            items_revision: app.library.liked.revision,
        },
    );
}

#[allow(dead_code)]
fn playlist_dialog(app: &mut App, playlist: &Playlist) {
    app.actions.push(Action::ShowDialog(Dialog::EditPlaylist {
        id: playlist.id.clone(),
        name: playlist.name.clone(),
        description: playlist.description.clone().unwrap_or_default(),
        public: playlist.public.unwrap_or(false),
    }));
}

#[allow(dead_code)]
fn rect_after(ui: &egui::Ui, height: f32) -> Rect {
    let cursor = ui.cursor();
    Rect::from_min_size(
        pos2(cursor.left(), cursor.top()),
        vec2(ui.available_width(), height),
    )
}

#[allow(dead_code)]
fn palette_of(app: &App) -> Palette {
    app.palette
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_alone_is_a_view_that_plays_only_its_matches() {
        let items = make_test_tracks();
        let uri = |index: usize| items[index].0.uri().to_string();
        assert_eq!(view_uris(&items, &[0, 1, 2, 3], "", None), None);
        let filtered = view_indices(&items, "soda", None);
        assert_eq!(
            view_uris(&items, &filtered, "soda", None).as_deref(),
            Some(&[uri(1)][..])
        );
        let sorted = Some(TableSort {
            column: SortColumn::Title,
            ascending: false,
        });
        let order = view_indices(&items, "", sorted);
        assert_eq!(
            view_uris(&items, &order, "", sorted).as_deref(),
            Some(&order.iter().map(|&index| uri(index)).collect::<Vec<_>>()[..])
        );
        assert_eq!(
            view_uris(&items, &[], "nothing matches", None).as_deref(),
            Some(&[][..]),
            "an empty view is still a view, so Play does not fall back"
        );
    }

    struct KeyboardTable {
        ctx: egui::Context,
        app: App,
        items: Vec<TableItem>,
        filter: String,
        height: f32,
        editable: bool,
        items_revision: u64,
    }

    impl KeyboardTable {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            Self {
                ctx,
                app: test_app(),
                items: make_test_tracks(),
                filter: String::new(),
                height: 600.0,
                editable: false,
                items_revision: 0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(1000.0, self.height),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    widgets::search_field(
                        ui,
                        &self.app.palette,
                        egui::Id::new("keyboard-filter"),
                        &mut self.filter,
                        "Filter",
                        220.0,
                    );
                    egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                        table(
                            &mut self.app,
                            ui,
                            Table {
                                items: &self.items,
                                context: RowContext::Context {
                                    uri: "sonic:playlist:test".into(),
                                    editable_playlist: self
                                        .editable
                                        .then(|| ("test".to_string(), None)),
                                },
                                show_album: true,
                                show_cover: true,
                                show_added: false,
                                show_added_by: false,
                                page: Page::Playlist("test".into()),
                                loading: false,
                                error: None,
                                can_load_more: false,
                                filter: &self.filter,
                                items_revision: self.items_revision,
                            },
                        );
                    });
                },
            );
            output.textures_delta.clear();
            output.platform_output.accesskit_update.unwrap()
        }

        fn focus_song(&mut self, name: &str) -> egui::accesskit::NodeId {
            let tree = self.frame(vec![]);
            // The topmost, when a song is in the list more than once.
            let id = tree
                .nodes
                .iter()
                .filter(|(_, node)| {
                    node.label()
                        .is_some_and(|label| label.starts_with(&format!("Play {name},")))
                })
                .min_by(|(_, a), (_, b)| {
                    let top = |node: &egui::accesskit::Node| node.bounds().map_or(0.0, |b| b.y0);
                    top(a).total_cmp(&top(b))
                })
                .expect("song row")
                .0;
            self.frame(vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Focus,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: id,
                    data: None,
                },
            )]);
            id
        }

        fn key(&mut self, key: egui::Key) -> egui::accesskit::TreeUpdate {
            self.modified_key(key, egui::Modifiers::NONE)
        }

        fn modified_key(
            &mut self,
            key: egui::Key,
            modifiers: egui::Modifiers,
        ) -> egui::accesskit::TreeUpdate {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }])
        }

        fn focused_label(&mut self) -> String {
            let tree = self.frame(vec![]);
            tree.nodes
                .iter()
                .find(|(id, _)| *id == tree.focus)
                .unwrap()
                .1
                .label()
                .unwrap()
                .to_string()
        }
    }

    #[test]
    fn keyboard_arrows_follow_song_rows_and_enter_plays_the_focused_song() {
        let mut table = KeyboardTable::new();
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowUp);
        assert!(table.focused_label().starts_with("Play Bohemian Rhapsody,"));
        // Consecutive key frames cover repeat without relying on an idle pass
        // to establish a focus lock on each new row.
        table.key(egui::Key::ArrowDown);
        table.key(egui::Key::ArrowDown);
        assert!(table.focused_label().starts_with("Play Despacito,"));
        table.key(egui::Key::ArrowDown);
        table.key(egui::Key::ArrowDown);
        assert!(table.focused_label().starts_with("Play Ubermensch,"));
        table.key(egui::Key::ArrowUp);
        table.app.actions.clear();
        table.key(egui::Key::Enter);
        assert!(
            matches!(table.app.actions.as_slice(), [Action::PlayFromRow { uri, index: 2, .. }] if uri == "sonic:track:t_2")
        );
    }

    #[test]
    fn keyboard_arrows_follow_the_filtered_sorted_view() {
        let mut table = KeyboardTable::new();
        for index in [1, 3] {
            let PlayableItem::Track(track) = &mut table.items[index].0;
            track.artists[0].name = "Shared artist".into();
        }
        table.filter = "Shared artist".into();
        table.app.table_sorts.insert(
            Page::Playlist("test".into()),
            TableSort {
                column: SortColumn::Title,
                ascending: false,
            },
        );
        table.focus_song("Ubermensch");
        table.key(egui::Key::ArrowDown);
        assert!(table.focused_label().starts_with("Play Cancion Animal,"));
        table.app.actions.clear();
        table.key(egui::Key::Enter);
        assert!(
            matches!(table.app.actions.as_slice(), [Action::PlayFromRow { context: RowContext::View { uris, .. }, uri, index: 1 }] if uri == "sonic:track:t_1" && uris.as_ref() == ["sonic:track:t_3", "sonic:track:t_1"])
        );
        table.filter = "Queen".into();
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowDown);
        assert!(table.focused_label().starts_with("Play Bohemian Rhapsody,"));
    }

    #[test]
    fn keyboard_arrows_work_after_clicking_a_song_body() {
        let mut table = KeyboardTable::new();
        let tree = table.frame(vec![]);
        let bounds = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.label()
                    .is_some_and(|label| label.starts_with("Play Bohemian Rhapsody,"))
            })
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let pos = pos2(bounds.x0 as f32 + 180.0, bounds.y0 as f32 + 8.0);
        for pressed in [true, false] {
            table.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(table.app.actions.is_empty(), "a body click only selects");
        let page = Page::Playlist("test".into());
        assert_eq!(
            table.app.picked_rows(&page),
            Some(&[0].into_iter().collect())
        );
        table.key(egui::Key::ArrowDown);
        assert!(table.focused_label().starts_with("Play Cancion Animal,"));
        assert_eq!(
            table.app.picked_rows(&page),
            Some(&[1].into_iter().collect()),
            "the arrows select only the song they land on"
        );
        table.key(egui::Key::Enter);
        assert!(matches!(
            table.app.actions.as_slice(),
            [Action::PlayFromRow { index: 1, .. }]
        ));
    }

    /// Select all, copy and paste on a song list, and the guards that keep
    /// them out of a text field and a playlist the account cannot edit.
    #[test]
    fn select_all_copy_and_paste_work_on_a_song_list() {
        let mut table = KeyboardTable::new();
        let page = Page::Playlist("test".into());
        table.focus_song("Bohemian Rhapsody");
        table.frame(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        assert_eq!(
            table.app.picked_rows(&page),
            Some(&(0..4).collect()),
            "select all picks every row shown"
        );
        table.frame(vec![]);
        table.app.actions.clear();
        table.frame(vec![egui::Event::Copy]);
        assert!(matches!(table.app.actions.as_slice(),
            [Action::CopySongs(uris)] if uris.len() == 4 && uris[0] == "sonic:track:t_0"));

        // Cut and paste need a playlist that takes songs.
        table.app.actions.clear();
        table.frame(vec![egui::Event::Cut]);
        assert!(table.app.actions.is_empty(), "nothing to cut from here");
        table.frame(vec![egui::Event::Paste("sonic:track:t_9".into())]);
        assert!(table.app.actions.is_empty());
        table.editable = true;
        table.frame(vec![egui::Event::Paste("sonic:track:t_9".into())]);
        assert!(matches!(table.app.actions.as_slice(),
            [Action::PasteSongs { playlist_id, text }]
                if playlist_id == "test" && text == "sonic:track:t_9"));
        table.app.actions.clear();
        table.frame(vec![egui::Event::Cut]);
        assert!(matches!(table.app.actions.as_slice(),
            [Action::CopySongs(copied), Action::RemoveFromPlaylist { playlist_id, uris }]
                if copied.len() == 4 && playlist_id == "test" && uris == copied));

        // A focused text field keeps the keys for its own text.
        table.app.actions.clear();
        table
            .ctx
            .memory_mut(|memory| memory.request_focus(egui::Id::new("keyboard-filter")));
        table.frame(vec![]);
        table.frame(vec![egui::Event::Copy, egui::Event::Paste("x".into())]);
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn shift_arrows_extend_and_shrink_selection_in_display_order() {
        let mut table = KeyboardTable::new();
        let page = Page::Playlist("test".into());
        table.app.table_sorts.insert(
            page.clone(),
            TableSort {
                column: SortColumn::Title,
                ascending: false,
            },
        );
        table.focus_song("Ubermensch");
        for (key, expected) in [
            (egui::Key::ArrowDown, vec![0, 1]),
            (egui::Key::ArrowDown, vec![0, 1, 2]),
            (egui::Key::ArrowUp, vec![0, 1]),
            (egui::Key::ArrowUp, vec![0]),
            (egui::Key::ArrowUp, vec![0]),
        ] {
            table.modified_key(key, egui::Modifiers::SHIFT);
            assert_eq!(
                table.app.picked_rows(&page),
                Some(&expected.into_iter().collect())
            );
        }
        table.key(egui::Key::ArrowDown);
        table.modified_key(egui::Key::ArrowDown, egui::Modifiers::SHIFT);
        assert_eq!(
            table.app.picked_rows(&page),
            Some(&[1, 2].into_iter().collect())
        );
    }

    #[test]
    fn delete_removes_selected_playlist_songs_and_respects_editing_guards() {
        let mut table = KeyboardTable::new();
        table.editable = true;
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowDown);
        table.key(egui::Key::Delete);
        assert!(matches!(table.app.actions.as_slice(),
            [Action::RemoveFromPlaylist { playlist_id, uris }]
                if playlist_id == "test" && uris == &["sonic:track:t_1"]));
        table.app.actions.clear();
        table.modified_key(egui::Key::ArrowDown, egui::Modifiers::SHIFT);
        table.key(egui::Key::Delete);
        assert!(matches!(table.app.actions.as_slice(),
            [Action::RemoveFromPlaylist { playlist_id, uris }]
                if playlist_id == "test" && uris == &["sonic:track:t_1", "sonic:track:t_2"]));
        table.app.actions.clear();
        table.editable = false;
        table.key(egui::Key::Delete);
        assert!(table.app.actions.is_empty());
        table.editable = true;
        table.app.dialog = Some(Dialog::Shortcuts);
        table.key(egui::Key::Delete);
        assert!(table.app.actions.is_empty());
        table.app.dialog = None;
        table
            .ctx
            .memory_mut(|memory| memory.request_focus(egui::Id::new("keyboard-filter")));
        table.frame(vec![]);
        table.key(egui::Key::Delete);
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn backspace_removes_playlist_songs_only_on_macos_and_respects_editing_guards() {
        let mut table = KeyboardTable::new();
        table.editable = true;
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowDown);
        table.modified_key(egui::Key::ArrowDown, egui::Modifiers::SHIFT);
        table.key(egui::Key::Backspace);
        if cfg!(target_os = "macos") {
            assert!(matches!(table.app.actions.as_slice(),
                [Action::RemoveFromPlaylist { playlist_id, uris }]
                    if playlist_id == "test" && uris == &["sonic:track:t_1", "sonic:track:t_2"]));
        } else {
            assert!(table.app.actions.is_empty());
        }
        table.app.actions.clear();
        table.editable = false;
        table.key(egui::Key::Backspace);
        assert!(table.app.actions.is_empty());
        table.editable = true;
        table.app.dialog = Some(Dialog::Shortcuts);
        table.key(egui::Key::Backspace);
        assert!(table.app.actions.is_empty());
        table.app.dialog = None;
        table
            .ctx
            .memory_mut(|memory| memory.request_focus(egui::Id::new("keyboard-filter")));
        table.frame(vec![]);
        table.key(egui::Key::Backspace);
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn delete_does_not_remove_replacement_rows_after_a_refresh() {
        let mut table = KeyboardTable::new();
        table.editable = true;
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowDown);
        assert!(
            table
                .app
                .picked_rows(&Page::Playlist("test".into()))
                .is_some()
        );
        table.items.swap(1, 2);
        table.items_revision += 1;
        table.key(egui::Key::Delete);
        assert!(
            table
                .app
                .picked_rows(&Page::Playlist("test".into()))
                .is_none()
        );
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn delete_leaves_the_playlist_alone_while_a_row_menu_is_open() {
        let mut table = KeyboardTable::new();
        table.editable = true;
        table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::ArrowDown);
        let tree = table.frame(vec![]);
        let bounds = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == tree.focus)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let pos = pos2(bounds.x0 as f32 + 180.0, bounds.y0 as f32 + 8.0);
        for pressed in [true, false] {
            table.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(egui::Popup::is_any_open(&table.ctx));
        table.key(egui::Key::Backspace);
        assert!(table.app.actions.is_empty());
        table.key(egui::Key::Delete);
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn keyboard_tab_reaches_row_controls_and_arrows_leave_the_filter_alone() {
        let mut table = KeyboardTable::new();
        let row = table.focus_song("Bohemian Rhapsody");
        table.key(egui::Key::Tab);
        let tree = table.frame(vec![]);
        assert_ne!(tree.focus, row);
        assert_eq!(table.focused_label(), "Queen");
        table
            .ctx
            .memory_mut(|memory| memory.request_focus(egui::Id::new("keyboard-filter")));
        table.frame(vec![]);
        table.key(egui::Key::ArrowDown);
        assert!(
            table
                .ctx
                .memory(|memory| memory.has_focus(egui::Id::new("keyboard-filter")))
        );
        assert!(table.app.actions.is_empty());
    }

    #[test]
    fn keyboard_arrows_scroll_through_virtual_rows_including_duplicate_songs() {
        let mut table = KeyboardTable::new();
        table.height = 240.0;
        table.items = vec![table.items[0].clone(); 40];
        let first = table.focus_song("Bohemian Rhapsody");
        for _ in 0..25 {
            table.key(egui::Key::ArrowDown);
        }
        let tree = table.frame(vec![]);
        assert_ne!(
            tree.focus, first,
            "duplicate songs must have distinct row focus"
        );
        let node = &tree
            .nodes
            .iter()
            .find(|(id, _)| *id == tree.focus)
            .unwrap()
            .1;
        let bounds = node.bounds().unwrap();
        assert!(
            bounds.y0 >= 0.0 && bounds.y1 <= f64::from(table.height),
            "focused row must scroll into view: {bounds:?}"
        );
        table.app.actions.clear();
        table.key(egui::Key::Enter);
        assert!(
            matches!(
                table.app.actions.as_slice(),
                [Action::PlayFromRow { index: 25, .. }]
            ),
            "{:?}",
            table.app.actions
        );
    }

    #[test]
    fn sorted_view_context_keeps_playlist_remove_rights() {
        let uris: Arc<[String]> = Arc::from(["sonic:track:a".to_string()]);
        let editable = Some(("pl1".to_string(), None));

        let base = RowContext::Context {
            uri: "sonic:playlist:pl1".into(),
            editable_playlist: editable.clone(),
        };
        assert_eq!(
            view_context(&base, Some(&uris)),
            RowContext::View {
                uris: Arc::clone(&uris),
                context_uri: "sonic:playlist:pl1".into(),
                editable_playlist: editable.clone(),
            }
        );

        let readonly = RowContext::Context {
            uri: "sonic:playlist:pl1".into(),
            editable_playlist: None,
        };
        assert_eq!(
            view_context(&readonly, Some(&uris)),
            RowContext::View {
                uris: Arc::clone(&uris),
                context_uri: "sonic:playlist:pl1".into(),
                editable_playlist: None,
            }
        );

        let loose = RowContext::Uris(Arc::from(["sonic:track:b".to_string()]));
        assert_eq!(
            view_context(&loose, Some(&uris)),
            RowContext::Uris(Arc::clone(&uris))
        );

        assert_eq!(view_context(&base, None), base);
    }

    /// Refresh lives in a playlist's More menu, and is disabled while the
    /// playlist is loading.
    #[test]
    fn playlist_refresh_lives_in_the_more_menu() {
        use egui::accesskit::{Action as AccessibleAction, ActionRequest, Role, TreeId};
        for loading in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            theme::install(&ctx);
            let mut app = test_app();
            let mut frame = |events| {
                app.actions.clear();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(800.0, 520.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        actions_row(
                            &mut app,
                            ui,
                            Actions {
                                play_uri: Some("sonic:playlist:test".into()),
                                view: None,
                                saved: None,
                                saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                                saved_tooltips: ("", ""),
                                owned_playlist: None,
                                reload: Some((Page::Playlist("test".into()), loading)),
                                name: "Test",
                            },
                            None,
                        )
                    },
                );
                output.textures_delta.clear();
                (
                    output.platform_output.accesskit_update.unwrap(),
                    std::mem::take(&mut app.actions),
                )
            };
            let click = |node| {
                egui::Event::AccessKitActionRequest(ActionRequest {
                    target_tree: TreeId::ROOT,
                    target_node: node,
                    action: AccessibleAction::Click,
                    data: None,
                })
            };
            let find = |tree: &egui::accesskit::TreeUpdate, label: &str| {
                tree.nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label) && node.role() == Role::Button)
                    .map(|(id, node)| (*id, node.is_disabled()))
            };
            frame(vec![]);
            let (tree, _) = frame(vec![]);
            let (more, _) = find(&tree, "More").expect("the More button");
            frame(vec![click(more)]);
            let (tree, _) = frame(vec![]);
            let label = if loading { "Refreshing…" } else { "Refresh" };
            let (refresh, disabled) = find(&tree, label).expect("Refresh in the More menu");
            assert_eq!(disabled, loading);
            let (_, actions) = frame(vec![click(refresh)]);
            assert_eq!(
                matches!(actions.as_slice(), [Action::Reload(Page::Playlist(id))] if id == "test"),
                !loading,
                "{actions:?}"
            );
            app.backend.shutdown();
        }
    }

    #[test]
    fn collection_shuffle_button_changes_mode_without_starting_playback() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut draw = |events| {
            let mut output = ctx.run_ui(input(events), |ui| {
                actions_row(
                    &mut app,
                    ui,
                    Actions {
                        play_uri: Some("sonic:playlist:test".into()),
                        view: None,
                        saved: None,
                        saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                        saved_tooltips: ("", ""),
                        owned_playlist: None,
                        reload: None,
                        name: "Test",
                    },
                    None,
                );
            });
            output.textures_delta.clear();
        };

        draw(vec![]);
        // The Shuffle button, right of the 56-point Play button.
        let pos = egui::pos2(87.0, 28.0);
        draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);

        assert!(
            matches!(app.actions.as_slice(), [Action::SetShuffle(true)]),
            "shuffle must not start playback: {:?}",
            app.actions
        );
        app.backend.shutdown();
    }

    #[test]
    fn shuffle_uses_a_filtered_view_but_not_a_merely_sorted_one() {
        assert!(should_play_view(false, false));
        assert!(should_play_view(false, true));
        assert!(!should_play_view(true, false));
        assert!(should_play_view(true, true));
    }
    use crate::api::models::{Album, ArtistRef, Image, Track};
    use crate::model::PlaylistPage;

    fn make_large_tracks(count: usize) -> Vec<TableItem> {
        (0..count)
            .map(|i| {
                let track = Track {
                    id: Some(format!("t_{i}")),
                    name: format!("Nested metadata song {i} with a longer title"),
                    uri: format!("spotify:track:large-{i}"),
                    duration_ms: 180_000,
                    artists: vec![ArtistRef {
                        id: Some(format!("artist-{i}")),
                        name: format!("Nested Artist Name {i}"),
                        uri: Some(format!("spotify:artist:artist-{i}")),
                    }],
                    album: Some(Album {
                        id: format!("alb-{i}"),
                        name: format!("Nested Album Title {i}"),
                        uri: format!("spotify:album:alb-{i}"),
                        images: vec![
                            Image {
                                url: format!("https://i.scdn.co/image/large-{i}-640"),
                                width: Some(640),
                                height: Some(640),
                            },
                            Image {
                                url: format!("https://i.scdn.co/image/large-{i}-300"),
                                width: Some(300),
                                height: Some(300),
                            },
                        ],
                        ..Album::default()
                    }),
                    ..Track::default()
                };
                (PlayableItem::Track(track), None, None)
            })
            .collect()
    }

    fn names_only_bytes(items: &[TableItem]) -> usize {
        items
            .iter()
            .map(|(item, ..)| item.uri().len() + item.name().len())
            .sum()
    }

    fn make_test_tracks() -> Vec<TableItem> {
        let titles = [
            "Bohemian Rhapsody",
            "Cancion Animal",
            "Despacito",
            "Ubermensch",
        ];
        let artists = ["Queen", "Soda Stereo", "Luis Fonsi", "Rammstein"];
        let albums = [
            "A Night at the Opera",
            "Cancion Animal Remastered",
            "Vida",
            "Mutter",
        ];

        (0..4)
            .map(|i| {
                let track = Track {
                    id: Some(format!("t_{i}")),
                    name: titles[i].to_string(),
                    uri: format!("sonic:track:t_{i}"),
                    duration_ms: (i as u32 + 1) * 60_000,
                    track_number: Some(i as u32 + 1),
                    disc_number: Some(1),
                    explicit: false,
                    is_local: false,
                    is_playable: Some(true),
                    artists: vec![
                        ArtistRef {
                            id: Some(format!("a_{i}")),
                            name: artists[i].to_string(),
                            uri: Some(format!("sonic:artist:a_{i}")),
                        },
                        ArtistRef {
                            id: Some(format!("feat_{i}")),
                            name: format!("Feat Artist {i}"),
                            uri: Some(format!("sonic:artist:feat_{i}")),
                        },
                    ],
                    album: Some(Album {
                        id: format!("alb_{i}"),
                        name: albums[i].to_string(),
                        uri: format!("sonic:album:alb_{i}"),
                        images: vec![],
                        release_date: Some("2020-01-01".to_string()),
                        album_type: Some("album".to_string()),
                        artists: vec![],
                        album_group: None,
                        total_tracks: Some(10),
                        label: None,
                        genres: vec![],
                        popularity: None,
                        tracks: None,
                        copyrights: vec![],
                        starred: None,
                    }),
                    popularity: None,
                    starred: None,
                };
                (
                    PlayableItem::Track(track),
                    Some(format!("2024-01-0{i}")),
                    Some(format!("User {i}")),
                )
            })
            .collect()
    }

    #[test]
    fn test_view_indices_filtering_and_sorting() {
        let items = make_test_tracks();

        // 1. Unfiltered and unsorted: natural order
        let visible = view_indices(&items, "", None);
        assert_eq!(visible, vec![0, 1, 2, 3]);

        // 2. Filter by track name
        let visible = view_indices(&items, "bohemian", None);
        assert_eq!(visible, vec![0]);

        // 3. Filter by artist name
        let visible = view_indices(&items, "soda", None);
        assert_eq!(visible, vec![1]);

        // 4. Filter by album name
        let visible = view_indices(&items, "mutter", None);
        assert_eq!(visible, vec![3]);

        // 5. Sort descending by title
        let sort = Some(TableSort {
            column: SortColumn::Title,
            ascending: false,
        });
        let visible = view_indices(&items, "", sort);
        assert_eq!(visible, vec![3, 2, 1, 0]);
    }

    #[test]
    fn text_sort_keeps_case_insensitive_ties_in_source_order() {
        let mut items = make_test_tracks();
        for (item, label) in items.iter_mut().zip(["Beta", "alpha", "ALPHA", "zeta"]) {
            let PlayableItem::Track(track) = &mut item.0;
            track.name = label.into();
            track.album.as_mut().unwrap().name = label.into();
            item.2 = Some(label.into());
        }
        for column in [SortColumn::Title, SortColumn::Album, SortColumn::AddedBy] {
            assert_eq!(
                view_indices(
                    &items,
                    "",
                    Some(TableSort {
                        column,
                        ascending: true
                    })
                ),
                vec![1, 2, 0, 3]
            );
            assert_eq!(
                view_indices(
                    &items,
                    "",
                    Some(TableSort {
                        column,
                        ascending: false
                    })
                ),
                vec![3, 0, 1, 2]
            );
        }
    }

    #[test]
    fn text_sort_normalizes_each_visible_row_once() {
        let labels = ["Beta", "alpha", "ALPHA", "zeta"];
        let mut visible = [0, 1, 2, 3];
        let calls = std::cell::Cell::new(0);
        sort_by_text_key(&mut visible, false, |index| {
            calls.set(calls.get() + 1);
            labels[index].to_lowercase()
        });
        assert_eq!(calls.get(), visible.len());
        assert_eq!(visible, [3, 0, 1, 2]);
    }

    #[test]
    fn test_table_cache_validation() {
        let sort = Some(TableSort {
            column: SortColumn::Title,
            ascending: true,
        });
        let cache = TableCache {
            sort,
            needle: "desp".to_string(),
            items_revision: 5,
            user_names_revision: 2,
            visible: Arc::new([2]),
            view_uris: Some(Arc::new(["sonic:track:t_2".to_string()])),
        };

        // Cache hit
        assert!(
            cache.sort == sort
                && cache.needle == "desp"
                && cache.items_revision == 5
                && cache.user_names_revision == 2
        );

        // Cache miss on sort change
        let diff_sort = Some(TableSort {
            column: SortColumn::Title,
            ascending: false,
        });
        assert_ne!(cache.sort, diff_sort);

        // Cache miss on filter change
        assert_ne!(cache.needle, "bohemian");

        // Cache miss on items_revision change
        assert_ne!(cache.items_revision, 6);

        // Cache miss on user_names_revision change
        assert_ne!(cache.user_names_revision, 3);
    }

    fn test_app() -> App {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-table-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        App::new(
            &crate::backend::Waker::default(),
            crate::paths::AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            crate::settings::Settings::default(),
            crate::app::AppOptions {
                media_controls: false,
                tray: false,
            },
        )
    }

    #[test]
    fn table_row_cache_hits_until_revision_or_generation_changes() {
        let mut app = test_app();
        let mut builds = 0;
        let page = Page::LikedSongs;
        cached_table_items(&mut app, page.clone(), 1, 0, 0, || {
            builds += 1;
            make_test_tracks()
        });
        cached_table_items(&mut app, page.clone(), 1, 0, 0, || {
            builds += 1;
            panic!("cache hit rebuilt the table");
        });
        assert_eq!(builds, 1);
        cached_table_items(&mut app, page.clone(), 1, 1, 0, || {
            builds += 1;
            make_test_tracks()
        });
        assert_eq!(builds, 2, "revision change must rebuild");
        cached_table_items(&mut app, page, 2, 1, 0, || {
            builds += 1;
            make_test_tracks()
        });
        assert_eq!(builds, 3, "generation change must rebuild");
    }

    #[test]
    fn table_row_cache_memory_counts_nested_metadata_on_a_large_collection() {
        let mut app = test_app();
        let items = make_large_tracks(500);
        let names_only = names_only_bytes(&items);
        assert_eq!(app.table_rows_retained_bytes(), 0);
        cached_table_items(&mut app, Page::LikedSongs, 0, 0, 0, || items);
        let after = app.table_rows_retained_bytes();
        assert!(
            after > names_only,
            "retained bytes must include nested album, artist, and image strings, not just titles: names_only={names_only} after={after}"
        );
        assert!(
            after > 80_000,
            "500 tracks with nested metadata should retain a substantial copy: {after}"
        );
    }

    #[test]
    fn table_row_cache_drops_when_the_backing_page_is_evicted() {
        let mut app = test_app();
        let page = Page::Playlist("pl-gone".into());
        app.playlist_pages
            .insert("pl-gone".into(), PlaylistPage::default());
        cached_table_items(&mut app, page.clone(), 1, 0, 0, make_test_tracks);
        assert!(app.table_rows.contains_key(&page));
        app.playlist_pages.remove("pl-gone");
        app.open(Page::LikedSongs);
        assert!(
            !app.table_rows.contains_key(&page),
            "evicting the page map must drop the table-row copy"
        );
    }

    #[test]
    fn table_row_cache_keeps_at_most_two_pages() {
        let mut app = test_app();
        for i in 0..5 {
            let page = Page::Playlist(format!("pl{i}"));
            app.playlist_pages
                .insert(format!("pl{i}"), PlaylistPage::default());
            app.history.push(page.clone());
            app.history_index = app.history.len() - 1;
            cached_table_items(&mut app, page, 1, 0, 0, make_test_tracks);
        }
        assert_eq!(app.table_rows.len(), 2);
        assert!(
            app.table_rows.contains_key(&Page::Playlist("pl4".into())),
            "the open page stays"
        );
    }
}
