#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align, Color32, FontId, RichText, Rounding, Stroke};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, SystemTime},
};

const ONE_DAY: Duration = Duration::from_secs(24 * 60 * 60);
const TWO_DAYS: Duration = Duration::from_secs(48 * 60 * 60);
const BG: Color32 = Color32::from_rgb(13, 19, 31);
const SIDEBAR: Color32 = Color32::from_rgb(17, 26, 42);
const SURFACE: Color32 = Color32::from_rgb(23, 33, 51);
const BORDER: Color32 = Color32::from_rgb(43, 57, 78);
const FG: Color32 = Color32::from_rgb(239, 245, 255);
const MUTED: Color32 = Color32::from_rgb(153, 170, 193);
const BLUE: Color32 = Color32::from_rgb(77, 145, 255);
const GREEN: Color32 = Color32::from_rgb(61, 213, 168);
const AMBER: Color32 = Color32::from_rgb(248, 193, 104);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    UserTemp,
    WindowsTemp,
    DirectX,
    Nvidia,
    Thumbnails,
    IconCache,
    EdgeWeb,
    EdgeCode,
    EdgeGraphics,
    EdgeMedia,
}

const CATEGORIES: [Category; 10] = [
    Category::UserTemp, Category::WindowsTemp, Category::DirectX, Category::Nvidia,
    Category::Thumbnails, Category::IconCache, Category::EdgeWeb,
    Category::EdgeCode, Category::EdgeGraphics, Category::EdgeMedia,
];

impl Category {
    fn title(self) -> &'static str {
        match self {
            Self::UserTemp => "Temporários do usuário",
            Self::WindowsTemp => "Temporários do Windows",
            Self::DirectX => "Shaders DirectX",
            Self::Nvidia => "Shaders NVIDIA",
            Self::Thumbnails => "Miniaturas do Explorador",
            Self::IconCache => "Ícones do Explorador",
            Self::EdgeWeb => "Páginas e imagens do Edge",
            Self::EdgeCode => "Código armazenado pelo Edge",
            Self::EdgeGraphics => "Cache gráfico do Edge",
            Self::EdgeMedia => "Cache de mídia do Edge",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::UserTemp => "Arquivos temporários antigos do seu perfil; mínimo de 48 horas.",
            Self::WindowsTemp => "Temporários antigos da pasta Windows\\Temp; pode exigir permissão.",
            Self::DirectX => "Shaders compilados que jogos e programas podem recriar.",
            Self::Nvidia => "Shaders DX/GL da NVIDIA, quando o driver estiver instalado.",
            Self::Thumbnails => "Bancos de miniaturas thumbcache_*.db do Explorador.",
            Self::IconCache => "Bancos iconcache_*.db usados nas visualizações do Windows.",
            Self::EdgeWeb => "Cache de rede e arquivos de páginas, inclusive Network\\Cache.",
            Self::EdgeCode => "Code Cache de scripts compilados pelo navegador.",
            Self::EdgeGraphics => "Cache de GPU, Dawn e shaders em todos os perfis.",
            Self::EdgeMedia => "Arquivos temporários de reprodução de mídia.",
        }
    }
    fn is_edge(self) -> bool {
        matches!(self, Self::EdgeWeb | Self::EdgeCode | Self::EdgeGraphics | Self::EdgeMedia)
    }
    fn min_age(self) -> Duration {
        match self {
            Self::UserTemp | Self::WindowsTemp => TWO_DAYS,
            Self::EdgeWeb | Self::EdgeCode | Self::EdgeGraphics | Self::EdgeMedia => Duration::ZERO,
            _ => ONE_DAY,
        }
    }
    fn matches_file(self, path: &Path) -> bool {
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else { return false; };
        let filename = filename.to_ascii_lowercase();
        match self {
            Self::Thumbnails => filename.starts_with("thumbcache_") && filename.ends_with(".db"),
            Self::IconCache => filename.starts_with("iconcache_") && filename.ends_with(".db"),
            _ => true,
        }
    }
    fn restrict_to_root(self) -> bool { matches!(self, Self::Thumbnails | Self::IconCache) }
}

