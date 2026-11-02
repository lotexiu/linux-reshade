//! Medição de desempenho da própria layer, ligada só quando alguém pede (`ipc::stats_on`): quanto ela gasta no processador por
//! quadro (efeitos, profundidade, ganchos de desenho) e quanto a GPU gasta em cada efeito. O resultado vai no status e aparece no
//! painel (aba Desempenho) e em `lrctl layers`.

use ipc::PerfStats;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_CHECK_MS: AtomicU64 = AtomicU64::new(0);

static FRAMES: AtomicU64 = AtomicU64::new(0);
static LAYER_NS: AtomicU64 = AtomicU64::new(0);
static EFFECTS_NS: AtomicU64 = AtomicU64::new(0);
static DEPTH_NS: AtomicU64 = AtomicU64::new(0);
static HOOKS_NS: AtomicU64 = AtomicU64::new(0);
static HOOK_CALLS: AtomicU64 = AtomicU64::new(0);

/// Quantos desenhos separam duas medições dos ganchos (medir cada um custaria mais que o que se quer medir).
pub const HOOK_SAMPLE_EVERY: u64 = 64;

struct Window {
    started: Instant,
    /// Tempo da GPU por etapa (microssegundos, suavizado), na ordem da cadeia.
    gpu: Vec<(String, f32)>,
}

static WINDOW: Mutex<Option<Window>> = Mutex::new(None);

fn clock_ms() -> u64 {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    START.elapsed().as_millis() as u64
}

/// A medição está ligada? Olha o marcador no máximo a cada meio segundo.
pub fn enabled() -> bool {
    let now = clock_ms();
    if now.saturating_sub(LAST_CHECK_MS.load(Ordering::Relaxed)) >= 500 {
        LAST_CHECK_MS.store(now, Ordering::Relaxed);
        let on = ipc::stats_on();
        if ENABLED.swap(on, Ordering::Relaxed) != on {
            if !on {
                crate::status::set_perf(None);
            }
            *WINDOW.lock().unwrap() = None;
            for counter in [&FRAMES, &LAYER_NS, &EFFECTS_NS, &DEPTH_NS, &HOOKS_NS, &HOOK_CALLS] {
                counter.store(0, Ordering::Relaxed);
            }
        }
    }
    ENABLED.load(Ordering::Relaxed)
}

/// Um gancho de desenho mediu `elapsed`; vale por `HOOK_SAMPLE_EVERY` chamadas.
pub fn hook_sampled(elapsed: Duration) {
    HOOKS_NS.fetch_add(elapsed.as_nanos() as u64 * HOOK_SAMPLE_EVERY, Ordering::Relaxed);
}

/// Conta uma chamada de gancho e diz se esta deve ser medida.
pub fn hook_should_sample() -> bool {
    ENABLED.load(Ordering::Relaxed) && HOOK_CALLS.fetch_add(1, Ordering::Relaxed) % HOOK_SAMPLE_EVERY == 0
}

/// O tempo da GPU de cada etapa da cadeia (nome, microssegundos), já suavizado por quem mede.
pub fn set_gpu(steps: Vec<(String, f32)>) {
    if let Some(window) = WINDOW.lock().unwrap().as_mut() {
        window.gpu = steps;
    }
}

/// Fim de um present: o que a layer gastou nele (tudo menos o present do driver), dividido entre efeitos e profundidade.
pub fn frame_done(layer: Duration, effects: Duration, depth: Duration) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    LAYER_NS.fetch_add(layer.as_nanos() as u64, Ordering::Relaxed);
    EFFECTS_NS.fetch_add(effects.as_nanos() as u64, Ordering::Relaxed);
    DEPTH_NS.fetch_add(depth.as_nanos() as u64, Ordering::Relaxed);
    let perf = {
        let mut guard = WINDOW.lock().unwrap();
        let window = guard.get_or_insert_with(|| Window { started: Instant::now(), gpu: Vec::new() });
        let elapsed = window.started.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let frames = FRAMES.swap(0, Ordering::Relaxed).max(1);
        let per_frame = |counter: &AtomicU64| (counter.swap(0, Ordering::Relaxed) / frames / 1000) as u32;
        let seconds = elapsed.as_secs_f64();
        let gpu_steps: Vec<(String, u32)> = window.gpu.iter().map(|(n, us)| (n.clone(), *us as u32)).collect();
        window.started = Instant::now();
        PerfStats {
            fps_centi: (frames as f64 / seconds * 100.0) as u32,
            frame_us: (seconds * 1e6 / frames as f64) as u32,
            layer_us: per_frame(&LAYER_NS),
            effects_us: per_frame(&EFFECTS_NS),
            depth_us: per_frame(&DEPTH_NS),
            hooks_us: per_frame(&HOOKS_NS),
            gpu_us: gpu_steps.iter().map(|(_, us)| *us).sum(),
            gpu_steps,
        }
    };
    crate::status::set_perf(Some(perf));
}
