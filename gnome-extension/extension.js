import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const APP_ID = 'io.github.LuizFer1.Tex';
const APP_PATH = '/io/github/LuizFer1/Tex';
const SHELL_NAME = 'io.github.LuizFer1.Tex.Shell';
const SHELL_PATH = '/io/github/LuizFer1/Tex/Shell';
const PRESENT_WAIT_SECONDS = 2;

const APP_XML = `<node>
  <interface name="io.github.LuizFer1.Tex">
    <method name="Toggle"/>
    <method name="History"/>
    <method name="Settings"/>
    <method name="About"/>
    <method name="TogglePause"/>
    <method name="ApplyUpdate"/>
    <method name="Quit"/>
    <property name="Paused" type="b" access="read"/>
    <property name="UpdateReady" type="s" access="read"/>
    <property name="Labels" type="as" access="read"/>
  </interface>
</node>`;

const SHELL_XML = `<node>
  <interface name="io.github.LuizFer1.Tex.Shell">
    <method name="WorkAreaUnderCursor">
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
      <arg type="u" direction="out" name="width"/>
      <arg type="u" direction="out" name="height"/>
      <arg type="d" direction="out" name="scale"/>
    </method>
    <method name="PrimaryWorkArea">
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
      <arg type="u" direction="out" name="width"/>
      <arg type="u" direction="out" name="height"/>
      <arg type="d" direction="out" name="scale"/>
    </method>
    <method name="Present">
      <arg type="s" direction="in" name="title"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
    </method>
    <method name="SetShortcuts">
      <arg type="s" direction="in" name="tex"/>
      <arg type="s" direction="in" name="history"/>
      <arg type="b" direction="out" name="tex_grabbed"/>
      <arg type="b" direction="out" name="history_grabbed"/>
    </method>
    <signal name="ShortcutActivated">
      <arg type="s" name="id"/>
    </signal>
    <method name="Deliver">
      <arg type="u" direction="in" name="serial"/>
      <arg type="s" direction="in" name="mime"/>
      <arg type="ay" direction="out" name="data"/>
    </method>
    <method name="SetClipboard">
      <arg type="s" direction="in" name="mime"/>
      <arg type="ay" direction="in" name="data"/>
    </method>
    <signal name="Offer">
      <arg type="u" name="serial"/>
      <arg type="as" name="mimes"/>
      <arg type="s" name="owner"/>
    </signal>
  </interface>
</node>`;

const AppProxy = Gio.DBusProxy.makeProxyWrapper(APP_XML);

const TexButton = GObject.registerClass(
class TexButton extends PanelMenu.Button {
    _init(onClick) {
        super._init(0.0, 'Tex', false);
        this._onClick = onClick;
        this.add_child(new St.Icon({
            gicon: new Gio.ThemedIcon({
                names: [`${APP_ID}-symbolic`, APP_ID, 'edit-find-symbolic'],
            }),
            style_class: 'system-status-icon',
        }));
    }

    vfunc_event(event) {
        if (event.type() === Clutter.EventType.BUTTON_PRESS &&
            event.get_button() === Clutter.BUTTON_PRIMARY) {
            this._onClick();
            return Clutter.EVENT_STOP;
        }
        return super.vfunc_event(event);
    }
});

