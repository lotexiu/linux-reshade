//! O que o runtime dá aos efeitos a cada quadro (tempo, data, estatísticas de FPS, "processando") e o controle que a pessoa edita no painel.

use super::*;

/// Valores do frame que o runtime fornece aos uniforms com `source`.
pub(super) fn frame_info(now: Instant, previous: Option<Instant>, frame_count: u64) -> FrameInfo {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let frame_time_ms = previous.map(|p| now.duration_since(p).as_secs_f32() * 1000.0).unwrap_or(0.0);
    let [fps_avg, fps_min, fps_max, fps_low1] = fps_stats(frame_time_ms);
    let busy = if busy_now() { 1.0 } else { 0.0 };
    FrameInfo { time_ms: now.duration_since(*START).as_secs_f32() * 1000.0, frame_time_ms, frame_count: frame_count as u32, date: utc_date(unix), fps_avg, fps_min, fps_max, fps_low1, busy }
}

/// Algo está sendo processado para este jogo: o vigia compila efeitos (marcador na pasta de estado) ou a layer monta uma cadeia. A pasta é lida de
/// quando em quando (o present roda centenas de vezes por segundo).
pub(super) fn busy_now() -> bool {
    static CACHE: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);
    if CHAIN_BUILDING.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, value)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_millis(250)
    {
        return *value;
    }
    let value = ipc::busy_now();
    *cache = Some((Instant::now(), value));
    value
}

/// Média, mínimo, máximo e "1% low" de quadros por segundo na janela dos últimos 5 s: `[avg, min, max, low1]`; zeros enquanto não há dados.
pub(super) fn fps_stats(frame_time_ms: f32) -> [f32; 4] {
    static WINDOW: std::sync::Mutex<Option<FrameWindow>> = std::sync::Mutex::new(None);
    let mut guard = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    let window = guard.get_or_insert_with(|| FrameWindow { times: std::collections::VecDeque::new(), total_ms: 0.0, stats: [0.0; 4], since: 0 });
    // Pausas enormes (a tela do jogo carregando, o jogo parado por um instante) não fazem parte do desempenho: um quadro além de 1 s vale 1 s.
    if frame_time_ms > 0.0 {
        let ms = frame_time_ms.min(1000.0);
        window.times.push_back(ms);
        window.total_ms += ms;
        while window.total_ms > FPS_WINDOW_MS && window.times.len() > 1 {
            window.total_ms -= window.times.pop_front().unwrap_or(0.0);
        }
        window.since += 1;
        if window.since >= FPS_REFRESH_FRAMES || window.stats[0] == 0.0 {
            window.since = 0;
            window.stats = fps_summary(window.times.iter().copied());
        }
    }
    window.stats
}

/// `[média, mínimo, máximo, 1% low]` de quadros por segundo de uma lista de tempos de quadro (ms).
pub(super) fn fps_summary(times: impl Iterator<Item = f32>) -> [f32; 4] {
    let mut sorted: Vec<f32> = times.filter(|t| *t > 0.0).collect();
    if sorted.is_empty() {
        return [0.0; 4];
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    let mean_ms = sorted.iter().sum::<f32>() / n as f32;
    let at = |q: f32| sorted[(((n - 1) as f32) * q).round() as usize];
    // O "1% low" é a taxa do quadro no percentil 99 dos tempos (os 1% mais lentos começam aí).
    [1000.0 / mean_ms, 1000.0 / sorted[n - 1], 1000.0 / sorted[0], 1000.0 / at(0.99)]
}

#[cfg(test)]
mod fps_tests {
    use super::fps_summary;

    #[test]
    pub(super) fn the_summary_reports_average_worst_best_and_one_percent_low() {
        // 99 quadros de 10 ms (100 fps) e um de 50 ms (20 fps).
        let times = std::iter::repeat_n(10.0, 99).chain(std::iter::once(50.0));
        let [avg, min, max, low1] = fps_summary(times);
        assert!((avg - 1000.0 / 10.4).abs() < 0.01, "média: {avg}");
        assert_eq!((min, max), (20.0, 100.0));
        assert!((low1 - 100.0).abs() < 0.01 || (low1 - 20.0).abs() < 0.01, "1% low: {low1}");
        assert_eq!(fps_summary(std::iter::empty()), [0.0; 4]);
    }
}

/// O controle que a pessoa edita no painel (ver `ipc::overlay`), relido a cada 80 ms: o arquivo é pequeno, mas o present roda
/// centenas de vezes por segundo.
pub(super) fn active_control() -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(Instant, Option<String>)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, value)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_millis(80)
    {
        return value.clone();
    }
    let value = ipc::overlay(&crate::status::game_id(), std::time::Duration::from_millis(2500)).map(|(_, uniform)| uniform);
    *cache = Some((Instant::now(), value.clone()));
    value
}
