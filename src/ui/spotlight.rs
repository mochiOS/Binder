use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::platform::{AppInfo, DesktopPlatform, SystemAction};
use viewkit::{
    draw_command::ImageSampling,
    event::{EventContext, EventResult, ViewEvent},
    platform::{CursorIcon, Key, PointerButton},
    prelude::*,
    view::{Constraints, MeasureContext, PaintContext},
};

const PANEL_WIDTH: f32 = 620.0;
const SEARCH_HEIGHT: f32 = 56.0;
const ROW_HEIGHT: f32 = 58.0;
const PANEL_PADDING: f32 = 12.0;
const MAX_RESULTS: usize = 7;
const MAX_INDEXED_FILES: usize = 2048;
const MAX_FILE_DEPTH: usize = 4;
const SETTINGS_BUNDLE_ID: &str = "org.mochios.settings";

#[derive(Default)]
pub(crate) struct SpotlightState {
    open: bool,
    query: String,
    selected: usize,
    ignore_next_space: bool,
    files_indexed: bool,
    files: Vec<PathBuf>,
}

#[derive(Clone)]
enum SearchAction {
    Application(AppInfo),
    Settings,
    File(PathBuf),
    System(SystemAction),
    Calculation(String),
}

#[derive(Clone)]
struct SearchResult {
    title: String,
    subtitle: String,
    symbol: SymbolName,
    action: SearchAction,
}

#[derive(Clone)]
enum CachedIcon {
    Image(ImageData),
    Missing,
}

pub(crate) struct SpotlightLayer<C> {
    content: C,
    state: Rc<RefCell<SpotlightState>>,
    platform: Rc<RefCell<dyn DesktopPlatform>>,
    apps: State<Vec<AppInfo>>,
    pending_activation: Rc<RefCell<super::app_library::PendingAppActivation>>,
    menu_open: State<bool>,
    control_center_open: State<bool>,
    notification_center_open: State<bool>,
    app_library_open: State<bool>,
    hovered: Cell<Option<usize>>,
    pressed: Cell<Option<usize>>,
    icon_cache: RefCell<HashMap<PathBuf, CachedIcon>>,
}

