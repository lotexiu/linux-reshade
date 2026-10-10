//! `lrpanel`: a interface gráfica do linux-reshade.
//!
//! Uso normal: `lrpanel`. Para testes automáticos: `--smoke <Efeito>` aciona os widgets e confere o
//! resultado (sai com código diferente de 0 se algo falhar); `--effect <Efeito>` o adiciona ao preset do jogo (e o tira ao sair);
//! `--screenshot <arquivo.png>` grava a janela; `--hold <ms>` a mantém aberta antes de sair; `--wait <ms>` espera antes de agir.

mod controls;
mod games;
mod window;

use control_core::compiler::Compiler;
use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Default)]
struct Options {
    smoke: Option<String>,
    screenshot: Option<String>,
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
        self.smoke.is_some() || self.screenshot.is_some() || self.effect.is_some() || self.hold_ms > 0 || self.wait_ms > 0 || self.keepalive_check
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
            "--keepalive-check" => options.keepalive_check = true,
            "--screenshot" => options.screenshot = Some(args.next().ok_or("--screenshot precisa de um arquivo .png")?),
            other => return Err(format!("opção desconhecida: {other}\nuso: lrpanel [--smoke <Efeito>] [--effect <Efeito>] [--screenshot <arquivo.png>] [--wait <ms>] [--hold <ms>]")),
        }
    }
    Ok(options)
}

/// Grava a janela, já desenhada, num PNG.
fn save_screenshot(window: &gtk::ApplicationWindow, path: &str) -> Result<(), String> {
    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node().ok_or("a janela não desenhou nada")?;
    let renderer = window.native().and_then(|n| n.renderer()).ok_or("a janela não tem renderizador")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn main() -> glib::ExitCode {
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
            eprintln!("lrpanel: não consegui abrir {}: {e}", ipc::state_path().display());
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

    if let Some(name) = &options.smoke {
        let problems = ui.smoke(name);
        if problems.is_empty() {
            println!("lrpanel smoke: ok");
        }
        for p in problems {
            fail(format!("smoke FALHOU: {p}"));
        }
    }
    if let Some(name) = &options.effect {
        if ui.choose_effect(name) {
            glib::timeout_future(Duration::from_millis(600)).await; // dá tempo de compilar e de a layer responder
        } else {
            fail(format!("o efeito {name:?} não está na lista"));
        }
    }
    if let Some(path) = &options.screenshot {
        match save_screenshot(&ui.window, path) {
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