#[derive(Default, Clone, Copy)]
struct Stats { bytes: u64, files: u64, errors: u64, too_recent: u64 }
impl Stats {
    fn accumulate(&mut self, other: &Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.files = self.files.saturating_add(other.files);
        self.errors = self.errors.saturating_add(other.errors);
        self.too_recent = self.too_recent.saturating_add(other.too_recent);
    }
}

#[derive(Clone)]
struct Target {
    category: Category,
    roots: Vec<PathBuf>,
    selected: bool,
    found: Stats,
    blocked: bool,
}
impl Target {
    fn new(category: Category, roots: Vec<PathBuf>) -> Self {
        Self { category, roots, selected: true, found: Stats::default(), blocked: false }
    }
}

#[derive(Clone)]
struct CleanupReport {
    per_category: Vec<(Category, Stats)>,
    total: Stats,
    edge_skipped: bool,
}

enum Job { Analyze, Clean }
enum WorkerMessage {
    Progress { done: usize, total: usize, label: String },
    Finished { targets: Vec<Target>, report: Option<CleanupReport>, warnings: u64 },
}

fn env_dir(key: &str) -> Option<PathBuf> {
    env::var_os(key).map(PathBuf::from).filter(|p| p.is_absolute())
}
fn is_safe_directory(path: &Path) -> bool {
    // Verifica toda a cadeia: nao segue links simbolicos/junctions mesmo na raiz.
    for ancestor in path.ancestors() {
        let Ok(meta) = fs::symlink_metadata(ancestor) else { return false; };
        if !meta.is_dir() || meta.file_type().is_symlink() || is_reparse(&meta) { return false; }
    }
    true
}
#[cfg(windows)]
fn is_reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool { false }

fn edge_profiles() -> (Option<PathBuf>, Vec<PathBuf>) {
    let Some(base) = env_dir("LOCALAPPDATA").map(|p| p.join("Microsoft/Edge/User Data")) else {
        return (None, vec![]);
    };
    let mut profiles = Vec::new();
    if is_safe_directory(&base) {
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let profile_number = name.strip_prefix("Profile ").map(|v| {
                    !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
                }).unwrap_or(false);
                if name == "Default" || name == "Guest Profile" || profile_number {
                    let path = entry.path();
                    if is_safe_directory(&path) { profiles.push(path); }
                }
            }
        }
    }
    profiles.sort();
    (Some(base), profiles)
}
fn build_targets() -> Vec<Target> {
    let local = env_dir("LOCALAPPDATA");
    let windir = env_dir("WINDIR");
    let programdata = env_dir("PROGRAMDATA");
    let (edge_base, profiles) = edge_profiles();
    CATEGORIES.iter().copied().map(|category| {
        let mut roots = Vec::new();
        match category {
            Category::UserTemp => {
                // Usar o local padrao, e nao um TEMP arbitrario, para proteger arquivos pessoais.
                if let Some(local) = &local { roots.push(local.join("Temp")); }
            }
            Category::WindowsTemp => {
                if let Some(windir) = &windir { roots.push(windir.join("Temp")); }
            }
            Category::DirectX => {
                if let Some(local) = &local { roots.push(local.join("D3DSCache")); }
            }
            Category::Nvidia => {
                if let Some(local) = &local {
                    roots.push(local.join("NVIDIA/DXCache"));
                    roots.push(local.join("NVIDIA/GLCache"));
                }
                if let Some(data) = &programdata {
                    roots.push(data.join("NVIDIA Corporation/NV_Cache"));
                }
            }
            Category::Thumbnails | Category::IconCache => {
                if let Some(local) = &local {
                    roots.push(local.join("Microsoft/Windows/Explorer"));
                }
            }
            Category::EdgeWeb => for p in &profiles {
                roots.push(p.join("Network/Cache"));
                roots.push(p.join("Cache"));
            },
            Category::EdgeCode => for p in &profiles {
                roots.push(p.join("Code Cache"));
            },
            Category::EdgeGraphics => {
                for p in &profiles {
                    for sub in ["GPUCache", "DawnCache", "GrShaderCache", "ShaderCache"] {
                        roots.push(p.join(sub));
                    }
                }
                if let Some(base) = &edge_base {
                    roots.push(base.join("ShaderCache"));
                    roots.push(base.join("GrShaderCache"));
                }
            }
            Category::EdgeMedia => for p in &profiles {
                roots.push(p.join("Media Cache"));
            },
        }
        Target::new(category, roots)
    }).collect()
}

