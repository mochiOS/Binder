use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform, ProcessId};
use crate::window::DesktopWindows;
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::{Key, PointerButton},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const MAX_VISIBLE: usize = 7;
const ITEM_WIDTH: f32 = 86.0;
const ITEM_HEIGHT: f32 = 96.0;
const ICON_SIZE: f32 = 56.0;
const PANEL_PADDING: f32 = 16.0;

#[derive(Clone)]
struct Entry {
    process: ProcessId,
    app: AppInfo,
    minimized: bool,
}

#[derive(Default)]
pub(crate) struct AppSwitcherState {
    entries: Vec<Entry>,
    selected: usize,
    open: bool,
}

#[derive(Clone)]
enum CachedIcon {
    Image(ImageData),
    Missing,
}

pub(crate) struct AppSwitcherLayer<C> {
    content: C,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    windows: State<DesktopWindows>,
    apps: State<Vec<AppInfo>>,
    running_apps: State<Vec<String>>,
    state: Rc<RefCell<AppSwitcherState>>,
    hovered: Cell<Option<usize>>,
    icon_cache: RefCell<HashMap<PathBuf, CachedIcon>>,
}

impl<C: View> AppSwitcherLayer<C> {
    pub(crate) fn new(
        content: C,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        windows: State<DesktopWindows>,
        apps: State<Vec<AppInfo>>,
        running_apps: State<Vec<String>>,
        state: Rc<RefCell<AppSwitcherState>>,
    ) -> Self {
        Self {
            content,
            platform,
            windows,
            apps,
            running_apps,
            state,
            hovered: Cell::new(None),
            icon_cache: RefCell::new(HashMap::new()),
        }
    }

    fn begin_or_advance(&self, backwards: bool) {
        let mut state = self.state.borrow_mut();
        if !state.open {
            let apps = self.apps.get();
            let platform = self.platform.borrow();
            let by_bundle = apps
                .iter()
                .cloned()
                .map(|app| (app.bundle_id.clone(), app))
                .collect::<HashMap<_, _>>();
            let by_process = apps
                .into_iter()
                .filter_map(|app| {
                    platform
                        .process_id_for_bundle(&app.bundle_id)
                        .map(|process| (process, app))
                })
                .collect::<HashMap<_, _>>();
            drop(platform);
            let desktop = self.windows.get();
            let mut seen = HashSet::new();
            state.entries = desktop
                .windows
                .iter()
                .rev()
                .filter_map(|window| {
                    let process = window.process_id?;
                    if !seen.insert(process) {
                        return None;
                    }
                    Some(Entry {
                        process,
                        app: by_process.get(&process)?.clone(),
                        minimized: desktop
                            .windows
                            .iter()
                            .filter(|candidate| candidate.process_id == Some(process))
                            .all(|candidate| candidate.minimized),
                    })
                })
                .collect();
            for bundle_id in self.running_apps.get() {
                let Some(app) = by_bundle.get(&bundle_id).cloned() else {
                    continue;
                };
                let Some(process) = self.platform.borrow().process_id_for_bundle(&bundle_id) else {
                    continue;
                };
                if seen.insert(process) {
                    state.entries.push(Entry {
                        process,
                        app,
                        minimized: false,
                    });
                }
            }
            state.open = !state.entries.is_empty();
            state.selected = if state.entries.len() > 1 { 1 } else { 0 };
            if backwards && state.entries.len() > 1 {
                state.selected = state.entries.len() - 1;
            }
            return;
        }
        let count = state.entries.len();
        if count > 0 {
            state.selected = if backwards {
                (state.selected + count - 1) % count
            } else {
                (state.selected + 1) % count
            };
        }
    }

    fn finish(&self, activate: bool) {
        let entry = {
            let mut state = self.state.borrow_mut();
            let entry = activate
                .then(|| state.entries.get(state.selected).cloned())
                .flatten();
            state.open = false;
            state.entries.clear();
            entry
        };
        self.hovered.set(None);
        if let Some(entry) = entry {
            if let Err(error) = self.platform.borrow().activate_application(entry.process) {
                eprintln!("failed to activate app {}: {error:?}", entry.app.bundle_id);
            }
            self.windows.update(|desktop| {
                desktop.activate_process(entry.process);
            });
        }
    }

    fn visible_range(state: &AppSwitcherState) -> std::ops::Range<usize> {
        let count = state.entries.len().min(MAX_VISIBLE);
        let start = state
            .selected
            .saturating_sub(count / 2)
            .min(state.entries.len().saturating_sub(count));
        start..start + count
    }

