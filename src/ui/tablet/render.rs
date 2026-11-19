//! Logical-coordinate layout shared by painting and gesture hit testing.
use super::*;
use viewkit::draw_command::{DrawCommand, PaintTransform};

pub(super) struct Geometry {
    library_bottom: f32,
    pub bounds: Rect,
    pub canvas: Rect,
    pub application: Rect,
    pub dock: Rect,
    pub edge: Rect,
    pub keyboard: Option<keyboard::KeyboardGeometry>,
    pub search: Rect,
    pub gap: f32,
    pub pitch: f32,
    pub row_pitch: f32,
    pub icon: f32,
    pub dock_pitch: f32,
}

struct NotificationSheetLayout {
    time: Rect,
    date: Rect,
    list: Rect,
    controls: Rect,
}

impl NotificationSheetLayout {
    fn new(geometry: &Geometry, theme: &Theme) -> Self {
        let bounds = geometry.bounds;
        let margin = theme.spacing.extra_large;
        let top = (bounds.size.height * 0.08)
            .max(theme.layout.status_bar_height + margin)
            .min(theme.layout.settings_row_height * 2.0);
        let time = Rect::new(
            bounds.origin.x,
            bounds.origin.y + top,
            bounds.size.width,
            theme.typography.display_large.line_height,
        );
        let date = Rect::new(
            bounds.origin.x,
            time.origin.y + time.size.height + theme.spacing.extra_small,
            bounds.size.width,
            theme.typography.label.line_height,
        );
        let list_top = date.origin.y + date.size.height + theme.layout.section_gap * 2.0;
        let width = (bounds.size.width - margin * 2.0)
            .max(0.0)
            .min(theme.layout.content_max_width);
        let list = Rect::new(
            bounds.origin.x + (bounds.size.width - width) * 0.5,
            list_top,
            width,
            (geometry.edge.origin.y - margin - list_top).max(0.0),
        );
        let side = theme.controls.touch_height;
        let controls = Rect::new(
            bounds.origin.x + bounds.size.width - margin - side,
            bounds.origin.y + theme.layout.status_bar_height + theme.spacing.small,
            side,
            side,
        );
        Self {
            time,
            date,
            list,
            controls,
        }
    }
}

impl Geometry {
    pub fn for_environment(
        bounds: Rect,
        theme: &Theme,
        dock_count: usize,
        environment: WindowEnvironment,
    ) -> Self {
        let mut geometry = Self::new(environment.safe_bounds(bounds), theme, dock_count);
        let content = environment.content_bounds(bounds);
        geometry.library_bottom = content.origin.y + content.size.height;
        geometry.library_bottom = geometry.library_bottom.min(geometry.edge.origin.y);
        geometry
    }
    pub fn with_keyboard(mut self, keyboard: Option<keyboard::KeyboardGeometry>) -> Self {
        self.keyboard = keyboard;
        if let Some(keyboard) = keyboard {
            if keyboard.docked {
                self.application.size.height = (keyboard.keys.origin.y - self.application.origin.y)
                    .clamp(1.0, self.application.size.height);
            }
            self.library_bottom = self.library_bottom.min(keyboard.keys.origin.y);
        }
        self
    }
    pub fn new(bounds: Rect, theme: &Theme, dock_count: usize) -> Self {
        let gap = theme.spacing.large.max(12.0);
        let status = theme.typography.label.line_height + gap;
        let footer = theme.layout.control_height.max(44.0);
        let available = (bounds.size.height - status - footer - gap * 2.0).max(44.0);
        // Adapt the icon footprint to both axes. Theme controls give the lower
        // touch target; cell proportions provide the visual size on tablets.
        let row_pitch = available / model::ROWS as f32;
        let desired_columns =
            ((bounds.size.width / (theme.layout.control_height * 3.0)).floor() as u32 / 2 * 2)
                .clamp(2, 6) as f32;
        let icon = (row_pitch.min(bounds.size.width / (desired_columns + 1.0)) * 0.38).clamp(
            theme.layout.control_height,
            theme.layout.control_height * 3.0,
        );
        let dock_pitch = icon + theme.spacing.small;
        let dock_width = icon + gap;
        let dock_height =
            (dock_count.max(1) as f32 * dock_pitch + gap - theme.spacing.small).min(available);
        let dock = Rect::new(
            bounds.origin.x + bounds.size.width - dock_width - gap,
            bounds.origin.y + status + (available - dock_height) * 0.5 + gap,
            dock_width,
            dock_height,
        );
        let canvas = Rect::new(
            bounds.origin.x + gap,
            bounds.origin.y + status + gap,
            (bounds.size.width - dock_width - gap * 3.0).max(44.0),
            available,
        );
        let columns = desired_columns;
        let pitch = canvas.size.width / columns;
        let icon = icon.min(pitch * 0.72).min(row_pitch * 0.65).max(24.0);
        let edge = Rect::new(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height - footer,
            bounds.size.width,
            footer,
        );
        let search_width = (bounds.size.width - gap * 4.0).clamp(44.0, 560.0);
        let search = Rect::new(
            bounds.origin.x + (bounds.size.width - search_width) * 0.5,
            bounds.origin.y + status + gap + theme.typography.title_medium.line_height + gap,
            search_width,
            theme.controls.touch_height,
        );
        Self {
            library_bottom: edge.origin.y,
            bounds,
            canvas,
            application: Rect::new(
                bounds.origin.x,
                bounds.origin.y + status,
                bounds.size.width,
                (bounds.size.height - status).max(1.0),
            ),
            dock,
            edge,
            keyboard: None,
            search,
            gap,
            pitch,
            row_pitch,
            icon,
            dock_pitch,
        }
    }
    pub fn application_icon_scale(&self) -> f32 {
        (self.icon / self.application.size.width.max(1.0)).clamp(0.01, 1.0)
    }
    pub fn home_cell(&self, slot: Slot, scroll: f32) -> Rect {
        Rect::new(
            self.canvas.origin.x + slot.column as f32 * self.pitch - scroll,
            self.canvas.origin.y + slot.row as f32 * self.row_pitch,
            self.pitch,
            self.row_pitch,
        )
    }
    pub fn slot(&self, point: Point, scroll: f32) -> Slot {
        Slot {
            column: ((point.x - self.canvas.origin.x + scroll) / self.pitch)
                .floor()
                .clamp(0.0, 255.0) as u32,
            row: ((point.y - self.canvas.origin.y) / self.row_pitch)
                .floor()
                .clamp(0.0, (model::ROWS - 1) as f32) as u32,
        }
    }
    pub fn icon_rect(&self, cell: Rect) -> Rect {
        Rect::new(
            cell.origin.x + (cell.size.width - self.icon) * 0.5,
            cell.origin.y + (cell.size.height - self.icon - 26.0).max(0.0) * 0.5,
            self.icon,
            self.icon,
        )
    }
    pub fn card_pitch(&self) -> f32 {
        (self.bounds.size.width * 0.56).max(220.0) + self.gap
    }
    pub fn card(&self, index: usize, scroll: f32) -> Rect {
        let width = self.card_pitch() - self.gap;
        let height = (width * self.application.size.height / self.application.size.width.max(1.0)
            + 48.0)
            .min(self.bounds.size.height * 0.72)
            .max(120.0);
        Rect::new(
            self.bounds.origin.x
                + (self.bounds.size.width - width) * 0.5
                + index as f32 * self.card_pitch()
                - scroll,
            self.bounds.origin.y + (self.bounds.size.height - height) * 0.5,
            width,
            height,
        )
    }
    pub fn drawer_cell(&self, index: usize, scroll: f32) -> Rect {
        let width = (self.bounds.size.width - self.gap * 4.0).max(44.0);
        let columns = (width / (self.icon + self.gap * 4.0)).floor().max(2.0) as usize;
        let pitch = width / columns as f32;
        let row_height = self.icon + self.gap * 3.0 + 28.0;
        Rect::new(
            self.bounds.origin.x
                + (self.bounds.size.width - width) * 0.5
                + (index % columns) as f32 * pitch,
            self.search.origin.y
                + self.search.size.height
                + self.gap
                + (index / columns) as f32 * row_height
                - scroll,
            pitch,
            row_height,
        )
    }
    pub fn folder(&self) -> Rect {
        let width = (self.bounds.size.width - self.gap * 4.0)
            .min(560.0)
            .max(44.0);
        let height = (self.bounds.size.height - self.gap * 8.0)
            .min(440.0)
            .max(120.0);
        Rect::new(
            self.bounds.origin.x + (self.bounds.size.width - width) * 0.5,
            self.bounds.origin.y + (self.bounds.size.height - height) * 0.5,
            width,
            height,
        )
    }
    pub fn library_body(&self) -> Rect {
        Rect::new(
            self.bounds.origin.x + self.gap,
            self.search.origin.y + self.search.size.height + self.gap,
            self.bounds.size.width - self.gap * 2.0,
            (self.library_bottom - self.search.origin.y - self.search.size.height - self.gap)
                .max(1.0),
        )
    }
    pub fn folder_body(&self) -> Rect {
        let panel = self.folder();
        Rect::new(
            panel.origin.x + self.gap,
            panel.origin.y + 52.0,
            panel.size.width - self.gap * 2.0,
            (panel.size.height - 52.0 - self.gap).max(1.0),
        )
    }
    pub fn folder_cell(&self, index: usize, scroll: f32) -> Rect {
        let panel = self.folder();
        let columns = ((panel.size.width - self.gap * 2.0) / 100.0)
            .floor()
            .max(2.0) as usize;
        let pitch = (panel.size.width - self.gap * 2.0) / columns as f32;
        Rect::new(
            panel.origin.x + self.gap + (index % columns) as f32 * pitch,
            panel.origin.y + 52.0 + (index / columns) as f32 * 112.0 - scroll,
            pitch,
            112.0,
        )
    }
}