impl<C: View> SpotlightLayer<C> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        content: C,
        state: Rc<RefCell<SpotlightState>>,
        platform: Rc<RefCell<dyn DesktopPlatform>>,
        apps: State<Vec<AppInfo>>,
        pending_activation: Rc<RefCell<super::app_library::PendingAppActivation>>,
        menu_open: State<bool>,
        control_center_open: State<bool>,
        notification_center_open: State<bool>,
        app_library_open: State<bool>,
    ) -> Self {
        Self {
            content,
            state,
            platform,
            apps,
            pending_activation,
            menu_open,
            control_center_open,
            notification_center_open,
            app_library_open,
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            icon_cache: RefCell::new(HashMap::new()),
        }
    }

    fn open(&self) {
        self.menu_open.set(false);
        self.control_center_open.set(false);
        self.notification_center_open.set(false);
        self.app_library_open.set(false);
        let mut state = self.state.borrow_mut();
        state.open = true;
        state.query.clear();
        state.selected = 0;
        state.ignore_next_space = true;
        if !state.files_indexed {
            state.files = index_home_files();
            state.files_indexed = true;
        }
        self.hovered.set(None);
        self.pressed.set(None);
    }

    fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.open = false;
        state.query.clear();
        state.selected = 0;
        state.ignore_next_space = false;
        self.hovered.set(None);
        self.pressed.set(None);
    }

    fn results(&self) -> Vec<SearchResult> {
        let state = self.state.borrow();
        build_results(&state.query, &self.apps.get(), &state.files)
    }

    fn panel(bounds: Rect, result_count: usize) -> Rect {
        let width = PANEL_WIDTH.min((bounds.size.width - 48.0).max(320.0));
        let height = SEARCH_HEIGHT + PANEL_PADDING * 2.0 + result_count.max(1) as f32 * ROW_HEIGHT;
        Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height * 0.16).max(72.0),
            width,
            height,
        )
    }

    fn search_rect(panel: Rect) -> Rect {
        Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + PANEL_PADDING,
            panel.size.width - PANEL_PADDING * 2.0,
            SEARCH_HEIGHT,
        )
    }

    fn row_rect(panel: Rect, index: usize) -> Rect {
        Rect::new(
            panel.origin.x + PANEL_PADDING,
            panel.origin.y + PANEL_PADDING + SEARCH_HEIGHT + index as f32 * ROW_HEIGHT,
            panel.size.width - PANEL_PADDING * 2.0,
            ROW_HEIGHT,
        )
    }

    fn hit(&self, panel: Rect, result_count: usize, position: Point) -> Option<usize> {
        (0..result_count).find(|index| Self::row_rect(panel, *index).contains(position))
    }

    fn load_icon(&self, path: &Path) -> CachedIcon {
        if let Some(icon) = self.icon_cache.borrow().get(path) {
            return icon.clone();
        }
        let icon = if path.extension().and_then(|value| value.to_str()) == Some("svg") {
            SvgData::from_path(path)
                .ok()
                .and_then(|svg| ImageData::from_svg(&svg, 64, 64).ok())
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        } else {
            ImageData::thumbnail_from_path(path, 64, 64)
                .map_or(CachedIcon::Missing, CachedIcon::Image)
        };
        self.icon_cache
            .borrow_mut()
            .insert(path.to_path_buf(), icon.clone());
        icon
    }

    fn paint_result_icon(
        &self,
        result: &SearchResult,
        bounds: Rect,
        context: &mut PaintContext<'_>,
    ) {
        if let SearchAction::Application(app) = &result.action {
            if let Some(path) = app.icon.as_deref()
                && let CachedIcon::Image(icon) = self.load_icon(path)
            {
                Image::new(icon)
                    .content_mode(ImageContentMode::Fit)
                    .sampling(ImageSampling::Bicubic)
                    .radius(CornerRadius::Small)
                    .paint(bounds, context);
                return;
            }
            super::app_library::paint_fallback_icon(bounds, context);
            return;
        }
        Rectangle::new()
            .color(RectangleColor::Custom(Theme::current().shell.item_enabled))
            .radius(CornerRadius::Medium)
            .paint(bounds, context);
        Icon::new(result.symbol)
            .size(17.0)
            .color(Theme::current().shell.primary_text)
            .paint(bounds, context);
    }

    fn activate(&self, result: SearchResult, context: &mut EventContext<'_>) {
        match result.action {
            SearchAction::Application(app) => {
                self.pending_activation.borrow_mut().queue(app);
                self.close();
            }
            SearchAction::Settings => {
                if let Some(app) = self
                    .apps
                    .get()
                    .into_iter()
                    .find(|app| app.bundle_id == SETTINGS_BUNDLE_ID)
                {
                    self.pending_activation.borrow_mut().queue(app);
                }
                self.close();
            }
            SearchAction::File(path) => {
                open_file(&path);
                self.close();
            }
            SearchAction::System(action) => {
                if let Err(error) = self.platform.borrow().perform_system_action(action) {
                    eprintln!("Spotlight system action failed: {error:?}");
                }
                self.close();
            }
            SearchAction::Calculation(value) => {
                let mut state = self.state.borrow_mut();
                state.query = value;
                state.selected = 0;
            }
        }
        context.request_redraw();
    }
}

