// Ferramenta de desenvolvimento: no Wayland o cliente não escolhe a própria posição, então o
// vkcube de teste é posicionado pelo compositor. Monitor: o mais à esquerda. Canto: inferior esquerdo.
//
// ATENÇÃO: código que roda DENTRO do compositor. Um erro nativo aqui derruba o GNOME Shell e, no
// Wayland, a sessão inteira (aconteceu em 2026-10-09: SIGSEGV em move_frame). Por isso:
//  - só mexe em janela normal, ainda gerenciada, mapeada e com tamanho conhecido;
//  - cancela o trabalho pendente se a janela for destruída antes (sinal `unmanaging`);
//  - tudo dentro de try/catch (não pega falha nativa, mas evita exceção JS solta).
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

const MATCH = 'vkcube';
const DELAY_MS = 300; // título/app_id e a superfície ainda podem não existir na criação

export default class VkcubePlacement extends Extension {
    enable() {
        this._pending = new Map(); // janela -> {source, unmanaging}
        this._created = global.display.connect('window-created', (_display, win) => this._track(win));
    }

    disable() {
        global.display.disconnect(this._created);
        for (const win of [...this._pending.keys()])
            this._forget(win);
        this._pending = null;
    }

    _track(win) {
        if (this._pending.has(win))
            return;
        const entry = {source: 0, unmanaging: 0};
        entry.unmanaging = win.connect('unmanaging', () => this._forget(win));
        entry.source = GLib.timeout_add(GLib.PRIORITY_DEFAULT, DELAY_MS, () => {
            entry.source = 0; // já disparou: não remover de novo
            try {
                this._place(win);
            } catch (e) {
                logError(e, 'vkcube-placement');
            }
            this._forget(win);
            return GLib.SOURCE_REMOVE;
        });
        this._pending.set(win, entry);
    }

    _forget(win) {
        const entry = this._pending?.get(win);
        if (!entry)
            return;
        this._pending.delete(win);
        if (entry.source)
            GLib.source_remove(entry.source);
        try {
            win.disconnect(entry.unmanaging);
        } catch (_e) {
            // a janela já foi destruída: nada a desconectar
        }
    }

    _placeable(win) {
        return win.get_display() !== null &&
            win.get_window_type() === Meta.WindowType.NORMAL &&
            win.get_compositor_private() !== null &&
            !win.minimized &&
            win.get_frame_rect().height > 0;
    }

    _place(win) {
        if (!this._placeable(win))
            return;
        const text = `${win.get_wm_class()} ${win.get_title()}`.toLowerCase();
        if (!text.includes(MATCH))
            return;
        const monitors = Main.layoutManager.monitors;
        if (!monitors.length)
            return;
        const left = monitors.reduce((a, b) => (b.x < a.x ? b : a));
        const work = Main.layoutManager.getWorkAreaForMonitor(left.index); // respeita o dock
        const frame = win.get_frame_rect();
        win.move_frame(true, work.x, work.y + work.height - frame.height);
    }
}