pub(super) enum Hit {
    Application(String),
    Folder(u64),
    Library,
    Notifications,
    Controls,
    Search,
    Done,
    Close(ProcessId),
}

impl TabletShell {
    pub(super) fn app_transform(&self, geometry: &Geometry) -> PaintTransform {
        let state = self.state.borrow();
        let progress = state.app_motion.value.clamp(0.0, 1.0);
        let center = Point::new(
            geometry.application.origin.x + geometry.application.size.width * 0.5,
            geometry.application.origin.y + geometry.application.size.height * 0.5,
        );
        PaintTransform {
            anchor: center,
            scale: geometry.application_icon_scale()
                + (1.0 - geometry.application_icon_scale()) * progress,
            offset: Point::new(state.app_x.value - center.x, state.app_y.value - center.y),
            opacity: (progress * 2.0).min(1.0),
        }
    }

    pub(super) fn launch_anchor(&self, bundle: &str, point: Point, geometry: &Geometry) -> Point {
        let state = self.state.borrow();
        let icon = match state.surface {
            Surface::Home if geometry.dock.contains(point) => {
                let index = ((point.y - geometry.dock.origin.y - geometry.gap * 0.5
                    + state.dock_scroll)
                    / geometry.dock_pitch)
                    .floor();
                let size = geometry.dock.size.width - geometry.gap;
                Rect::new(
                    geometry.dock.origin.x + geometry.gap * 0.5,
                    geometry.dock.origin.y + geometry.gap * 0.5 + index * geometry.dock_pitch
                        - state.dock_scroll,
                    size,
                    size,
                )
            }
            Surface::Home => geometry.icon_rect(
                geometry.home_cell(geometry.slot(point, state.scroll.value), state.scroll.value),
            ),
            Surface::Library => {
                let apps = self.resources.apps.get();
                let filtered =
                    super::super::app_library::matching_app_indices(&apps, &state.search.value());
                let Some(index) = filtered
                    .iter()
                    .position(|index| apps[*index].bundle_id == bundle)
                else {
                    return point;
                };
                let cell = geometry.drawer_cell(index, state.app_scroll);
                Rect::new(
                    cell.origin.x + (cell.size.width - geometry.icon) * 0.5,
                    cell.origin.y + 8.0,
                    geometry.icon,
                    geometry.icon,
                )
            }
            Surface::Folder(id) => {
                let scroll = state.folder_scroll;
                drop(state);
                let Some(index) = self
                    .folder_applications(id)
                    .iter()
                    .position(|id| id == bundle)
                else {
                    return point;
                };
                let cell = geometry.folder_cell(index, scroll);
                Rect::new(
                    cell.origin.x + (cell.size.width - geometry.icon) * 0.5,
                    cell.origin.y + 8.0,
                    geometry.icon,
                    geometry.icon,
                )
            }
            _ => return point,
        };
        Point::new(
            icon.origin.x + icon.size.width * 0.5,
            icon.origin.y + icon.size.height * 0.5,
        )
    }

