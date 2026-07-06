use std::{
    cmp::min,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use serde::{Deserialize, Serialize};

const CACHE_FILE_NAME: &str = "plugins.json";
const CACHE_VERSION: u8 = 2;
const REFRESH_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DETAILS_SCROLL_STEP: usize = 8;

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = App::new()?.run(&mut terminal);
    ratatui::restore();
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Plugin {
    name: String,
    family: String,
    format: PluginFormat,
    scope: PluginScope,
    path: PathBuf,
    version: Option<String>,
    modified: Option<u64>,
}

impl Plugin {
    fn title(&self) -> String {
        let version = self
            .version
            .as_deref()
            .map(|version| format!(" {version}"))
            .unwrap_or_default();
        format!("{}{}", self.name, version)
    }

    fn summary(&self) -> String {
        format!("{} - {} - {}", self.format, self.scope, self.path.display())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PluginFormat {
    Vst2,
    Vst3,
    AudioUnit,
}

impl std::fmt::Display for PluginFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginFormat::Vst2 => f.write_str("VST2"),
            PluginFormat::Vst3 => f.write_str("VST3"),
            PluginFormat::AudioUnit => f.write_str("AU"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PluginScope {
    System,
    User,
}

impl std::fmt::Display for PluginScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginScope::System => f.write_str("System"),
            PluginScope::User => f.write_str("User"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PluginCache {
    #[serde(default)]
    version: u8,
    generated_at: u64,
    plugins: Vec<Plugin>,
}

#[derive(Debug, Clone)]
struct PluginRoot {
    path: PathBuf,
    format: PluginFormat,
    scope: PluginScope,
}

#[derive(Debug)]
struct App {
    plugins: Vec<Plugin>,
    roots: Vec<PluginRoot>,
    selected: usize,
    list_state: ListState,
    details_scroll: usize,
    status: String,
    confirm: Option<ConfirmAction>,
    notice: Option<Notice>,
    cache_generated_at: u64,
    last_refresh_check: Instant,
}

#[derive(Debug, Clone)]
struct Notice {
    title: String,
    lines: Vec<String>,
}

#[derive(Debug, Clone)]
enum ConfirmAction {
    DeleteSelected(Plugin),
    DeleteFamily {
        family: String,
        plugins: Vec<Plugin>,
    },
}

impl ConfirmAction {
    fn title(&self) -> &'static str {
        match self {
            ConfirmAction::DeleteSelected(_) => "Delete selected plugin?",
            ConfirmAction::DeleteFamily { .. } => "Delete all related versions?",
        }
    }

    fn plugins(&self) -> &[Plugin] {
        match self {
            ConfirmAction::DeleteSelected(plugin) => std::slice::from_ref(plugin),
            ConfirmAction::DeleteFamily { plugins, .. } => plugins,
        }
    }
}

impl App {
    fn new() -> Result<Self> {
        let roots = plugin_roots();
        let cache = load_or_refresh_cache(&roots)?;
        let mut app = Self {
            plugins: cache.plugins,
            roots,
            selected: 0,
            list_state: ListState::default(),
            details_scroll: 0,
            status: format!("Loaded cache from {}", format_unix_time(cache.generated_at)),
            confirm: None,
            notice: None,
            cache_generated_at: cache.generated_at,
            last_refresh_check: Instant::now(),
        };
        app.normalize_selection();
        Ok(app)
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press && self.handle_key(key)? {
                        return Ok(());
                    }
                }
            }

            self.refresh_if_roots_changed()?;
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let [main_area, help_area] = frame.area().layout(&Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(2),
        ]));
        let [list_area, details_area] = main_area.layout(&Layout::horizontal([
            Constraint::Percentage(42),
            Constraint::Percentage(58),
        ]));

        self.render_plugin_list(frame, list_area);
        self.render_details(frame, details_area);
        self.render_help(frame, help_area);

        if let Some(confirm) = &self.confirm {
            render_confirm(frame, frame.area(), confirm);
        }

        if let Some(notice) = &self.notice {
            render_notice(frame, frame.area(), notice);
        }
    }

    fn render_plugin_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .plugins
            .iter()
            .map(|plugin| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(plugin.format.to_string(), Style::new().fg(Color::Cyan)),
                        Span::raw(" "),
                        Span::raw(plugin.title()),
                    ]),
                    Line::from(Span::styled(
                        plugin.summary(),
                        Style::new().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect();

        let title = format!(" Audio Plugins ({}) ", self.plugins.len());
        let list = List::new(items)
            .block(Block::new().title(title).borders(Borders::ALL))
            .highlight_style(
                Style::new()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_details(&mut self, frame: &mut Frame, area: Rect) {
        let lines = self.details_lines();
        let max_scroll = lines
            .len()
            .saturating_sub(area.height.saturating_sub(2) as usize);
        self.details_scroll = min(self.details_scroll, max_scroll);

        let paragraph = Paragraph::new(lines)
            .block(Block::new().title(" Details ").borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .scroll((self.details_scroll as u16, 0));
        frame.render_widget(paragraph, area);

        if max_scroll > 0 {
            let mut scrollbar_state =
                ScrollbarState::new(max_scroll + 1).position(self.details_scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut scrollbar_state,
            );
        }
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let help = Line::from(vec![
            Span::styled(" j/k ", Style::new().fg(Color::Yellow)),
            Span::raw("move  "),
            Span::styled(" ctrl-d/ctrl-u ", Style::new().fg(Color::Yellow)),
            Span::raw("details scroll  "),
            Span::styled(" r ", Style::new().fg(Color::Yellow)),
            Span::raw("refresh  "),
            Span::styled(" d ", Style::new().fg(Color::Yellow)),
            Span::raw("delete selected  "),
            Span::styled(" D ", Style::new().fg(Color::Yellow)),
            Span::raw("delete related  "),
            Span::styled(" q ", Style::new().fg(Color::Yellow)),
            Span::raw("quit  "),
            Span::styled(self.status.as_str(), Style::new().fg(Color::DarkGray)),
        ]);
        frame.render_widget(
            Paragraph::new(help).block(Block::new().borders(Borders::TOP)),
            area,
        );
    }

    fn details_lines(&self) -> Vec<Line<'static>> {
        let Some(plugin) = self.selected_plugin() else {
            return vec![
                Line::from("No audio plugins found."),
                Line::from(""),
                Line::from("Press r to scan standard macOS VST and Audio Unit locations."),
            ];
        };

        let related = self.related_plugins(plugin);
        let mut lines = vec![
            Line::from(Span::styled(
                plugin.title(),
                Style::new().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            labeled_line("Format", plugin.format.to_string()),
            labeled_line("Scope", plugin.scope.to_string()),
            labeled_line(
                "Version",
                plugin
                    .version
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            labeled_line(
                "Modified",
                plugin
                    .modified
                    .map(format_unix_time)
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            labeled_line("Path", plugin.path.display().to_string()),
            Line::from(""),
            Line::from(Span::styled(
                "Related versions",
                Style::new().add_modifier(Modifier::BOLD),
            )),
        ];

        for candidate in related {
            lines.push(Line::from(format!(
                "- {} - {} - {} - {}",
                candidate.name,
                candidate.format,
                candidate.scope,
                candidate.version.as_deref().unwrap_or("unknown")
            )));
            lines.push(Line::from(Span::styled(
                format!("  {}", candidate.path.display()),
                Style::new().fg(Color::DarkGray),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(
            "Delete selected removes only the highlighted bundle.",
        ));
        lines.push(Line::from(
            "Delete related removes every matching VST2/VST3/AU bundle shown above.",
        ));
        lines.push(Line::from(
            "System plugins under /Library may require elevated permissions.",
        ));
        lines
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.notice.is_some() {
            self.notice = None;
            return Ok(matches!(key.code, KeyCode::Char('q') | KeyCode::Esc));
        }

        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }

        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => return Ok(true),
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => self.select_next(),
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => self.select_previous(),
            (_, KeyCode::Char('g')) => self.select_first(),
            (_, KeyCode::Char('G')) => self.select_last(),
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => self.scroll_details_down(),
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => self.scroll_details_up(),
            (_, KeyCode::Char('r')) => self.refresh("Manual refresh complete")?,
            (_, KeyCode::Char('d')) => self.confirm_delete_selected(),
            (_, KeyCode::Char('D')) => self.confirm_delete_family(),
            _ => {}
        }

        Ok(false)
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') => self.delete_confirmed()?,
            KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {
                self.confirm = None;
                self.status = "Delete cancelled".to_string();
            }
            _ => {}
        }
        Ok(false)
    }

    fn select_next(&mut self) {
        if self.plugins.is_empty() {
            return;
        }
        self.selected = min(self.selected + 1, self.plugins.len() - 1);
        self.details_scroll = 0;
        self.normalize_selection();
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.details_scroll = 0;
        self.normalize_selection();
    }

    fn select_first(&mut self) {
        self.selected = 0;
        self.details_scroll = 0;
        self.normalize_selection();
    }

    fn select_last(&mut self) {
        if !self.plugins.is_empty() {
            self.selected = self.plugins.len() - 1;
            self.details_scroll = 0;
            self.normalize_selection();
        }
    }

    fn scroll_details_down(&mut self) {
        self.details_scroll = self.details_scroll.saturating_add(DETAILS_SCROLL_STEP);
    }

    fn scroll_details_up(&mut self) {
        self.details_scroll = self.details_scroll.saturating_sub(DETAILS_SCROLL_STEP);
    }

    fn confirm_delete_selected(&mut self) {
        let Some(plugin) = self.selected_plugin() else {
            self.status = "Nothing selected".to_string();
            return;
        };
        self.confirm = Some(ConfirmAction::DeleteSelected(plugin.clone()));
    }

    fn confirm_delete_family(&mut self) {
        let Some(plugin) = self.selected_plugin() else {
            self.status = "Nothing selected".to_string();
            return;
        };
        let related = self.related_plugins(plugin);
        self.confirm = Some(ConfirmAction::DeleteFamily {
            family: plugin.family.clone(),
            plugins: related,
        });
    }

    fn delete_confirmed(&mut self) -> Result<()> {
        let Some(action) = self.confirm.take() else {
            return Ok(());
        };

        let plugins = action.plugins().to_vec();
        let mut deleted = 0;
        let mut failures = Vec::new();

        for plugin in plugins {
            match delete_plugin_bundle(&plugin, &self.roots) {
                Ok(()) => deleted += 1,
                Err(error) => failures.push(format!("{}: {error}", plugin.path.display())),
            }
        }

        self.refresh_after_delete()?;
        if failures.is_empty() {
            self.status = format!("Deleted {deleted} plugin bundle(s)");
        } else {
            let failed_count = failures.len();
            self.status = format!("Deleted {deleted}; failed {failed_count}");
            let mut lines = vec![
                format!("Deleted {deleted} plugin bundle(s)."),
                format!("Failed to delete {failed_count} plugin bundle(s)."),
                "".to_string(),
                "If this is a system plugin under /Library, run vstui with permissions that can write there.".to_string(),
                "".to_string(),
            ];
            lines.extend(failures);
            self.notice = Some(Notice {
                title: "Delete failed".to_string(),
                lines,
            });
        }

        Ok(())
    }

    fn refresh_after_delete(&mut self) -> Result<()> {
        self.refresh("Cache refreshed after delete")
    }

    fn refresh_if_roots_changed(&mut self) -> Result<()> {
        if self.last_refresh_check.elapsed() < REFRESH_POLL_INTERVAL {
            return Ok(());
        }
        self.last_refresh_check = Instant::now();

        if cached_plugins_changed(&self.plugins)
            || roots_changed_since(&self.roots, self.cache_generated_at)
        {
            self.refresh("Plugin directory changed; cache refreshed")?;
        }
        Ok(())
    }

    fn refresh(&mut self, status: &str) -> Result<()> {
        let cache = scan_and_store_cache(&self.roots)?;
        let old_path = self.selected_plugin().map(|plugin| plugin.path.clone());
        self.plugins = cache.plugins;
        self.cache_generated_at = cache.generated_at;
        self.selected = old_path
            .and_then(|path| self.plugins.iter().position(|plugin| plugin.path == path))
            .unwrap_or_else(|| min(self.selected, self.plugins.len().saturating_sub(1)));
        self.details_scroll = 0;
        self.status = format!("{status} at {}", format_unix_time(cache.generated_at));
        self.normalize_selection();
        Ok(())
    }

    fn selected_plugin(&self) -> Option<&Plugin> {
        self.plugins.get(self.selected)
    }

    fn related_plugins(&self, plugin: &Plugin) -> Vec<Plugin> {
        self.plugins
            .iter()
            .filter(|candidate| candidate.family == plugin.family)
            .cloned()
            .collect()
    }

    fn normalize_selection(&mut self) {
        if self.plugins.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = min(self.selected, self.plugins.len() - 1);
            self.list_state.select(Some(self.selected));
        }
    }
}

fn render_confirm(frame: &mut Frame, area: Rect, confirm: &ConfirmAction) {
    let popup = centered_rect(68, 42, area);
    frame.render_widget(Clear, popup);

    let mut lines = vec![
        Line::from(Span::styled(
            confirm.title(),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let ConfirmAction::DeleteFamily { family, .. } = confirm {
        lines.push(labeled_line("Family", family.clone()));
        lines.push(Line::from(""));
    }

    for plugin in confirm.plugins().iter().take(8) {
        lines.push(Line::from(format!(
            "- {} - {} - {}",
            plugin.name, plugin.format, plugin.scope
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", plugin.path.display()),
            Style::new().fg(Color::DarkGray),
        )));
    }

    if confirm.plugins().len() > 8 {
        lines.push(Line::from(format!(
            "...and {} more",
            confirm.plugins().len() - 8
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "y",
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" delete permanently, "),
        Span::styled(
            "n",
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" cancel"),
    ]));

    let paragraph = Paragraph::new(lines)
        .block(
            Block::new()
                .title(" Confirm ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Red)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn render_notice(frame: &mut Frame, area: Rect, notice: &Notice) {
    let popup = centered_rect(76, 48, area);
    frame.render_widget(Clear, popup);

    let mut lines: Vec<Line> = notice
        .lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to dismiss",
        Style::new().fg(Color::Yellow),
    )));

    let paragraph = Paragraph::new(lines)
        .block(
            Block::new()
                .title(format!(" {} ", notice.title))
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Red)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn labeled_line(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::new().fg(Color::Yellow)),
        Span::raw(value),
    ])
}

fn plugin_roots() -> Vec<PluginRoot> {
    let mut roots = vec![
        PluginRoot {
            path: PathBuf::from("/Library/Audio/Plug-Ins/VST"),
            format: PluginFormat::Vst2,
            scope: PluginScope::System,
        },
        PluginRoot {
            path: PathBuf::from("/Library/Audio/Plug-Ins/VST3"),
            format: PluginFormat::Vst3,
            scope: PluginScope::System,
        },
        PluginRoot {
            path: PathBuf::from("/Library/Audio/Plug-Ins/Components"),
            format: PluginFormat::AudioUnit,
            scope: PluginScope::System,
        },
    ];

    if let Some(home) = dirs::home_dir() {
        roots.push(PluginRoot {
            path: home.join("Library/Audio/Plug-Ins/VST"),
            format: PluginFormat::Vst2,
            scope: PluginScope::User,
        });
        roots.push(PluginRoot {
            path: home.join("Library/Audio/Plug-Ins/VST3"),
            format: PluginFormat::Vst3,
            scope: PluginScope::User,
        });
        roots.push(PluginRoot {
            path: home.join("Library/Audio/Plug-Ins/Components"),
            format: PluginFormat::AudioUnit,
            scope: PluginScope::User,
        });
    }

    roots
}

fn load_or_refresh_cache(roots: &[PluginRoot]) -> Result<PluginCache> {
    if let Some(cache) = load_cache()? {
        if !cache_is_stale(&cache, roots) {
            return Ok(cache);
        }
    }

    scan_and_store_cache(roots)
}

fn cache_is_stale(cache: &PluginCache, roots: &[PluginRoot]) -> bool {
    cache.version != CACHE_VERSION
        || cached_plugins_changed(&cache.plugins)
        || roots_changed_since(roots, cache.generated_at)
}

fn cached_plugins_changed(plugins: &[Plugin]) -> bool {
    plugins
        .iter()
        .any(|plugin| !is_installed_plugin_bundle(&plugin.path, plugin.format))
}

fn roots_changed_since(roots: &[PluginRoot], generated_at: u64) -> bool {
    roots.iter().any(|root| {
        root.path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(system_time_to_unix)
            .is_some_and(|modified| modified > generated_at)
    })
}

fn load_cache() -> Result<Option<PluginCache>> {
    let path = cache_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cache =
        serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(cache))
}

fn scan_and_store_cache(roots: &[PluginRoot]) -> Result<PluginCache> {
    let mut plugins = scan_plugins(roots)?;
    plugins.sort_by(|left, right| {
        left.family
            .cmp(&right.family)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.format.to_string().cmp(&right.format.to_string()))
            .then_with(|| left.scope.to_string().cmp(&right.scope.to_string()))
            .then_with(|| left.path.cmp(&right.path))
    });

    let cache = PluginCache {
        version: CACHE_VERSION,
        generated_at: unix_now(),
        plugins,
    };
    write_cache(&cache)?;
    Ok(cache)
}

fn write_cache(cache: &PluginCache) -> Result<()> {
    let path = cache_file_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("cache path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let contents = serde_json::to_string_pretty(cache)?;
    fs::write(&path, contents).with_context(|| format!("write {}", path.display()))
}

fn cache_file_path() -> Result<PathBuf> {
    let base =
        dirs::cache_dir().ok_or_else(|| anyhow!("could not resolve user cache directory"))?;
    Ok(base.join("vstui").join(CACHE_FILE_NAME))
}

fn scan_plugins(roots: &[PluginRoot]) -> Result<Vec<Plugin>> {
    let mut plugins = Vec::new();

    for root in roots {
        let Ok(entries) = fs::read_dir(&root.path) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !is_installed_plugin_bundle(&path, root.format) {
                continue;
            }

            let name = plugin_name(&path);
            plugins.push(Plugin {
                family: normalize_family(&name),
                name,
                format: root.format,
                scope: root.scope,
                modified: modified_unix(&path),
                version: plugin_version(&path),
                path,
            });
        }
    }

    Ok(plugins)
}

fn is_plugin_bundle(path: &Path, format: PluginFormat) -> bool {
    let expected_extension = match format {
        PluginFormat::Vst2 => "vst",
        PluginFormat::Vst3 => "vst3",
        PluginFormat::AudioUnit => "component",
    };
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
}

fn is_installed_plugin_bundle(path: &Path, format: PluginFormat) -> bool {
    if !is_plugin_bundle(path, format) {
        return false;
    }

    let Ok(metadata) = path.metadata() else {
        return false;
    };

    metadata.is_file() || path.join("Contents/Info.plist").is_file()
}

fn plugin_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

fn normalize_family(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect()
}

fn plugin_version(path: &Path) -> Option<String> {
    let info_plist = path.join("Contents/Info.plist");
    let contents = fs::read_to_string(info_plist).ok()?;
    read_plist_string(&contents, "CFBundleShortVersionString")
        .or_else(|| read_plist_string(&contents, "CFBundleVersion"))
}

fn read_plist_string(contents: &str, key: &str) -> Option<String> {
    let key_marker = format!("<key>{key}</key>");
    let after_key = contents.split_once(&key_marker)?.1;
    let after_open = after_key.split_once("<string>")?.1;
    let value = after_open.split_once("</string>")?.0.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn delete_plugin_bundle(plugin: &Plugin, roots: &[PluginRoot]) -> Result<()> {
    let path = &plugin.path;

    if !is_under_plugin_root(path, roots) {
        return Err(anyhow!(
            "refusing to delete outside known audio plugin roots"
        ));
    }

    if !is_plugin_bundle(path, plugin.format) {
        return Err(anyhow!("refusing to delete path with unexpected extension"));
    }

    let metadata = path
        .symlink_metadata()
        .with_context(|| format!("inspect {}", path.display()))?;
    let file_type = metadata.file_type();

    if file_type.is_symlink() || metadata.is_file() {
        fs::remove_file(path).with_context(|| format!("delete file {}", path.display()))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("delete directory {}", path.display()))?;
    } else {
        return Err(anyhow!("unsupported plugin path type"));
    }

    match path.symlink_metadata() {
        Ok(_) => Err(anyhow!("path still exists after delete")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("verify delete {}", path.display())),
    }
}

fn is_under_plugin_root(path: &Path, roots: &[PluginRoot]) -> bool {
    roots.iter().any(|root| path.starts_with(&root.path))
}

fn modified_unix(path: &Path) -> Option<u64> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(system_time_to_unix)
}

fn system_time_to_unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn unix_now() -> u64 {
    system_time_to_unix(SystemTime::now()).unwrap_or_default()
}

fn format_unix_time(timestamp: u64) -> String {
    format!("{timestamp}s since epoch")
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn scan_skips_empty_leftover_bundle_directories() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir(temp.path().join("Removed.vst3")).expect("create stale bundle");
        create_plugin_bundle(&temp.path().join("Installed.vst3"));

        let plugins = scan_plugins(&[PluginRoot {
            path: temp.path().to_path_buf(),
            format: PluginFormat::Vst3,
            scope: PluginScope::User,
        }])
        .expect("scan plugins");

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "Installed");
    }

    #[test]
    fn stale_cache_detects_empty_bundle_directories() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let path = temp.path().join("Removed.component");
        fs::create_dir(&path).expect("create stale component");

        let plugins = vec![Plugin {
            name: "Removed".to_string(),
            family: "removed".to_string(),
            format: PluginFormat::AudioUnit,
            scope: PluginScope::User,
            path,
            version: None,
            modified: None,
        }];

        assert!(cached_plugins_changed(&plugins));
    }

    #[test]
    fn scan_skips_broken_plugin_symlinks() {
        let temp = tempfile::tempdir().expect("create tempdir");
        symlink(
            temp.path().join("missing-target.vst3"),
            temp.path().join("Ghost.vst3"),
        )
        .expect("create broken symlink");

        let plugins = scan_plugins(&[PluginRoot {
            path: temp.path().to_path_buf(),
            format: PluginFormat::Vst3,
            scope: PluginScope::User,
        }])
        .expect("scan plugins");

        assert!(plugins.is_empty());
    }

    #[test]
    fn delete_plugin_bundle_removes_symlink_without_deleting_target() {
        let root = tempfile::tempdir().expect("create root tempdir");
        let target_parent = tempfile::tempdir().expect("create target tempdir");
        let target = target_parent.path().join("Target.vst3");
        let link = root.path().join("Linked.vst3");
        create_plugin_bundle(&target);
        symlink(&target, &link).expect("create plugin symlink");

        let plugin = Plugin {
            name: "Linked".to_string(),
            family: "linked".to_string(),
            format: PluginFormat::Vst3,
            scope: PluginScope::User,
            path: link.clone(),
            version: None,
            modified: None,
        };
        let roots = [PluginRoot {
            path: root.path().to_path_buf(),
            format: PluginFormat::Vst3,
            scope: PluginScope::User,
        }];

        delete_plugin_bundle(&plugin, &roots).expect("delete plugin symlink");

        assert!(!link.exists());
        assert!(target.join("Contents/Info.plist").is_file());
    }

    fn create_plugin_bundle(path: &Path) {
        fs::create_dir_all(path.join("Contents")).expect("create bundle contents");
        fs::write(
            path.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleShortVersionString</key>
  <string>1.2.3</string>
</dict>
</plist>
"#,
        )
        .expect("write plist");
    }
}