impl<C: View> View for SpotlightLayer<C> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content.paint(bounds, context);
        if !self.state.borrow().open {
            return;
        }
        let results = self.results();
        let panel = Self::panel(bounds, results.len());
        Rectangle::new()
            .color(RectangleColor::Custom(Theme::current().shell.scrim))
            .paint(bounds, context);
        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.panel_background,
            ))
            .radius(CornerRadius::Custom(20.0))
            .shadow(ShadowStyle::Floating)
            .paint(panel, context);

        let search = Self::search_rect(panel);
        Rectangle::new()
            .color(RectangleColor::Custom(
                Theme::current().shell.field_background,
            ))
            .radius(CornerRadius::Custom(14.0))
            .paint(search, context);
        Icon::new(SymbolName::Search)
            .size(19.0)
            .color(Theme::current().shell.secondary_text)
            .paint(
                Rect::new(
                    search.origin.x + 14.0,
                    search.origin.y,
                    28.0,
                    search.size.height,
                ),
                context,
            );
        let query = self.state.borrow().query.clone();
        Text::styled(
            if query.is_empty() {
                String::from("Search apps, files, settings, and actions")
            } else {
                query
            },
            TextRole::TitleSmall,
        )
        .color(if self.state.borrow().query.is_empty() {
            Theme::current().shell.tertiary_text
        } else {
            Theme::current().shell.primary_text
        })
        .paint(
            Rect::new(
                search.origin.x + 48.0,
                search.origin.y,
                search.size.width - 62.0,
                search.size.height,
            ),
            context,
        );

        if results.is_empty() {
            Text::styled("No results", TextRole::Label)
                .alignment(TextAlignment::Center)
                .color(Theme::current().shell.secondary_text)
                .paint(Self::row_rect(panel, 0), context);
            return;
        }
        let selected = self
            .state
            .borrow()
            .selected
            .min(results.len().saturating_sub(1));
        for (index, result) in results.iter().enumerate() {
            let row = Self::row_rect(panel, index);
            if index == selected || self.hovered.get() == Some(index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(if index == selected {
                        Theme::current().shell.item_enabled
                    } else {
                        Theme::current().shell.item_hover
                    }))
                    .radius(CornerRadius::Medium)
                    .paint(row, context);
            }
            let icon = Rect::new(row.origin.x + 8.0, row.origin.y + 9.0, 40.0, 40.0);
            self.paint_result_icon(result, icon, context);
            Text::styled(result.title.clone(), TextRole::Label)
                .weight(650)
                .color(Theme::current().shell.primary_text)
                .paint(
                    Rect::new(
                        row.origin.x + 60.0,
                        row.origin.y + 7.0,
                        row.size.width - 72.0,
                        24.0,
                    ),
                    context,
                );
            Text::styled(result.subtitle.clone(), TextRole::Caption)
                .color(Theme::current().shell.secondary_text)
                .paint(
                    Rect::new(
                        row.origin.x + 60.0,
                        row.origin.y + 30.0,
                        row.size.width - 72.0,
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
        if let ViewEvent::KeyPressed {
            key: Key::Space,
            modifiers,
        } = event
            && modifiers.alt()
        {
            if self.state.borrow().open {
                self.close();
            } else {
                self.open();
            }
            context.request_redraw();
            return EventResult::Consumed;
        }
        if !self.state.borrow().open {
            return self.content.handle_event(bounds, event, context);
        }

        let results = self.results();
        let panel = Self::panel(bounds, results.len());
        match event {
            ViewEvent::KeyPressed {
                key: Key::Escape, ..
            } => self.close(),
            ViewEvent::KeyPressed {
                key: Key::ArrowUp, ..
            }
            | ViewEvent::ArrowLeft => {
                let mut state = self.state.borrow_mut();
                state.selected = state.selected.saturating_sub(1);
            }
            ViewEvent::KeyPressed {
                key: Key::ArrowDown,
                ..
            }
            | ViewEvent::ArrowRight => {
                let mut state = self.state.borrow_mut();
                state.selected = (state.selected + 1).min(results.len().saturating_sub(1));
            }
            ViewEvent::Backspace => {
                let mut state = self.state.borrow_mut();
                state.query.pop();
                state.selected = 0;
            }
            ViewEvent::KeyPressed {
                key: Key::Enter, ..
            } => {
                let selected = self
                    .state
                    .borrow()
                    .selected
                    .min(results.len().saturating_sub(1));
                if let Some(result) = results.get(selected).cloned() {
                    self.activate(result, context);
                    return EventResult::Consumed;
                }
            }
            ViewEvent::TextInput { text } if text.contains(['\r', '\n']) => {
                let selected = self
                    .state
                    .borrow()
                    .selected
                    .min(results.len().saturating_sub(1));
                if let Some(result) = results.get(selected).cloned() {
                    self.activate(result, context);
                    return EventResult::Consumed;
                }
            }
            ViewEvent::TextInput { text } => {
                let mut state = self.state.borrow_mut();
                if state.ignore_next_space && text == " " {
                    state.ignore_next_space = false;
                    return EventResult::Consumed;
                }
                state.ignore_next_space = false;
                for character in text.chars().filter(|character| !character.is_control()) {
                    if state.query.chars().count() >= 96 {
                        break;
                    }
                    state.query.push(character);
                }
                state.selected = 0;
            }
            ViewEvent::PointerMoved { position } => {
                self.hovered.set(self.hit(panel, results.len(), *position));
                context.set_cursor(if self.hovered.get().is_some() {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                });
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                if !panel.contains(*position) {
                    self.close();
                } else {
                    self.pressed.set(self.hit(panel, results.len(), *position));
                    if let Some(index) = self.pressed.get() {
                        self.state.borrow_mut().selected = index;
                    }
                }
            }
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            } => {
                let released = self.hit(panel, results.len(), *position);
                let pressed = self.pressed.replace(None);
                if pressed.is_some()
                    && pressed == released
                    && let Some(result) = released.and_then(|index| results.get(index)).cloned()
                {
                    self.activate(result, context);
                    return EventResult::Consumed;
                }
            }
            _ => {}
        }
        context.request_redraw();
        EventResult::Consumed
    }
}