class Bridge {
    constructor(uuid) {
        this._uuid = uuid;
        this._mutter = new Gio.Settings({schema_id: 'org.gnome.mutter'});
        this._grabs = new Map();
        this._pending = null;
        this._pendingTimeout = 0;
        this._app = null;
        this._button = null;
        this._clipboard = St.Clipboard.get_default();
        this._serial = 0;
        this._writing = false;

        this._exported = Gio.DBusExportedObject.wrapJSObject(SHELL_XML, this);
        this._exported.export(Gio.DBus.session, SHELL_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, SHELL_NAME, Gio.BusNameOwnerFlags.NONE, null, null);

        this._acceleratorId = global.display.connect('accelerator-activated',
            (_display, action) => this._acceleratorActivated(action));
        this._windowCreatedId = global.display.connect('window-created',
            (_display, window) => this._windowCreated(window));
        this._selection = global.display.get_selection();
        this._ownerChangedId = this._selection.connect('owner-changed',
            (_selection, type, source) => this._ownerChanged(type, source));
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, APP_ID,
            Gio.BusNameWatcherFlags.NONE,
            () => this._appAppeared(),
            () => this._appVanished());
    }

    destroy() {
        Gio.bus_unwatch_name(this._watchId);
        this._appVanished();
        global.display.disconnect(this._acceleratorId);
        global.display.disconnect(this._windowCreatedId);
        this._selection.disconnect(this._ownerChangedId);
        this._ungrabAll();
        if (this._pendingTimeout)
            GLib.source_remove(this._pendingTimeout);
        Gio.bus_unown_name(this._nameId);
        this._exported.unexport();
    }

    WorkAreaUnderCursor() {
        return this._workArea(global.display.get_current_monitor());
    }

    PrimaryWorkArea() {
        return this._workArea(Main.layoutManager.primaryIndex);
    }

    Present(title, x, y) {
        const window = global.display.list_all_windows()
            .find(candidate => this._isTex(candidate, title));
        const actor = window?.get_compositor_private();
        if (window && actor?.visible) {
            this._place(window, x, y);
            return;
        }
        if (window && actor) {
            const id = actor.connect('first-frame', () => {
                actor.disconnect(id);
                this._place(window, x, y);
            });
            return;
        }
        this._pending = {title, x, y};
        if (this._pendingTimeout)
            GLib.source_remove(this._pendingTimeout);
        this._pendingTimeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT,
            PRESENT_WAIT_SECONDS, () => {
                this._pending = null;
                this._pendingTimeout = 0;
                return GLib.SOURCE_REMOVE;
            });
    }

    SetShortcuts(tex, history) {
        this._ungrabAll();
        return [this._grab('tex', tex), this._grab('history', history)];
    }

    DeliverAsync([serial, mime], invocation) {
        if (serial !== this._serial) {
            invocation.return_value(new GLib.Variant('(ay)', [new Uint8Array()]));
            return;
        }
        this._clipboard.get_content(St.ClipboardType.CLIPBOARD, mime, (_clipboard, bytes) => {
            const data = bytes?.get_data() ?? new Uint8Array();
            invocation.return_value(new GLib.Variant('(ay)', [data]));
        });
    }

    SetClipboard(mime, data) {
        this._writing = true;
        try {
            this._clipboard.set_content(St.ClipboardType.CLIPBOARD, mime, new GLib.Bytes(data));
        } finally {
            this._writing = false;
        }
    }

    _ownerChanged(type, source) {
        if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
            return;
        this._serial += 1;
        if (this._writing || !source)
            return;
        this._exported.emit_signal('Offer', new GLib.Variant('(uass)',
            [this._serial, source.get_mimetypes(), this._focusedExe()]));
    }

    _focusedExe() {
        const pid = global.display.focus_window?.get_pid() ?? -1;
        if (pid <= 0)
            return '';
        try {
            const path = GLib.file_read_link(`/proc/${pid}/exe`);
            return GLib.path_get_basename(path).replace(/ \(deleted\)$/, '');
        } catch {
            return '';
        }
    }

    _workArea(index) {
        const area = Main.layoutManager.getWorkAreaForMonitor(index);
        const logical = this._mutter.get_strv('experimental-features')
            .includes('scale-monitor-framebuffer');
        const scale = logical ? 1 : global.display.get_monitor_scale(index);
        return [area.x, area.y, area.width, area.height, scale];
    }

    _isTex(window, title) {
        return window.get_wm_class() === APP_ID && window.get_title() === title;
    }

    _windowCreated(window) {
        const pending = this._pending;
        if (!pending || window.get_wm_class() !== APP_ID)
            return;
        const actor = window.get_compositor_private();
        if (!actor)
            return;
        const id = actor.connect('first-frame', () => {
            actor.disconnect(id);
            if (this._pending === pending && window.get_title() === pending.title) {
                this._pending = null;
                this._place(window, pending.x, pending.y);
            }
        });
    }

    _place(window, x, y) {
        if (x >= 0 && y >= 0)
            window.move_frame(true, x, y);
        Main.activateWindow(window);
    }

    _grab(id, accelerator) {
        if (!accelerator)
            return true;
        const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
        if (action === Meta.KeyBindingAction.NONE)
            return false;
        const name = Meta.external_binding_name_for_action(action);
        Main.wm.allowKeybinding(name, Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW);
        this._grabs.set(action, {id, name});
        return true;
    }

    _ungrabAll() {
        for (const [action, {name}] of this._grabs) {
            global.display.ungrab_accelerator(action);
            Main.wm.allowKeybinding(name, Shell.ActionMode.NONE);
        }
        this._grabs.clear();
    }

    _acceleratorActivated(action) {
        const grab = this._grabs.get(action);
        if (grab)
            this._exported.emit_signal('ShortcutActivated', new GLib.Variant('(s)', [grab.id]));
    }

    _appAppeared() {
        new AppProxy(Gio.DBus.session, APP_ID, APP_PATH, (proxy, error) => {
            if (error) {
                console.error(`Tex: ${error.message}`);
                return;
            }
            this._app = proxy;
            this._propertiesId = proxy.connect('g-properties-changed', () => this._fillMenu());
            this._button = new TexButton(() => this._call('Toggle'));
            this._fillMenu();
            Main.panel.addToStatusArea(this._uuid, this._button, 0, 'right');
        }, null, Gio.DBusProxyFlags.DO_NOT_AUTO_START);
    }

    _appVanished() {
        this._button?.destroy();
        this._button = null;
        if (this._app)
            this._app.disconnect(this._propertiesId);
        this._app = null;
    }

    _fillMenu() {
        if (!this._button)
            return;
        const menu = this._button.menu;
        menu.removeAll();
        const labels = this._app?.Labels ?? [];
        const add = (index, fallback, method) => {
            const item = new PopupMenu.PopupMenuItem(labels[index] || fallback);
            item.connect('activate', () => this._call(method));
            menu.addMenuItem(item);
        };
        add(0, 'Open Tex', 'Toggle');
        add(1, 'Clipboard history', 'History');
        add(2, 'Settings', 'Settings');
        add(3, 'About Tex', 'About');
        add(4, 'Pause capture', 'TogglePause');
        if (this._app?.UpdateReady)
            add(5, 'Update', 'ApplyUpdate');
        menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        add(6, 'Quit', 'Quit');
    }

    _call(method) {
        this._app?.[`${method}Remote`](() => {});
    }
}

export default class TexExtension extends Extension {
    enable() {
        this._bridge = new Bridge(this.uuid);
    }

    disable() {
        this._bridge?.destroy();
        this._bridge = null;
    }
}