    fn panel(bounds: Rect, count: usize) -> Rect {
        let width = ITEM_WIDTH * count as f32 + PANEL_PADDING * 2.0;
        Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - ITEM_HEIGHT - PANEL_PADDING * 2.0) / 2.0,
            width,
            ITEM_HEIGHT + PANEL_PADDING * 2.0,
        )
    }

    fn item(panel: Rect, local: usize) -> Rect {
        Rect::new(
            panel.origin.x + PANEL_PADDING + local as f32 * ITEM_WIDTH,
            panel.origin.y + PANEL_PADDING,
            ITEM_WIDTH,
            ITEM_HEIGHT,
        )
    }

    fn hit(&self, bounds: Rect, position: Point) -> Option<usize> {
        let state = self.state.borrow();
        let range = Self::visible_range(&state);
        let panel = Self::panel(bounds, range.len());
        range
            .enumerate()
            .find_map(|(local, index)| Self::item(panel, local).contains(position).then_some(index))
    }

    fn load_icon(&self, path: &Path) -> CachedIcon {
        if let Some(icon) = self.icon_cache.borrow().get(path) {
            return icon.clone();
        }
        let icon = if path.extension().and_then(|value| value.to_str()) == Some("svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| ImageData::from_svg(&svg, 72, 72).ok())
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        } else {
            ImageData::thumbnail_from_path(path, 72, 72)
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        };
        self.icon_cache
            .borrow_mut()
            .insert(path.to_path_buf(), icon.clone());
        icon
    }

    fn paint_icon(&self, app: &AppInfo, bounds: Rect, context: &mut PaintContext<'_>) {
        if let Some(path) = app.icon.as_deref()
            && let CachedIcon::Image(image) = self.load_icon(path)
        {
            Image::new(image)
                .content_mode(ImageContentMode::Fit)
                .sampling(ImageSampling::Bicubic)
                .radius(CornerRadius::Custom(12.0))
                .paint(bounds, context);
        } else {
            super::app_library::paint_fallback_icon(bounds, context);
        }
    }
}

impl<C: View> View for AppSwitcherLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        let state = self.state.borrow();
        if !state.open {
            return;
        }
        let range = Self::visible_range(&state);
        let panel = Self::panel(bounds, range.len());
        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.panel_background,
            ))
            .radius(CornerRadius::Custom(20.0))
            .shadow(ShadowStyle::Card)
            .border(BorderStyle::custom(
                Theme::current().shell.panel_border,
                1.0,
            ))
            .paint(panel, context);
        for (local, index) in range.enumerate() {
            let entry = &state.entries[index];
            let item = Self::item(panel, local);
            if index == state.selected {
                Rectangle::new()
                    .color(RectangleColor::Custom(
                        Theme::current().shell.selection_soft,
                    ))
                    .radius(CornerRadius::Custom(14.0))
                    .paint(item, context);
            } else if self.hovered.get() == Some(index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(Theme::current().shell.item_hover))
                    .radius(CornerRadius::Custom(14.0))
                    .paint(item, context);
            }
            let icon = Rect::new(
                item.origin.x + (item.size.width - ICON_SIZE) / 2.0,
                item.origin.y + 7.0,
                ICON_SIZE,
                ICON_SIZE,
            );
            self.paint_icon(&entry.app, icon, context);
            if entry.minimized {
                Rectangle::new()
                    .color(RectangleColor::Custom(
                        Theme::current().colors.text_secondary,
                    ))
                    .radius(CornerRadius::Custom(3.0))
                    .paint(
                        Rect::new(icon.origin.x + 47.0, icon.origin.y + 47.0, 6.0, 6.0),
                        context,
                    );
            }
            Text::styled(entry.app.name.clone(), TextRole::Caption)
                .alignment(TextAlignment::Center)
                .paint(
                    Rect::new(
                        item.origin.x + 4.0,
                        item.origin.y + 68.0,
                        item.size.width - 8.0,
                        20.0,
                    ),
                    context,
                );
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        match event {
            ViewEvent::KeyPressed {
                key: Key::Tab,
                modifiers,
            } if modifiers.alt() => {
                self.begin_or_advance(modifiers.shift());
                context.request_redraw();
                EventResult::Consumed
            }
            ViewEvent::KeyReleased { key: Key::Alt, .. } if self.state.borrow().open => {
                self.finish(true);
                context.request_redraw();
                EventResult::Consumed
            }
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } if self.state.borrow().open => {
                self.finish(false);
                context.request_redraw();
                EventResult::Consumed
            }
            ViewEvent::PointerMoved { position } if self.state.borrow().open => {
                let hovered = self.hit(bounds, *position);
                if self.hovered.replace(hovered) != hovered {
                    context.request_redraw();
                }
                EventResult::Consumed
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } if self.state.borrow().open => {
                if let Some(index) = self.hit(bounds, *position) {
                    self.state.borrow_mut().selected = index;
                    self.finish(true);
                } else {
                    self.finish(false);
                }
                context.request_redraw();
                EventResult::Consumed
            }
            _ if self.state.borrow().open => EventResult::Consumed,
            _ => self.content.handle_event(bounds, event, context),
        }
    }
}
