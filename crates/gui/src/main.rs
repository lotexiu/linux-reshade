//! `lrpanel`: a interface gráfica do linux-reshade.
//!
//! Uso normal: `lrpanel`. Para testes automáticos: `--smoke <Efeito>` aciona os widgets e confere o
//! resultado (sai com código diferente de 0 se algo falhar); `--effect <Efeito>` o adiciona ao preset do jogo (e o tira ao sair);
//! `--screenshot <arquivo.png>` grava a janela; `--hold <ms>` a mantém aberta antes de sair; `--wait <ms>` espera antes de agir.

mod add_effect;
mod controls;
mod depth_picker;
mod disclosure;
mod flatpak_view;
mod effect_row;
mod games;
mod macros;
mod rx;
mod settings;
mod shortcuts_dialog;
mod window;

static GTK_WARNINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Quantos avisos ou erros o GTK emitiu desde o início (menos os de linha de base de `GtkImage`, que são conhecidos e inofensivos).
pub(crate) fn gtk_warnings() -> usize {
    GTK_WARNINGS.load(std::sync::atomic::Ordering::Relaxed)
}

fn count_gtk_warnings() {
    glib::log_set_writer_func(|level, fields| {
        let message = fields.iter().find(|f| f.key() == "MESSAGE").and_then(|f| f.value_str()).unwrap_or("");
        if matches!(level, glib::LogLevel::Warning | glib::LogLevel::Critical | glib::LogLevel::Error) && !message.contains("baselines") {
            GTK_WARNINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        glib::log_writer_default(level, fields)
    });
}

use control_core::compiler::Compiler;
use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use control_core::i18n::fmt;

#[derive(Default)]
struct Options {
    smoke: Option<String>,
    screenshot: Option<String>,
    /// Escolhe este jogo antes de agir (para fotografar a tela dele).
    game: Option<String>,
    /// Abre o editor de definições (para fotografar a tela com ele aberto).
    open_macros: bool,
    open_shortcuts: bool,
    /// Abre e fecha os menus da janela como uma pessoa e confere que nada some.
    smoke_popovers: bool,
    /// Abre esta página da janela (`add`, `flatpak`) antes de fotografar.
    page: Option<String>,
    /// Espera antes de agir (para um jogo abrir depois do painel).
    wait_ms: u64,
    /// Escolhe este efeito na lista (e desfaz ao sair).
    effect: Option<String>,
    /// Mantém a janela aberta por este tempo antes de sair, para outro processo conferir o estado.
    hold_ms: u64,
    /// Confere que a janela continua viva sozinha, sem que o teste a segure (o que acontece no uso normal).
    keepalive_check: bool,
}

impl Options {
    fn is_test_run(&self) -> bool {
        self.game.is_some() || self.smoke.is_some() || self.open_macros || self.open_shortcuts || self.smoke_popovers || self.page.is_some() || self.screenshot.is_some() || self.effect.is_some() || self.hold_ms > 0 || self.wait_ms > 0 || self.keepalive_check
    }
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke" => options.smoke = Some(args.next().ok_or("--smoke precisa do nome de um efeito")?),
            "--effect" => options.effect = Some(args.next().ok_or("--effect precisa do nome de um efeito")?),
            "--wait" => options.wait_ms = args.next().and_then(|v| v.parse().ok()).ok_or("--wait precisa de um tempo em milissegundos")?,
            "--hold" => options.hold_ms = args.next().and_then(|v| v.parse().ok()).ok_or("--hold precisa de um tempo em milissegundos")?,
            "--game" => options.game = Some(args.next().ok_or("--game precisa do id de um jogo")?),
            "--open-macros" => options.open_macros = true,
            "--open-shortcuts" => options.open_shortcuts = true,
            "--smoke-popovers" => options.smoke_popovers = true,
            "--page" => options.page = Some(args.next().ok_or("--page precisa do nome de uma página (add, flatpak)")?),
            "--keepalive-check" => options.keepalive_check = true,
            "--screenshot" => options.screenshot = Some(args.next().ok_or("--screenshot precisa de um arquivo .png")?),
            other => return Err(format!("opção desconhecida: {other}\nuso: lrpanel [--smoke <Efeito>] [--effect <Efeito>] [--screenshot <arquivo.png>] [--wait <ms>] [--hold <ms>]")),
        }
    }
    Ok(options)
}

/// Grava a janela, já desenhada, num PNG.
fn save_screenshot(window: &impl IsA<gtk::Widget>, path: &str) -> Result<(), String> {
    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node().ok_or("a janela não desenhou nada")?;
    let renderer = window.native().and_then(|n| n.renderer()).ok_or("a janela não tem renderizador")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

/// Tema escuro próprio: o do sistema pode deixar o texto de erro ilegível (letras brancas sobre fundo claro). Força o esquema
/// escuro do GTK e fixa as cores dos estados.
fn apply_dark_theme() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        ".error { color: #ff7b72; }\n.success { color: #7ee787; }\n.warning { color: #e3b341; }\n.dim-label { opacity: 0.7; }\ntooltip { background: #2b2d33; color: #e6e6e6; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
    }
}

