// Ferramenta de desenvolvimento: no Wayland o cliente não escolhe a própria posição, então o
// vkcube de teste é posicionado pelo compositor. Monitor: o mais à esquerda. Canto: inferior esquerdo.
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import GLib from 'gi://GLib';

const MATCH = 'vkcube';
const DELAY_MS = 200; // título/app_id ainda podem não estar definidos quando a janela é criada

export default class VkcubePlacement extends Extension {
    enable() {
        this._sources = new Set();
        this._created = global.display.connect('window-created', (_display, win) => {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, DELAY_MS, () => {
                this._sources.delete(id);
                this._place(win);
                return GLib.SOURCE_REMOVE;
            });
            this._sources.add(id);
        });
    }

    disable() {
        global.display.disconnect(this._created);
        for (const id of this._sources)
            GLib.source_remove(id);
        this._sources = null;
    }

    _matches(win) {
        const text = `${win.get_wm_class()} ${win.get_title()}`.toLowerCase();
        return text.includes(MATCH);
    }

    _place(win) {
        if (!this._matches(win))
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