fn build_results(query: &str, apps: &[AppInfo], files: &[PathBuf]) -> Vec<SearchResult> {
    let normalized = query.trim().to_ascii_lowercase();
    let mut results = Vec::new();
    let mut matching_apps = apps
        .iter()
        .filter(|app| {
            normalized.is_empty()
                || app.name.to_ascii_lowercase().contains(&normalized)
                || app.bundle_id.to_ascii_lowercase().contains(&normalized)
        })
        .cloned()
        .collect::<Vec<_>>();
    matching_apps.sort_by_key(|app| {
        (
            !app.name.to_ascii_lowercase().starts_with(&normalized),
            app.name.clone(),
        )
    });
    results.extend(matching_apps.into_iter().map(|app| SearchResult {
        title: app.name.clone(),
        subtitle: String::from("Application"),
        symbol: SymbolName::Grid,
        action: SearchAction::Application(app),
    }));

    if !normalized.is_empty() {
        for (title, keywords) in SETTINGS_ITEMS {
            if title.to_ascii_lowercase().contains(&normalized)
                || keywords.iter().any(|keyword| keyword.contains(&normalized))
            {
                results.push(SearchResult {
                    title: (*title).to_string(),
                    subtitle: String::from("Settings"),
                    symbol: SymbolName::Settings,
                    action: SearchAction::Settings,
                });
            }
        }
        for (title, keywords, symbol, action) in SYSTEM_ACTIONS {
            if title.to_ascii_lowercase().contains(&normalized)
                || keywords.iter().any(|keyword| keyword.contains(&normalized))
            {
                results.push(SearchResult {
                    title: (*title).to_string(),
                    subtitle: String::from("System Action"),
                    symbol: *symbol,
                    action: SearchAction::System(*action),
                });
            }
        }
        if normalized.chars().count() >= 2 {
            results.extend(
                files
                    .iter()
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.to_ascii_lowercase().contains(&normalized))
                    })
                    .take(4)
                    .map(|path| SearchResult {
                        title: path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("File")
                            .to_string(),
                        subtitle: path.to_string_lossy().into_owned(),
                        symbol: SymbolName::File,
                        action: SearchAction::File(path.clone()),
                    }),
            );
        }
        if let Some(value) = calculate(&normalized) {
            results.insert(
                0,
                SearchResult {
                    title: format_number(value),
                    subtitle: format!("Calculation · {query}"),
                    symbol: SymbolName::Info,
                    action: SearchAction::Calculation(format_number(value)),
                },
            );
        }
    }
    results.truncate(MAX_RESULTS);
    results
}

const SETTINGS_ITEMS: &[(&str, &[&str])] = &[
    (
        "General Settings",
        &["device", "language", "region", "time"],
    ),
    (
        "Appearance Settings",
        &["theme", "wallpaper", "accent", "display"],
    ),
    ("Input Settings", &["keyboard", "mouse", "touchpad", "ime"]),
    (
        "Notification Settings",
        &["notification", "focus", "banner"],
    ),
    ("Security Settings", &["security", "certificate", "trust"]),
    ("Default Apps", &["default", "association", "file type"]),
    ("Applications", &["application", "permission", "capability"]),
];