fn main() -> glib::ExitCode {
    count_gtk_warnings();
    let options = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lrpanel: {e}");
            return glib::ExitCode::from(2);
        }
    };
    let state = match ipc::Writer::create() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lrpanel: {}", fmt("could not open {}: {}", &[&ipc::state_path().display(), &e]));
            return glib::ExitCode::FAILURE;
        }
    };

    // Cada janela é independente: abrir de novo não reaproveita a anterior (e os testes não esbarram na do usuário).
    let app = gtk::Application::builder().application_id("dev.lotexiu.LinuxReshade").flags(gtk::gio::ApplicationFlags::NON_UNIQUE).build();
    let exit = Rc::new(RefCell::new(glib::ExitCode::SUCCESS));
    let state = RefCell::new(Some(state));
    app.connect_activate({
        let exit = exit.clone();
        let options = Rc::new(options);
        move |app| {
            let Some(state) = state.borrow_mut().take() else { return };
            apply_dark_theme();
            // Analisa os efeitos em segundo plano (compila cada um uma vez): a lista de adicionar mostra os incompatíveis.
            let _ = panel::start_analysis();
            // Quem abre o painel não precisa abrir um terminal: no AppImage a camada do Vulkan é instalada para o usuário (a biblioteca vai
            // para uma pasta fixa, pois o AppImage monta noutra a cada execução) e o daemon é iniciado se não há um rodando.
            if !options.is_test_run() {
                if std::env::var_os("APPIMAGE").is_some() {
                    let _ = control_core::install::first_run();
                    if !control_core::install::layer_installed() {
                        let _ = control_core::install::install_layer();
                    }
                }
                control_core::install::start_daemon_if_needed();
            }
            let home = panel::Home::new(Compiler::from_env(), config::Config::load().presets_dir, state);
            let ui = window::Ui::new(app, home);
            ui.window.present();
            // Os botões e o relógio da tela guardam só referências fracas da interface (para não formarem um ciclo):
            // alguém precisa segurá-la viva enquanto o programa roda. Sem isto ela some ao fim desta função e
            // nada responde. O teste `--keepalive-check` confere.
            let keep = ui.clone();
            app.connect_shutdown(move |_| {
                let _ = &keep;
            });
            if options.keepalive_check {
                glib::MainContext::default().spawn_local(keepalive_check(app.clone(), Rc::downgrade(&ui), exit.clone()));
                drop(ui);
            } else if options.is_test_run() {
                glib::MainContext::default().spawn_local(test_run(app.clone(), ui, options.clone(), exit.clone()));
            }
        }
    });
    // Os argumentos próprios já foram lidos; o GTK não deve tentar interpretá-los.
    app.run_with_args::<&str>(&[]);
    let code = *exit.borrow();
    code
}

/// Sem ninguém segurando a interface além do próprio programa, ela ainda existe e o relógio da tela segue rodando.
async fn keepalive_check(app: gtk::Application, ui: std::rc::Weak<window::Ui>, exit: Rc<RefCell<glib::ExitCode>>) {
    glib::timeout_future(Duration::from_millis(1500)).await;
    match ui.upgrade() {
        Some(_) => println!("lrpanel keepalive: ok"),
        None => {
            eprintln!("lrpanel: keepalive FALHOU: a interface foi destruída; os botões e a atualização não funcionariam");
            *exit.borrow_mut() = glib::ExitCode::FAILURE;
        }
    }
    app.quit();
}

/// O roteiro dos modos de teste: espera a janela ser desenhada, age como um usuário e sai deixando tudo como estava.
async fn test_run(app: gtk::Application, ui: Rc<window::Ui>, options: Rc<Options>, exit: Rc<RefCell<glib::ExitCode>>) {
    let fail = |why: String| {
        eprintln!("lrpanel: {why}");
        *exit.borrow_mut() = glib::ExitCode::FAILURE;
    };
    glib::timeout_future(Duration::from_millis(600 + options.wait_ms)).await;

    if let Some(game) = &options.game {
        ui.select_game(game);
        glib::timeout_future(Duration::from_millis(700)).await;
    }
    if let Some(name) = &options.smoke {
        let problems = ui.smoke(name);
        if problems.is_empty() {
            println!("lrpanel smoke: ok");
        }
        for p in problems {
            fail(format!("smoke FALHOU: {p}"));
        }
    }
    if options.smoke_popovers {
        let problems = ui.smoke_popovers().await;
        if problems.is_empty() {
            println!("lrpanel smoke-popovers: ok");
        }
        for p in problems {
            fail(format!("smoke-popovers FALHOU: {p}"));
        }
    }
    if let Some(name) = &options.effect {
        if ui.choose_effect(name) {
            glib::timeout_future(Duration::from_millis(600)).await; // dá tempo de compilar e de a layer responder
        } else {
            fail(format!("o efeito {name:?} não está na lista"));
        }
    }
    if let Some(page) = &options.page {
        ui.show_page(page);
        glib::timeout_future(Duration::from_millis(2500)).await;
    }
    if options.open_macros {
        ui.macros_popup();
        glib::timeout_future(Duration::from_millis(500)).await;
    }
    let dialog = options.open_shortcuts.then(|| shortcuts_dialog::open(&ui.window));
    if dialog.is_some() {
        glib::timeout_future(Duration::from_millis(500)).await;
    }
    if let Some(path) = &options.screenshot {
        let shot = match &dialog {
            Some(d) => save_screenshot(d, path),
            None => save_screenshot(&ui.window, path),
        };
        match shot {
            Ok(()) => println!("lrpanel: captura em {path}"),
            Err(e) => fail(format!("captura falhou: {e}")),
        }
    }
    if options.hold_ms > 0 {
        println!("lrpanel: janela aberta por {} ms", options.hold_ms);
        glib::timeout_future(Duration::from_millis(options.hold_ms)).await;
    }
    println!("lrpanel: {}", ui.describe_games());
    if let Some(name) = &options.effect {
        ui.remove_effect(name); // deixa o preset como estava
    }
    app.quit();
}