#[cfg(windows)]
fn edge_running() -> bool {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("tasklist");
    command.args(["/FI", "IMAGENAME eq msedge.exe", "/FO", "CSV", "/NH"]);
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command.output().map(|out| {
        if !out.status.success() { return true; }
        String::from_utf8_lossy(&out.stdout).to_ascii_lowercase().contains("\"msedge.exe\"")
    }).unwrap_or(true) // erro de deteccao: por precaucao, bloquear Edge
}
#[cfg(not(windows))]
fn edge_running() -> bool { false }

fn eligible_age(metadata: &fs::Metadata, age: Duration) -> bool {
    if age == Duration::ZERO { return true; }
    metadata.modified().ok()
        .and_then(|when| SystemTime::now().duration_since(when).ok())
        .map(|elapsed| elapsed >= age)
        .unwrap_or(false)
}

fn visit(root: &Path, category: Category, delete: bool, stats: &mut Stats) {
    if !root.exists() { return; }
    if !is_safe_directory(root) { stats.errors += 1; return; }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => { stats.errors += 1; continue; }
        };
        for item in entries {
            let item = match item { Ok(i) => i, Err(_) => { stats.errors += 1; continue; } };
            let path = item.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => { stats.errors += 1; continue; }
            };
            if metadata.file_type().is_symlink() || is_reparse(&metadata) { continue; }
            if metadata.is_dir() {
                if !category.restrict_to_root() { stack.push(path); }
            } else if metadata.is_file() && category.matches_file(&path) {
                if !eligible_age(&metadata, category.min_age()) {
                    stats.too_recent += 1;
                    continue;
                }
                if delete {
                    match fs::remove_file(&path) {
                        Ok(()) => {
                            stats.files += 1;
                            stats.bytes = stats.bytes.saturating_add(metadata.len());
                        }
                        Err(_) => stats.errors += 1,
                    }
                } else {
                    stats.files += 1;
                    stats.bytes = stats.bytes.saturating_add(metadata.len());
                }
            }
        }
    }
}

fn execute(mut targets: Vec<Target>, job: Job, tx: Sender<WorkerMessage>) {
    let clean = matches!(job, Job::Clean);
    let mut report = CleanupReport {
        per_category: Vec::new(), total: Stats::default(), edge_skipped: false,
    };
    if clean {
        for (i, target) in targets.iter().enumerate() {
            if !target.selected { continue; }
            let _ = tx.send(WorkerMessage::Progress {
                done: i, total: CATEGORIES.len() * 2,
                label: format!("Limpando: {}", target.category.title()),
            });
            if target.category.is_edge() && edge_running() {
                report.edge_skipped = true;
                continue;
            }
            let mut item_stats = Stats::default();
            for root in &target.roots {
                // Conferir novamente, para nao limpar caminhos alterados apos a analise.
                visit(root, target.category, true, &mut item_stats);
            }
            report.total.accumulate(&item_stats);
            report.per_category.push((target.category, item_stats));
        }
    }
    let mut warnings = 0;
    let blocked = edge_running();
    for (i, target) in targets.iter_mut().enumerate() {
        let _ = tx.send(WorkerMessage::Progress {
            done: if clean { CATEGORIES.len() + i } else { i },
            total: if clean { CATEGORIES.len() * 2 } else { CATEGORIES.len() },
            label: format!("Analisando: {}", target.category.title()),
        });
        target.blocked = target.category.is_edge() && blocked;
        let mut stats = Stats::default();
        if !target.blocked {
            for root in &target.roots { visit(root, target.category, false, &mut stats); }
        }
        warnings += stats.errors;
        target.found = stats;
    }
    let _ = tx.send(WorkerMessage::Finished {
        targets, report: if clean { Some(report) } else { None }, warnings,
    });
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut i = 0;
    while value >= 1024.0 && i < UNITS.len() - 1 { value /= 1024.0; i += 1; }
    if i == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[i]) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page { Overview, Report, About }