const SYSTEM_ACTIONS: &[(&str, &[&str], SymbolName, SystemAction)] = &[
    (
        "Lock Screen",
        &["lock"],
        SymbolName::Lock,
        SystemAction::LockScreen,
    ),
    (
        "Sleep",
        &["sleep", "suspend"],
        SymbolName::Minus,
        SystemAction::Sleep,
    ),
    (
        "Restart",
        &["restart", "reboot"],
        SymbolName::Refresh,
        SystemAction::Restart,
    ),
    (
        "Shut Down",
        &["shutdown", "power off"],
        SymbolName::XCircle,
        SystemAction::ShutDown,
    ),
];

fn index_home_files() -> Vec<PathBuf> {
    let root = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home"));
    let mut files = Vec::new();
    let mut pending = vec![(root, 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if files.len() >= MAX_INDEXED_FILES {
                break;
            }
            let path = entry.path();
            let hidden = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if hidden {
                continue;
            }
            if path.is_dir() && depth < MAX_FILE_DEPTH {
                pending.push((path, depth + 1));
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    files
}

#[cfg(target_os = "mochios")]
fn open_file(path: &Path) {
    let content_type = match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => "application/json",
        Some("toml") => "application/toml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("md") => "text/markdown",
        _ => "text/plain",
    };
    if let Some(path) = path.to_str()
        && let Err(error) = mochi_user_platform::workspace::open_document(
            path,
            content_type,
            mochi_user_platform::workspace::ASSOCIATION_ROLE_VIEW,
        )
    {
        eprintln!("Spotlight could not open {path}: {error:?}");
    }
}

#[cfg(not(target_os = "mochios"))]
fn open_file(_path: &Path) {}

fn calculate(input: &str) -> Option<f64> {
    let mut parser = Calculator::new(input);
    let value = parser.expression()?;
    parser.skip_spaces();
    (parser.offset == parser.bytes.len() && value.is_finite()).then_some(value)
}

fn format_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        let value = format!("{value:.8}");
        value
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

struct Calculator<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Calculator<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            offset: 0,
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            self.skip_spaces();
            match self.peek() {
                Some(b'+') => {
                    self.offset += 1;
                    value += self.term()?;
                }
                Some(b'-') => {
                    self.offset += 1;
                    value -= self.term()?;
                }
                _ => return Some(value),
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        loop {
            self.skip_spaces();
            match self.peek() {
                Some(b'*') => {
                    self.offset += 1;
                    value *= self.factor()?;
                }
                Some(b'/') => {
                    self.offset += 1;
                    let divisor = self.factor()?;
                    if divisor == 0.0 {
                        return None;
                    }
                    value /= divisor;
                }
                _ => return Some(value),
            }
        }
    }

    fn factor(&mut self) -> Option<f64> {
        self.skip_spaces();
        if self.peek() == Some(b'-') {
            self.offset += 1;
            return self.factor().map(|value| -value);
        }
        if self.peek() == Some(b'(') {
            self.offset += 1;
            let value = self.expression()?;
            self.skip_spaces();
            if self.peek() != Some(b')') {
                return None;
            }
            self.offset += 1;
            return Some(value);
        }
        let start = self.offset;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            self.offset += 1;
        }
        if start == self.offset {
            return None;
        }
        core::str::from_utf8(&self.bytes[start..self.offset])
            .ok()?
            .parse()
            .ok()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_respects_precedence_and_parentheses() {
        assert_eq!(calculate("2 + 3 * 4"), Some(14.0));
        assert_eq!(calculate("(2 + 3) * 4"), Some(20.0));
        assert_eq!(calculate("-8 / 2"), Some(-4.0));
        assert_eq!(calculate("1 / 0"), None);
        assert_eq!(calculate("hello"), None);
    }

    #[test]
    fn search_limits_results() {
        let apps = (0..12)
            .map(|index| AppInfo {
                bundle_id: format!("org.mochios.app{index}"),
                name: format!("App {index}"),
                root: PathBuf::new(),
                version: String::new(),
                developer: String::new(),
                entry: String::new(),
                description: String::new(),
                icon: None,
                resources: Vec::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(build_results("app", &apps, &[]).len(), MAX_RESULTS);
    }
}