    pub(super) fn home_destination(&self, geometry: &Geometry) -> Point {
        let fallback = Point::new(
            geometry.edge.origin.x + geometry.edge.size.width * 0.5,
            geometry.edge.origin.y + geometry.edge.size.height * 0.5,
        );
        let Some(process) = self.foreground().and_then(|window| window.process_id) else {
            return fallback;
        };
        let bundle = {
            let platform = self.resources.platform.borrow();
            self.resources
                .apps
                .get()
                .iter()
                .find(|app| platform.process_id_for_bundle(&app.bundle_id) == Some(process))
                .map(|app| app.bundle_id.clone())
        };
        let Some(bundle) = bundle else {
            return fallback;
        };
        let state = self.state.borrow();
        if let Some(item) = state.layout.items.iter().find(|item| match &item.kind {
            ItemKind::Application(id) => id == &bundle,
            ItemKind::Folder { applications, .. } => applications.contains(&bundle),
            _ => false,
        }) {
            let icon = geometry.icon_rect(geometry.home_cell(item.slot, state.scroll.value));
            if icon.intersection(geometry.canvas) == Some(icon) {
                return Point::new(
                    icon.origin.x + icon.size.width * 0.5,
                    icon.origin.y + icon.size.height * 0.5,
                );
            }
        }
        if let Some(index) = self
            .dock_apps()
            .iter()
            .position(|app| app.bundle_id == bundle)
        {
            return Point::new(
                geometry.dock.origin.x + geometry.dock.size.width * 0.5,
                geometry.dock.origin.y + geometry.gap * 0.5 + index as f32 * geometry.dock_pitch
                    - state.dock_scroll
                    + geometry.icon * 0.5,
            );
        }
        fallback
    }
    fn paint_icon(&self, app: &AppInfo, bounds: Rect, context: &mut PaintContext<'_>) {
        let state = self.state.borrow();
        if let Some((pressed, press)) = &state.press {
            if *pressed == bounds {
                let points = viewkit::omochi::rounded_rect_points(
                    bounds.size.width,
                    bounds.size.height,
                    bounds.size.width * 0.3,
                    8,
                )
                .into_iter()
                .map(|point| Point::new(point.x + bounds.origin.x, point.y + bounds.origin.y))
                .collect::<Vec<_>>();
                let sample = press.sample(&points, Instant::now());
                let (minimum, maximum) = sample
                    .points
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), point| {
                        (low.min(point.x), high.max(point.x))
                    });
                let scale = ((maximum - minimum) / bounds.size.width.max(1.0)).clamp(0.92, 1.03);
                context.with_presentation(
                    PaintTransform {
                        anchor: Point::new(
                            bounds.origin.x + bounds.size.width * 0.5,
                            bounds.origin.y + bounds.size.height * 0.5,
                        ),
                        scale,
                        offset: Point::default(),
                        opacity: 1.0,
                    },
                    true,
                    |context| self.icons.paint(app, bounds, context),
                );
                return;
            }
        }
        self.icons.paint(app, bounds, context);
    }
    pub(super) fn notification_bounds(&self, geometry: &Geometry) -> Rect {
        NotificationSheetLayout::new(geometry, &Theme::current()).list
    }

    pub(super) fn notification_settings_bounds(&self, geometry: &Geometry) -> Rect {
        NotificationSheetLayout::new(geometry, &Theme::current()).controls
    }

    pub(super) fn notification_settings_button(&self) -> Button {
        Button::with_interaction_and_label(self.state.borrow().notification_settings.clone(), "")
            .style(ButtonStyle::Standard)
            .size(ButtonSize::Touch)
            .radius(CornerRadius::Full)
            .accessibility_label("Quick settings")
            .content(
                Icon::new(SymbolName::ChevronDown).size(Theme::current().layout.control_icon_size),
            )
    }

    pub(super) fn home_item_at(&self, point: Point, geometry: &Geometry) -> Option<u64> {
        if !geometry.canvas.contains(point) {
            return None;
        }
        let state = self.state.borrow();
        state
            .layout
            .items
            .iter()
            .find(|item| {
                geometry
                    .home_cell(item.slot, state.scroll.value)
                    .contains(point)
            })
            .map(|item| item.id)
    }

    fn folder_applications(&self, id: u64) -> Vec<String> {
        self.state
            .borrow()
            .layout
            .items
            .iter()
            .find_map(|item| match &item.kind {
                ItemKind::Folder { applications, .. } if item.id == id => {
                    Some(applications.clone())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    pub(super) fn library_maximum(&self, geometry: &Geometry) -> f32 {
        let apps = self.resources.apps.get();
        let query = self.state.borrow().search.value();
        let count = super::super::app_library::matching_app_indices(&apps, &query).len();
        if count == 0 {
            return 0.0;
        }
        let last = geometry.drawer_cell(count - 1, 0.0);
        let body = geometry.library_body();
        (last.origin.y + last.size.height - body.origin.y - body.size.height).max(0.0)
    }

    pub(super) fn folder_maximum(&self, geometry: &Geometry) -> f32 {
        let Surface::Folder(id) = self.state.borrow().surface else {
            return 0.0;
        };
        let count = self.folder_applications(id).len();
        if count == 0 {
            return 0.0;
        }
        let last = geometry.folder_cell(count - 1, 0.0);
        let body = geometry.folder_body();
        (last.origin.y + last.size.height - body.origin.y - body.size.height).max(0.0)
    }

    pub(super) fn hit(&self, point: Point, geometry: &Geometry) -> Option<Hit> {
        let state = self.state.borrow();
        let surface = state.surface;
        if point.y < geometry.canvas.origin.y - geometry.gap {
            return Some(
                if point.x > geometry.bounds.origin.x + geometry.bounds.size.width * 0.7 {
                    Hit::Controls
                } else {
                    Hit::Notifications
                },
            );
        }
        match surface {
            Surface::Home => {
                if state.editing
                    && Rect::new(
                        geometry.bounds.origin.x + geometry.gap,
                        geometry.edge.origin.y - 36.0,
                        80.0,
                        36.0,
                    )
                    .contains(point)
                {
                    return Some(Hit::Done);
                }
                if geometry.dock.contains(point) {
                    let index = ((point.y - geometry.dock.origin.y - geometry.gap * 0.5
                        + state.dock_scroll)
                        / geometry.dock_pitch)
                        .floor();
                    return if index >= 0.0 {
                        self.dock_apps()
                            .get(index as usize)
                            .map(|app| Hit::Application(app.bundle_id.clone()))
                    } else {
                        None
                    };
                }
                if geometry.canvas.contains(point) {
                    for item in &state.layout.items {
                        if geometry
                            .home_cell(item.slot, state.scroll.value)
                            .contains(point)
                        {
                            return match &item.kind {
                                ItemKind::Application(id) => Some(Hit::Application(id.clone())),
                                ItemKind::Folder { .. } => Some(Hit::Folder(item.id)),
                                _ => None,
                            };
                        }
                    }
                }
            }
            Surface::Library => {
                if geometry.search.contains(point) {
                    return Some(Hit::Search);
                }
                if !geometry.library_body().contains(point) {
                    return None;
                }
                let apps = self.resources.apps.get();
                let filtered =
                    super::super::app_library::matching_app_indices(&apps, &state.search.value());
                for (index, app) in filtered.iter().enumerate() {
                    if geometry
                        .drawer_cell(index, state.app_scroll)
                        .contains(point)
                    {
                        return Some(Hit::Application(apps[*app].bundle_id.clone()));
                    }
                }
            }
            Surface::Switcher => {
                for (index, entry) in self.entries().iter().enumerate() {
                    let card = geometry.card(index, state.switch_scroll.value);
                    if Rect::new(
                        card.origin.x + card.size.width - 44.0,
                        card.origin.y,
                        44.0,
                        44.0,
                    )
                    .contains(point)
                    {
                        return Some(Hit::Close(entry.process));
                    }
                    if card.contains(point) {
                        return Some(Hit::Application(entry.app.bundle_id.clone()));
                    }
                }
            }
            Surface::Folder(id) => {
                let scroll = state.folder_scroll;
                drop(state);
                if !geometry.folder_body().contains(point) {
                    return None;
                }
                for (index, application) in self.folder_applications(id).iter().enumerate() {
                    if geometry.folder_cell(index, scroll).contains(point) {
                        return Some(Hit::Application(application.clone()));
                    }
                }
                return None;
            }
            Surface::Notifications => {
                if self.notification_settings_bounds(geometry).contains(point)
                {
                    return Some(Hit::Controls);
                }
            }
            _ => {}
        }
        None
    }

    fn paint_app(
        &self,
        app: &AppInfo,
        cell: Rect,
        icon_size: f32,
        foreground: Color,
        context: &mut PaintContext<'_>,
    ) {
        let icon = Rect::new(
            cell.origin.x + (cell.size.width - icon_size) * 0.5,
            cell.origin.y + 8.0,
            icon_size,
            icon_size,
        );
        self.paint_icon(app, icon, context);
        Text::styled(&app.name, TextRole::Caption)
            .alignment(TextAlignment::Center)
            .color(foreground)
            .paint(
                Rect::new(
                    cell.origin.x + 4.0,
                    icon.origin.y + icon_size + 8.0,
                    (cell.size.width - 8.0).max(1.0),
                    28.0,
                ),
                context,
            );
    }

    fn paint_home(&self, geometry: &Geometry, context: &mut PaintContext<'_>) {
        let apps = self.resources.apps.get();
        let state = self.state.borrow();
        context.display_list.push(DrawCommand::PushClip {
            rect: geometry.canvas,
        });
        for item in &state.layout.items {
            let cell = geometry.home_cell(item.slot, state.scroll.value);
            if cell.intersection(geometry.canvas).is_none()
                || state.dragged.is_some_and(|(id, _)| id == item.id)
            {
                continue;
            }
            let icon = geometry.icon_rect(cell);
            match &item.kind {
                ItemKind::Application(id) => {
                    if let Some(app) = apps.iter().find(|app| &app.bundle_id == id) {
                        self.paint_icon(app, icon, context);
                        Text::styled(&app.name, TextRole::Caption)
                            .alignment(TextAlignment::Center)
                            .color(Color::WHITE)
                            .paint(
                                Rect::new(
                                    cell.origin.x,
                                    icon.origin.y + icon.size.height + 8.0,
                                    cell.size.width,
                                    28.0,
                                ),
                                context,
                            );
                    }
                }
                ItemKind::Folder { name, applications } => {
                    Rectangle::new()
                        .color(RectangleColor::Custom(Color::WHITE.with_alpha(74)))
                        .radius(CornerRadius::Custom(icon.size.width * 0.28))
                        .paint(icon, context);
                    for (index, id) in applications.iter().take(4).enumerate() {
                        if let Some(app) = apps.iter().find(|app| &app.bundle_id == id) {
                            let size = icon.size.width * 0.34;
                            let inset = icon.size.width * 0.11;
                            self.paint_icon(
                                app,
                                Rect::new(
                                    icon.origin.x
                                        + inset
                                        + (index % 2) as f32 * icon.size.width * 0.44,
                                    icon.origin.y
                                        + inset
                                        + (index / 2) as f32 * icon.size.height * 0.44,
                                    size,
                                    size,
                                ),
                                context,
                            );
                        }
                    }
                    Text::styled(name, TextRole::Caption)
                        .alignment(TextAlignment::Center)
                        .color(Color::WHITE)
                        .paint(
                            Rect::new(
                                cell.origin.x,
                                icon.origin.y + icon.size.height + 8.0,
                                cell.size.width,
                                28.0,
                            ),
                            context,
                        );
                }
                ItemKind::Widget { .. } => {}
            }
            if state.editing {
                Rectangle::new()
                    .color(RectangleColor::Custom(Color::WHITE.with_alpha(40)))
                    .radius(CornerRadius::Full)
                    .paint(
                        Rect::new(icon.origin.x - 4.0, icon.origin.y - 4.0, 12.0, 12.0),
                        context,
                    );
            }
        }
        context.display_list.push(DrawCommand::PopClip);
        if let Some((id, position)) = state.dragged {
            if let Some(item) = state.layout.items.iter().find(|item| item.id == id) {
                let icon = Rect::new(
                    position.x - geometry.icon * 0.5,
                    position.y - geometry.icon * 0.5,
                    geometry.icon,
                    geometry.icon,
                );
                match &item.kind {
                    ItemKind::Application(bundle) => {
                        if let Some(app) = apps.iter().find(|app| &app.bundle_id == bundle) {
                            self.paint_icon(app, icon, context);
                        }
                    }
                    ItemKind::Folder { applications, .. } => {
                        Rectangle::new()
                            .color(RectangleColor::Custom(Color::WHITE.with_alpha(74)))
                            .radius(CornerRadius::Large)
                            .paint(icon, context);
                        for (index, bundle) in applications.iter().take(4).enumerate() {
                            if let Some(app) = apps.iter().find(|app| &app.bundle_id == bundle) {
                                self.paint_icon(
                                    app,
                                    Rect::new(
                                        icon.origin.x
                                            + geometry.icon * 0.11
                                            + (index % 2) as f32 * geometry.icon * 0.44,
                                        icon.origin.y
                                            + geometry.icon * 0.11
                                            + (index / 2) as f32 * geometry.icon * 0.44,
                                        geometry.icon * 0.34,
                                        geometry.icon * 0.34,
                                    ),
                                    context,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        drop(state);
        self.paint_dock(geometry, context);
        let maximum = self.maximum_scroll(geometry);
        let state = self.state.borrow();
        let step = (geometry.canvas.size.width * 0.5).max(1.0);
        let count = (maximum / step).round() as usize + 1;
        let shown = count.min(9);
        let selected = state.scroll.value / step;
        let first = (selected.floor() as usize)
            .saturating_sub(shown / 2)
            .min(count.saturating_sub(shown));
        let dot = 7.0;
        let gap = context.theme.spacing.small;
        let width = shown as f32 * (dot + gap) - gap;
        for local in 0..shown {
            let index = first + local;
            let proximity = (1.0 - (index as f32 - selected).abs()).clamp(0.0, 1.0);
            Rectangle::new()
                .radius(CornerRadius::Full)
                .color(RectangleColor::Custom(
                    Color::WHITE.with_alpha((100.0 + 155.0 * proximity) as u8),
                ))
                .paint(
                    Rect::new(
                        geometry.bounds.origin.x
                            + (geometry.bounds.size.width - width) * 0.5
                            + local as f32 * (dot + gap),
                        geometry.edge.origin.y - 18.0,
                        dot,
                        dot,
                    ),
                    context,
                );
        }
        if state.editing {
            Button::new("Done")
                .style(ButtonStyle::Ghost)
                .content(Text::label("Done").color(Color::WHITE))
                .paint(
                    Rect::new(
                        geometry.bounds.origin.x + geometry.gap,
                        geometry.edge.origin.y - 36.0,
                        80.0,
                        36.0,
                    ),
                    context,
                );
        }
    }

    fn paint_dock(&self, geometry: &Geometry, context: &mut PaintContext<'_>) {
        let background = context
            .display_list
            .sample_background(geometry.dock, context.theme.shell.window_background);
        let material = super::super::dock_material::DockMaterial::for_backdrop(background);
        Rectangle::new()
            .color(RectangleColor::Custom(material.background))
            .radius(CornerRadius::Custom(
                super::super::desktop_metrics::DOCK_RADIUS,
            ))
            .paint(geometry.dock, context);
        let apps = self.dock_apps();
        let running = self.resources.running.get();
        let scroll = self.state.borrow().dock_scroll;
        context.display_list.push(DrawCommand::PushRoundedClip {
            rect: geometry.dock,
            radius: super::super::desktop_metrics::DOCK_RADIUS,
        });
        for (index, app) in apps.iter().enumerate() {
            let size = (geometry.dock.size.width - geometry.gap).max(24.0);
            let icon = Rect::new(
                geometry.dock.origin.x + geometry.gap * 0.5,
                geometry.dock.origin.y + geometry.gap * 0.5 + index as f32 * geometry.dock_pitch
                    - scroll,
                size,
                size,
            );
            if icon.intersection(geometry.dock).is_none() {
                continue;
            }
            self.paint_icon(app, icon, context);
            if running.contains(&app.bundle_id) {
                Rectangle::new()
                    .color(RectangleColor::Custom(material.running_indicator))
                    .radius(CornerRadius::Full)
                    .paint(
                        Rect::new(
                            icon.origin.x - 5.0,
                            icon.origin.y + size * 0.5 - 2.0,
                            4.0,
                            4.0,
                        ),
                        context,
                    );
            }
        }
        context.display_list.push(DrawCommand::PopClip);
    }

    fn paint_library(&self, geometry: &Geometry, context: &mut PaintContext<'_>) {
        viewkit::prelude::Surface::floating()
            .radius(CornerRadius::Custom(
                context.theme.shell.desktop_corner_radius,
            ))
            .paint(
                Rect::new(
                    geometry.bounds.origin.x + geometry.gap,
                    geometry.application.origin.y,
                    geometry.bounds.size.width - geometry.gap * 2.0,
                    geometry.edge.origin.y - geometry.application.origin.y,
                ),
                context,
            );
        Text::styled("Applications", TextRole::TitleMedium)
            .alignment(TextAlignment::Center)
            .paint(
                Rect::new(
                    geometry.bounds.origin.x,
                    geometry.application.origin.y + geometry.gap,
                    geometry.bounds.size.width,
                    36.0,
                ),
                context,
            );
        self.search().paint(geometry.search, context);
        let apps = self.resources.apps.get();
        let state = self.state.borrow();
        let filtered =
            super::super::app_library::matching_app_indices(&apps, &state.search.value());
        let body = geometry.library_body();
        context
            .display_list
            .push(DrawCommand::PushClip { rect: body });
        for (index, app) in filtered.iter().enumerate() {
            let cell = geometry.drawer_cell(index, state.app_scroll);
            if cell.intersection(body).is_some() {
                self.paint_app(
                    &apps[*app],
                    cell,
                    geometry.icon.min(cell.size.width * 0.6),
                    context.theme.shell.primary_text,
                    context,
                );
            }
        }
        if filtered.is_empty() {
            Text::styled("No applications found", TextRole::Body)
                .alignment(TextAlignment::Center)
                .paint(body, context);
        }
        context.display_list.push(DrawCommand::PopClip);
    }

    fn paint_switcher(&self, geometry: &Geometry, context: &mut PaintContext<'_>) {
        let entries = self.entries();
        let scroll = self.state.borrow().switch_scroll.value;
        let mut requested_preview = false;
        if entries.is_empty() {
            Text::styled("No running applications", TextRole::TitleMedium)
                .alignment(TextAlignment::Center)
                .color(Color::WHITE)
                .paint(geometry.application, context);
        }
        for (index, entry) in entries.iter().enumerate() {
            let mut card = geometry.card(index, scroll);
            let amount = self.state.borrow().sheet.value.clamp(0.0, 1.0);
            if let Some((process, origin)) = self
                .state
                .borrow()
                .switch_origin
                .filter(|(process, _)| *process == entry.process)
            {
                let _ = process;
                card = Rect::new(
                    origin.origin.x + (card.origin.x - origin.origin.x) * amount,
                    origin.origin.y + (card.origin.y - origin.origin.y) * amount,
                    origin.size.width + (card.size.width - origin.size.width) * amount,
                    origin.size.height + (card.size.height - origin.size.height) * amount,
                );
            }
            if card.intersection(geometry.bounds).is_none() {
                continue;
            }
            viewkit::prelude::Surface::floating()
                .radius(CornerRadius::Custom(
                    context.theme.shell.desktop_corner_radius,
                ))
                .paint(card, context);
            let icon = Rect::new(
                card.origin.x + geometry.gap,
                card.origin.y + 8.0,
                28.0,
                28.0,
            );
            self.paint_icon(&entry.app, icon, context);
            Text::styled(&entry.app.name, TextRole::Label).paint(
                Rect::new(
                    icon.origin.x + 36.0,
                    icon.origin.y + 4.0,
                    (card.size.width - 100.0).max(1.0),
                    28.0,
                ),
                context,
            );
            Icon::new(SymbolName::X).size(14.0).paint(
                Rect::new(
                    card.origin.x + card.size.width - 36.0,
                    card.origin.y + 8.0,
                    28.0,
                    28.0,
                ),
                context,
            );
            let preview = Rect::new(
                card.origin.x + 8.0 * amount,
                card.origin.y + 48.0 * amount,
                (card.size.width - 16.0 * amount).max(1.0),
                (card.size.height - 56.0 * amount).max(1.0),
            );
            if let Some(window) = self.resources.windows.with(|desktop| {
                desktop
                    .windows
                    .iter()
                    .rev()
                    .find(|window| window.process_id == Some(entry.process))
                    .cloned()
            }) {
                if window.externally_composited() {
                    let image = window.remote_window.and_then(|id| {
                        if let Some(cached) = self.state.borrow().switch_previews.get(&id) {
                            return cached.clone();
                        }
                        if requested_preview {
                            context.request_redraw_at(Instant::now());
                            return None;
                        }
                        requested_preview = true;
                        let image = self.resources.platform.borrow().native_window_preview(id);
                        self.state.borrow_mut().switch_previews.insert(id, image.clone());
                        image
                    });
                    if let Some(image) = image {
                        Image::new(image).radius(CornerRadius::Large).paint(preview, context);
                    }
                    continue;
                }
                context.display_list.push(DrawCommand::PushRoundedClip {
                    rect: preview,
                    radius: 12.0,
                });
                let content = self.content(&window);
                let scale = (preview.size.width / geometry.application.size.width.max(1.0))
                    .min(preview.size.height / geometry.application.size.height.max(1.0));
                context.with_presentation(
                    PaintTransform {
                        anchor: geometry.application.origin,
                        scale,
                        offset: Point::new(
                            preview.origin.x - geometry.application.origin.x,
                            preview.origin.y - geometry.application.origin.y,
                        ),
                        opacity: 1.0,
                    },
                    false,
                    |context| content.paint(geometry.application, context),
                );
                context.display_list.push(DrawCommand::PopClip);
            } else {
                Text::styled("Application is running", TextRole::Caption)
                    .alignment(TextAlignment::Center)
                    .paint(preview, context);
            }
        }
    }

    fn paint_folder(&self, id: u64, geometry: &Geometry, context: &mut PaintContext<'_>) {
        let panel = geometry.folder();
        viewkit::prelude::Surface::floating()
            .radius(CornerRadius::Custom(
                context.theme.shell.desktop_corner_radius,
            ))
            .paint(panel, context);
        let name = self
            .state
            .borrow()
            .layout
            .items
            .iter()
            .find_map(|item| {
                if item.id == id {
                    if let ItemKind::Folder { name, .. } = &item.kind {
                        Some(name.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default();
        Text::styled(name, TextRole::TitleMedium)
            .alignment(TextAlignment::Center)
            .paint(
                Rect::new(
                    panel.origin.x,
                    panel.origin.y + 12.0,
                    panel.size.width,
                    32.0,
                ),
                context,
            );
        let apps = self.resources.apps.get();
        let scroll = self.state.borrow().folder_scroll;
        context.display_list.push(DrawCommand::PushClip {
            rect: geometry.folder_body(),
        });
        for (index, id) in self.folder_applications(id).iter().enumerate() {
            if let Some(app) = apps.iter().find(|app| &app.bundle_id == id) {
                self.paint_app(
                    app,
                    geometry.folder_cell(index, scroll),
                    geometry
                        .icon
                        .min(geometry.folder_cell(index, scroll).size.width * 0.6),
                    context.theme.shell.primary_text,
                    context,
                );
            }
        }
        context.display_list.push(DrawCommand::PopClip);
    }

    pub(super) fn paint_scene(&self, geometry: Geometry, context: &mut PaintContext<'_>) {
        self.resources.wallpaper.paint(geometry.bounds, context);
        let (surface, previous, amount) = {
            let state = self.state.borrow();
            (
                state.surface,
                state.previous,
                state.sheet.value.clamp(0.0, 1.0),
            )
        };
        let progress = self.state.borrow().app_motion.value.clamp(0.0, 1.0);
        let native_foreground = self
            .foreground()
            .is_some_and(|window| window.externally_composited());
        if matches!(surface, Surface::Home)
            || (!native_foreground && (previous == Surface::Home || progress < 1.0))
        {
            if surface == Surface::Notifications {
                context.with_presentation(
                    PaintTransform {
                        anchor: geometry.bounds.origin,
                        scale: 1.0,
                        offset: Point::new(0.0, 0.0),
                        opacity: 1.0 - amount,
                    },
                    false,
                    |context| self.paint_home(&geometry, context),
                );
            } else {
                self.paint_home(&geometry, context);
            }
        }
        // Panel pixels outside these sensors remain transparent to both
        // composition and input. App events go directly through Kagami.
        if surface == Surface::Application {
            context.display_list.add_input_region(geometry.edge);
            context.display_list.add_input_region(Rect::new(
                geometry.bounds.origin.x,
                geometry.bounds.origin.y,
                geometry.bounds.size.width,
                geometry.application.origin.y - geometry.bounds.origin.y,
            ));
        } else {
            context.display_list.add_input_region(geometry.bounds);
        }
        if let Some(keyboard) = geometry.keyboard {
            context.display_list.add_input_region(keyboard.keys);
        }
        if surface != Surface::Switcher
            && (surface == Surface::Application
                || previous == Surface::Application
                || progress > 0.0)
        {
            let presented = self.state.borrow().presented;
            let window = self.foreground().or_else(|| {
                self.resources.windows.with(|desktop| {
                    desktop
                        .windows
                        .iter()
                        .find(|window| Some(window.id) == presented)
                        .cloned()
                })
            });
            if let Some(window) = window.filter(|window| !window.externally_composited()) {
                let mut transform = self.app_transform(&geometry);
                if surface == Surface::Notifications {
                    transform.opacity *= 1.0 - amount;
                }
                context.with_presentation(
                    transform,
                    surface == Surface::Application && progress > 0.99,
                    |context| {
                        context.display_list.push(DrawCommand::PushRoundedClip {
                            rect: geometry.application,
                            radius: context.theme.shell.desktop_corner_radius
                                * (1.0 - progress)
                                * 2.0
                                / self.app_transform(&geometry).scale,
                        });
                        Rectangle::new()
                            .color(RectangleColor::Custom(
                                context.theme.shell.window_background,
                            ))
                            .paint(geometry.application, context);
                        self.content(&window).paint(geometry.application, context);
                        context.display_list.push(DrawCommand::PopClip);
                    },
                );
            }
        }
        if !matches!(surface, Surface::Home | Surface::Application) && amount > 0.0 {
            Rectangle::new()
                .color(RectangleColor::Custom(
                    Color::BLACK.with_alpha((90.0 * amount) as u8),
                ))
                .paint(geometry.bounds, context);
            let travel = (1.0 - amount) * surface.sheet_travel(geometry.bounds.size.height);
            let offset = match surface {
                Surface::Library => travel,
                Surface::Notifications | Surface::Controls => -travel,
                _ => 0.0,
            };
            context.with_presentation(
                PaintTransform {
                    anchor: geometry.bounds.origin,
                    scale: 1.0,
                    offset: Point::new(0.0, offset),
                    opacity: if surface == Surface::Switcher {
                        1.0
                    } else {
                        amount
                    },
                },
                amount > 0.99,
                |context| match surface {
                    Surface::Library => self.paint_library(&geometry, context),
                    Surface::Notifications => {
                        let clock = self.resources.system_bar.get().clock;
                        let layout = NotificationSheetLayout::new(&geometry, context.theme);
                        Text::styled(clock.time, TextRole::DisplayLarge)
                            .weight(500)
                            .alignment(TextAlignment::Center)
                            .color(Color::WHITE)
                            .paint(layout.time, context);
                        Text::styled(clock.date, TextRole::Label)
                            .alignment(TextAlignment::Center)
                            .color(Color::WHITE.with_alpha(220))
                            .paint(layout.date, context);
                        self.notifications.paint(layout.list, context);
                        self.notification_settings_button()
                            .paint(layout.controls, context);
                    }
                    Surface::Switcher => self.paint_switcher(&geometry, context),
                    Surface::Folder(id) => self.paint_folder(id, &geometry, context),
                    Surface::Controls => self.controls.paint(geometry.bounds, context),
                    _ => {}
                },
            );
        } else {
            self.notifications.paint(geometry.bounds, context);
        }
        let system = self.resources.system_bar.get();
        let foreground = if surface == Surface::Application {
            context.theme.shell.primary_text
        } else {
            Color::WHITE
        };
        if surface != Surface::Notifications {
            Text::styled(
                format!("{}  {}", system.clock.date, system.clock.time),
                TextRole::Caption,
            )
            .color(foreground)
            .paint(
                Rect::new(
                    geometry.bounds.origin.x + geometry.gap,
                    geometry.bounds.origin.y + 12.0,
                    320.0,
                    24.0,
                ),
                context,
            );
        }
        let right = geometry.bounds.origin.x + geometry.bounds.size.width - geometry.gap;
        if system.battery.available {
            Icon::new(SymbolName::BatteryFull)
                .size(24.0)
                .color(foreground)
                .paint(
                    Rect::new(right - 64.0, geometry.bounds.origin.y + 10.0, 28.0, 24.0),
                    context,
                );
        }
        if system.network.available() {
            Icon::new(SymbolName::Wifi)
                .size(20.0)
                .color(foreground)
                .paint(
                    Rect::new(right - 24.0, geometry.bounds.origin.y + 10.0, 24.0, 24.0),
                    context,
                );
        }
        if let Some(keyboard) = geometry.keyboard {
            let state = self.state.borrow();
            state.keyboard.panel.paint_with_footer(
                &state.keyboard.layouts,
                keyboard.keys,
                if keyboard.docked {
                    geometry.edge.size.height
                } else {
                    0.0
                },
                system.japanese_input,
                context,
            );
        }
        let width = (geometry.bounds.size.width * 0.22).clamp(112.0, 280.0);
        let navigation_amount = self.state.borrow().navigation_bar.amount(amount);
        if navigation_amount > 0.001 {
            let navigation_color = viewkit::animation::interpolate(
                Color::WHITE,
                context.theme.shell.primary_text,
                progress * (1.0 - amount),
            );
            let height = 5.0;
            let bottom = geometry.edge.origin.y + geometry.edge.size.height;
            Rectangle::new()
                .color(RectangleColor::Custom(
                    navigation_color.with_alpha((190.0 * navigation_amount) as u8),
                ))
                .radius(CornerRadius::Full)
                .paint(
                    Rect::new(
                        geometry.bounds.origin.x + (geometry.bounds.size.width - width) * 0.5,
                        bottom - (geometry.edge.size.height * navigation_amount + height) * 0.5,
                        width,
                        height,
                    ),
                    context,
                );
        }
        if let Some(error) = &self.state.borrow().error {
            Text::styled(error, TextRole::Caption)
                .color(Color::WHITE)
                .alignment(TextAlignment::Center)
                .paint(
                    Rect::new(
                        geometry.bounds.origin.x,
                        geometry.edge.origin.y - 60.0,
                        geometry.bounds.size.width,
                        32.0,
                    ),
                    context,
                );
        }
        let mut state = self.state.borrow_mut();
        if let Some((bounds, press)) = &state.press {
            let points = viewkit::omochi::rounded_rect_points(
                bounds.size.width,
                bounds.size.height,
                12.0,
                8,
            )
            .into_iter()
            .map(|point| Point::new(point.x + bounds.origin.x, point.y + bounds.origin.y))
            .collect::<Vec<_>>();
            let sample = press.sample(&points, Instant::now());
            if sample.animating {
                context.request_redraw_at(Instant::now() + std::time::Duration::from_millis(16));
            } else if !press.is_active() {
                state.press = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_column_keeps_header_and_touch_controls_clear_in_both_orientations() {
        for (width, height) in [(800., 1100.), (1280., 800.), (600., 480.)] {
            let geometry = Geometry::for_environment(
                Rect::new(0., 0., width, height),
                &Theme::LIGHT,
                3,
                WindowEnvironment {
                    safe_area: viewkit::edge_insets::EdgeInsets::new(20., 0., 16., 0.),
                    ..Default::default()
                },
            );
            let layout = NotificationSheetLayout::new(&geometry, &Theme::LIGHT);
            assert!(layout.list.origin.y > layout.date.origin.y + layout.date.size.height);
            assert!(layout.list.origin.x >= geometry.bounds.origin.x);
            assert!(layout.list.origin.y + layout.list.size.height <= geometry.edge.origin.y);
            assert!(layout.list.size.width <= Theme::LIGHT.layout.content_max_width);
            assert_eq!(layout.controls.size, Size::new(44., 44.));
            assert!(
                (layout.list.origin.x + layout.list.size.width * 0.5 - width * 0.5).abs() < 0.01
            );
        }
    }
    #[test]
    fn keyboard_reduces_library_without_resizing_active_application() {
        let bounds = Rect::new(0., 0., 800., 1100.);
        let environment = WindowEnvironment {
            input_mode: InputMode::Touch,
            safe_area: viewkit::edge_insets::EdgeInsets::new(24., 0., 20., 0.),
            ..Default::default()
        };
        let home = Geometry::for_environment(bounds, &Theme::LIGHT, 3, environment);
        let keyboard = Geometry::for_environment(
            bounds,
            &Theme::LIGHT,
            3,
            WindowEnvironment {
                keyboard_occlusion: Some(Rect::new(0., 700., 800., 400.)),
                ..environment
            },
        );
        assert_eq!(home.application, keyboard.application);
        assert_eq!(home.bounds, environment.safe_bounds(bounds));
        let library = keyboard.library_body();
        assert_eq!(library.origin.y + library.size.height, 700.);
        assert!(home.library_body().size.height > library.size.height);
    }

    #[test]
    fn docked_keys_reserve_app_space_but_floating_keys_preserve_it() {
        let theme = &Theme::LIGHT;
        let mut keyboard = keyboard::Keyboard::default();
        keyboard.layouts = viewkit::software_keyboard::parse_layouts(
            "[US]\na b c\n[かな]\n+あいうえお +かきくけこ",
        )
        .unwrap();
        keyboard.update_target(Some(keyboard::Target::Search));
        for (width, height) in [(1920., 1080.), (720., 1080.), (600., 480.)] {
            let screen = Rect::new(0., 0., width, height);
            let full = Geometry::new(screen, theme, 5).application;
            for selected in [0, 1] {
                keyboard.panel.selected = selected;
                let home = Geometry::new(screen, theme, 5);
                let panel = keyboard.geometry(home.edge, height, theme).unwrap();
                let edge = home.edge;
                let app = home.with_keyboard(Some(panel)).application;
                assert_eq!(app.origin, full.origin);
                assert!(panel.keys.origin.y + panel.keys.size.height <= edge.origin.y);
                if selected == 0 {
                    assert_eq!(panel.keys.size.width, width);
                    assert_eq!(app.origin.y + app.size.height, panel.keys.origin.y);
                } else {
                    assert!(panel.keys.size.width <= theme.controls.touch_height * 8.5);
                    assert!(panel.keys.origin.x > width * 0.25);
                    assert_eq!(app, full);
                }
            }
        }
    }

    #[test]
    fn application_starts_at_the_home_icon_width_in_each_orientation() {
        for (width, height) in [(1280., 800.), (800., 1280.), (640., 480.)] {
            let geometry = Geometry::new(Rect::new(0., 0., width, height), &Theme::LIGHT, 7);
            let width = geometry.application.size.width * geometry.application_icon_scale();
            assert!((width - geometry.icon).abs() < 0.001);
        }
    }

    #[test]
    fn icon_footprint_grows_with_the_logical_viewport() {
        let small = Geometry::new(Rect::new(0.0, 0.0, 800.0, 600.0), &Theme::LIGHT, 7);
        let medium = Geometry::new(Rect::new(0.0, 0.0, 1280.0, 800.0), &Theme::LIGHT, 7);
        let large = Geometry::new(Rect::new(0.0, 0.0, 1920.0, 1200.0), &Theme::LIGHT, 7);
        assert!(small.icon < medium.icon && medium.icon < large.icon);
        assert!(small.dock.size.width < large.dock.size.width);
    }
    #[test]
    fn rotation_keeps_icons_inside_the_canvas_and_clear_of_fixed_dock() {
        for size in [(1280., 800.), (800., 1280.), (640., 480.)] {
            let geometry = Geometry::new(Rect::new(0., 0., size.0, size.1), &Theme::LIGHT, 7);
            assert!(
                geometry.canvas.origin.x + geometry.canvas.size.width <= geometry.dock.origin.x
            );
            assert!(
                geometry.application.origin.y + geometry.application.size.height
                    <= geometry.bounds.origin.y + geometry.bounds.size.height
            );
            for row in 0..model::ROWS {
                let cell = geometry.home_cell(Slot { column: 0, row }, 0.0);
                let icon = geometry.icon_rect(cell);
                assert!(icon.origin.y + icon.size.height <= geometry.edge.origin.y);
            }
        }
    }
}