struct CleanerApp {
    targets: Vec<Target>,
    rx: Option<Receiver<WorkerMessage>>,
    busy: bool,
    analyzed: bool,
    confirm: bool,
    page: Page,
    progress: f32,
    progress_label: String,
    status: String,
    report: Option<CleanupReport>,
}
impl Default for CleanerApp {
    fn default() -> Self {
        Self {
            targets: build_targets(), rx: None, busy: false, analyzed: false,
            confirm: false, page: Page::Overview, progress: 0.0,
            progress_label: "".into(), status: "Pronto para analisar os caches.".into(),
            report: None,
        }
    }
}
impl CleanerApp {
    fn begin(&mut self, job: Job) {
        if self.busy { return; }
        self.busy = true;
        self.confirm = false;
        self.progress = 0.0;
        self.progress_label = if matches!(job, Job::Analyze) { "Analisando arquivos..." } else { "Limpando arquivos..." }.into();
        let mut targets = build_targets();
        for target in &mut targets {
            if let Some(old) = self.targets.iter().find(|t| t.category == target.category) {
                target.selected = old.selected;
            }
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        thread::spawn(move || execute(targets, job, tx));
    }
    fn consume_messages(&mut self) {
        let mut finished = None;
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Progress { done, total, label } => {
                        self.progress = done as f32 / total.max(1) as f32;
                        self.progress_label = label;
                    }
                    WorkerMessage::Finished { targets, report, warnings } => {
                        finished = Some((targets, report, warnings));
                    }
                }
            }
        }
        if let Some((targets, report, warnings)) = finished {
            self.targets = targets;
            self.analyzed = true;
            self.busy = false;
            self.rx = None;
            self.progress = 1.0;
            if let Some(report) = report {
                self.status = format!("Limpeza concluída: {} em {} arquivos. Falhas de acesso: {}.",
                    size(report.total.bytes), report.total.files, report.total.errors);
                self.report = Some(report);
                self.page = Page::Report;
            } else {
                self.status = if warnings > 0 {
                    format!("Análise concluída. {} arquivos ou pastas sem acesso.", warnings)
                } else { "Análise concluída. Escolha as categorias para limpar.".into() };
            }
        }
    }
    fn selected_stats(&self) -> Stats {
        let mut stats = Stats::default();
        for target in self.targets.iter().filter(|t| t.selected && !t.blocked) {
            stats.accumulate(&target.found);
        }
        stats
    }
    fn selected_count(&self) -> usize { self.targets.iter().filter(|t| t.selected).count() }
    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 12.0, BLUE);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "W", FontId::proportional(26.0), Color32::WHITE);
            ui.vertical(|ui| {
                ui.label(RichText::new("WinCache").size(18.0).strong().color(FG));
                ui.label(RichText::new("CLEANER  2.0").size(10.0).strong().color(MUTED));
            });
        });
        ui.add_space(40.0);
        ui.label(RichText::new("MENU").size(10.0).strong().color(MUTED));
        ui.add_space(12.0);
        for (page, label) in [(Page::Overview, "▣   Painel de limpeza"),
                             (Page::Report, "◫   Último relatório"),
                             (Page::About, "ⓘ   Sobre e segurança")] {
            let active = self.page == page;
            let color = if active { FG } else { MUTED };
            let button = egui::Button::new(RichText::new(label).size(14.0).color(color))
                .fill(if active { SURFACE } else { SIDEBAR })
                .rounding(Rounding::same(9.0))
                .stroke(if active { Stroke::new(1.0, BORDER) } else { Stroke::NONE });
            if ui.add_sized([190.0, 40.0], button).clicked() { self.page = page; }
            ui.add_space(5.0);
        }
        ui.with_layout(egui::Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("Windows 10 / 11").size(11.0).color(MUTED));
            ui.label(RichText::new("Sem registro • Sem dados pessoais").size(10.0).color(MUTED));
            ui.add_space(12.0);
            egui::Frame::none().fill(SURFACE).rounding(Rounding::same(10.0))
                .inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                    ui.set_min_width(166.0);
                    ui.label(RichText::new("●  LIMPEZA CONSERVADORA").size(10.0).strong().color(GREEN));
                    ui.add_space(5.0);
                    ui.label(RichText::new("Arquivos pessoais preservados.").size(11.0).color(MUTED));
                });
        });
    }
    fn heading(ui: &mut egui::Ui, title: &str, description: &str) {
        ui.label(RichText::new(title).size(27.0).strong().color(FG));
        ui.add_space(3.0);
        ui.label(RichText::new(description).size(12.0).color(MUTED));
    }
    fn metric(ui: &mut egui::Ui, label: &str, value: &str, hint: &str, emphasis: bool) {
        egui::Frame::none().fill(SURFACE).stroke(Stroke::new(1.0, BORDER))
            .rounding(Rounding::same(12.0)).inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_min_height(88.0);
                ui.label(RichText::new(label).size(11.0).color(MUTED));
                ui.add_space(9.0);
                ui.label(RichText::new(value).size(25.0).strong().color(if emphasis { GREEN } else { FG }));
                ui.label(RichText::new(hint).size(10.0).color(MUTED));
            });
    }
    fn target_card(ui: &mut egui::Ui, target: &mut Target, max_bytes: u64, busy: bool) {
        let active = target.selected;
        egui::Frame::none().fill(SURFACE)
            .stroke(Stroke::new(1.0, if active { BORDER } else { Color32::from_rgb(34, 44, 60) }))
            .rounding(Rounding::same(11.0))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled(!busy, egui::Checkbox::new(&mut target.selected, ""));
                    ui.add_space(3.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(target.category.title()).size(14.0).strong().color(FG));
                        ui.add_space(3.0);
                        ui.label(RichText::new(target.category.description()).size(11.0).color(MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.vertical(|ui| {
                            if target.blocked {
                                ui.label(RichText::new("EDGE ABERTO").size(11.0).strong().color(AMBER));
                            } else {
                                ui.label(RichText::new(size(target.found.bytes)).size(17.0).strong().color(FG));
                            }
                            ui.label(RichText::new(format!("{} arquivos", target.found.files)).size(10.0).color(MUTED));
                        });
                    });
                });
                ui.add_space(9.0);
                let fraction = if max_bytes == 0 { 0.0 } else { target.found.bytes as f32 / max_bytes as f32 };
                ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .fill(if target.category.is_edge() { GREEN } else { BLUE })
                    .desired_width(ui.available_width()));
            });
    }
    fn overview(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Limpeza inteligente", "Analise e remova apenas caches temporários, sem afetar seus dados pessoais.");
        ui.add_space(20.0);
        let stats = self.selected_stats();
        ui.columns(3, |columns| {
            Self::metric(&mut columns[0], "ESPAÇO ESTIMADO", &size(stats.bytes), "Selecionado e acessível", true);
            Self::metric(&mut columns[1], "ARQUIVOS ELEGÍVEIS", &stats.files.to_string(), "Podem estar em uso", false);
            Self::metric(&mut columns[2], "CATEGORIAS ATIVAS", &format!("{} / 10", self.selected_count()), "Windows + Microsoft Edge", false);
        });
        ui.add_space(23.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Categorias de limpeza").size(18.0).strong().color(FG));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(!self.busy, egui::Button::new("Selecionar tudo")).clicked() {
                    for t in &mut self.targets { t.selected = true; }
                }
                if ui.add_enabled(!self.busy, egui::Button::new("Desmarcar tudo")).clicked() {
                    for t in &mut self.targets { t.selected = false; }
                }
            });
        });
        ui.add_space(12.0);
        let max_bytes = self.targets.iter().map(|t| t.found.bytes).max().unwrap_or(0);
        for (section, edge) in [("WINDOWS E SISTEMA", false), ("MICROSOFT EDGE", true)] {
            ui.add_space(6.0);
            ui.label(RichText::new(section).size(10.0).strong().color(if edge { GREEN } else { BLUE }));
            ui.add_space(7.0);
            for target in self.targets.iter_mut().filter(|t| t.category.is_edge() == edge) {
                Self::target_card(ui, target, max_bytes, self.busy);
                ui.add_space(8.0);
            }
        }
        ui.add_space(12.0);
        if self.busy {
            ui.label(RichText::new(&self.progress_label).color(MUTED));
            ui.add(egui::ProgressBar::new(self.progress).desired_width(ui.available_width()).fill(BLUE));
        } else {
            ui.horizontal(|ui| {
                let analyze = egui::Button::new(RichText::new("Analisar caches").strong().color(FG))
                    .fill(Color32::from_rgb(46, 66, 93)).rounding(Rounding::same(9.0))
                    .min_size(egui::vec2(168.0, 42.0));
                if ui.add(analyze).clicked() { self.begin(Job::Analyze); }
                let clean = egui::Button::new(RichText::new("Limpar selecionados").strong().color(Color32::from_rgb(10, 28, 30)))
                    .fill(GREEN).rounding(Rounding::same(9.0))
                    .min_size(egui::vec2(198.0, 42.0));
                if ui.add_enabled(self.analyzed && self.selected_count() > 0 && stats.files > 0, clean).clicked() {
                    self.confirm = true;
                }
            });
        }
        ui.add_space(12.0);
        ui.label(RichText::new(&self.status).size(12.0).color(MUTED));
        ui.add_space(7.0);
        ui.label(RichText::new("Arquivos em uso ou inacessíveis são ignorados. Feche o Edge para permitir sua limpeza.")
            .size(11.0).color(MUTED));
        ui.add_space(15.0);
    }
    fn report_page(&self, ui: &mut egui::Ui) {
        Self::heading(ui, "Relatório da limpeza", "Resultado real dos arquivos removidos na última execução.");
        ui.add_space(22.0);
        let Some(report) = &self.report else {
            egui::Frame::none().fill(SURFACE).rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(20.0)).show(ui, |ui| {
                    ui.label(RichText::new("Nenhuma limpeza realizada nesta sessão.").size(15.0).color(FG));
                    ui.add_space(7.0);
                    ui.label(RichText::new("Vá ao Painel, analise os caches e confirme a exclusão.").color(MUTED));
                });
            return;
        };
        ui.columns(3, |cols| {
            Self::metric(&mut cols[0], "ESPAÇO REMOVIDO", &size(report.total.bytes), "Somente arquivos removidos", true);
            Self::metric(&mut cols[1], "ARQUIVOS REMOVIDOS", &report.total.files.to_string(), "Exclusão confirmada", false);
            Self::metric(&mut cols[2], "FALHAS", &report.total.errors.to_string(), "Sem permissão / em uso", false);
        });
        ui.add_space(18.0);
        if report.edge_skipped {
            ui.colored_label(AMBER, "O Edge estava em execução; as categorias do navegador foram ignoradas.");
            ui.add_space(12.0);
        }
        ui.label(RichText::new("Detalhamento por categoria").size(18.0).strong().color(FG));
        ui.add_space(10.0);
        for (category, stats) in &report.per_category {
            egui::Frame::none().fill(SURFACE).rounding(Rounding::same(9.0))
                .inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(category.title()).size(13.0).color(FG));
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{}  |  {} arquivos  |  {} falhas", size(stats.bytes), stats.files, stats.errors))
                                .size(11.0).color(MUTED));
                        });
                    });
                });
            ui.add_space(7.0);
        }
        ui.add_space(12.0);
        ui.label(RichText::new("Espaço estimado = soma dos tamanhos lógicos dos arquivos; não equivale sempre ao espaço livre no disco.")
            .size(11.0).color(MUTED));
    }
    fn about_page(&self, ui: &mut egui::Ui) {
        Self::heading(ui, "Proteção e funcionamento", "Transparência sobre o que o WinCache Cleaner remove.");
        ui.add_space(22.0);
        for (title, body, color) in [
            ("O que é removido", "Apenas arquivos em diretórios predefinidos de cache ou bancos específicos de miniaturas/ícones. Temporários com menos de 48 horas são preservados; shaders e caches do Explorer, com menos de 24 horas, também.", GREEN),
            ("O que é protegido", "Documentos, Downloads, área de trabalho, Lixeira, senhas, cookies, histórico, favoritos, sessões, banco de dados dos sites, WebCache e Service Workers. Não limpa registro, Windows Update, WinSxS ou Prefetch.", BLUE),
            ("Microsoft Edge", "O navegador precisa estar fechado. O programa não encerra processos e não remove perfis. Aplicativos podem recriar caches após a limpeza. Perfis fora do caminho padrão não são pesquisados.", AMBER),
            ("Diretórios protegidos", "Links simbólicos e junctions não são percorridos. Arquivos sem permissão e bloqueados não são removidos. Não é necessário executar como administrador para caches do usuário.", BLUE),
        ] {
            egui::Frame::none().fill(SURFACE).stroke(Stroke::new(1.0, BORDER))
                .rounding(Rounding::same(11.0)).inner_margin(egui::Margin::same(17.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(title).size(15.0).strong().color(color));
                    ui.add_space(7.0);
                    ui.label(RichText::new(body).size(12.0).color(MUTED));
                });
            ui.add_space(11.0);
        }
        ui.label(RichText::new("WinCache Cleaner 0.2.0 • código-fonte aberto • limpeza sem telemetria")
            .size(11.0).color(MUTED));
    }
    fn confirmation_window(&mut self, ctx: &egui::Context) {
        if !self.confirm { return; }
        let stats = self.selected_stats();
        egui::Window::new("Confirmar limpeza")
            .collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([435.0, 230.0]).show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("Remover ate {} em {} arquivos?", size(stats.bytes), stats.files))
                    .size(18.0).strong().color(FG));
                ui.add_space(9.0);
                ui.label(RichText::new("A exclusão é permanente. Somente os caches das categorias selecionadas serão processados.")
                    .color(MUTED));
                if self.targets.iter().any(|t| t.selected && t.category.is_edge()) && edge_running() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("O Edge está aberto: suas categorias serão ignoradas.").color(AMBER));
                }
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() { self.confirm = false; }
                    let action = egui::Button::new(RichText::new("Confirmar exclusão").strong().color(BG)).fill(GREEN);
                    if ui.add(action).clicked() { self.begin(Job::Clean); }
                });
            });
    }
}

impl eframe::App for CleanerApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.consume_messages();
        if self.busy { ctx.request_repaint_after(Duration::from_millis(75)); }
        egui::SidePanel::left("sidebar").exact_width(224.0).resizable(false)
            .frame(egui::Frame::none().fill(SIDEBAR).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| self.navigation(ui));
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG)
            .inner_margin(egui::Margin::symmetric(28.0, 24.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match self.page {
                        Page::Overview => self.overview(ui),
                        Page::Report => self.report_page(ui),
                        Page::About => self.about_page(ui),
                    }
                });
            });
        self.confirmation_window(ctx);
    }
}

fn configure_style(ctx: &egui::Context) {
    #[cfg(windows)]
    if let Some(font_path) = env_dir("WINDIR").map(|p| p.join("Fonts/segoeui.ttf")) {
        if let Ok(data) = fs::read(font_path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("Segoe UI".into(), egui::FontData::from_owned(data).into());
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "Segoe UI".into());
            }
            ctx.set_fonts(fonts);
        }
    }
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.override_text_color = Some(FG);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(39, 56, 80);
    style.visuals.selection.bg_fill = BLUE;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    ctx.set_style(style);
}
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1010.0, 740.0])
            .with_min_inner_size([780.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native("WinCache Cleaner", options, Box::new(|cc| {
        configure_style(&cc.egui_ctx);
        Ok(Box::<CleanerApp>::default())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explorer_cache_databases_are_eligible() {
        assert!(Category::Thumbnails.matches_file(Path::new("thumbcache_256.db")));
        assert!(Category::IconCache.matches_file(Path::new("iconcache_32.db")));
        assert!(!Category::Thumbnails.matches_file(Path::new("History.db")));
        assert!(!Category::IconCache.matches_file(Path::new("thumbcache_32.db")));
    }
    #[test]
    fn safe_size_formatting() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1024), "1.0 KB");
    }
    #[test]
    fn cache_policy_never_includes_service_worker_paths() {
        let all = build_targets();
        assert_eq!(all.len(), CATEGORIES.len());
        assert!(all.iter().flat_map(|t| &t.roots).all(|p| {
            let s = p.to_string_lossy().to_ascii_lowercase();
            !s.contains("service worker") && !s.contains("webcache")
                && !s.contains("downloads") && !s.contains("cookies")
        }));
    }
}
