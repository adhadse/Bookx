"""A real GNOME session in a box: gnome-shell (Mutter) headless, driven with real input.

Everything here runs INSIDE `dbus-run-session` (see ../session-tests.sh), so the session bus
is private. Input goes through Mutter's own RemoteDesktop D-Bus API, screenshots through
org.gnome.Shell.Screenshot, widget inspection through AT-SPI. Nothing touches the user's session.
"""
import os
import signal
import subprocess
import sys
import time

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

BTN_LEFT, BTN_RIGHT = 0x110, 0x111  # linux evdev codes

KEYSYMS = {
    "Return": 0xFF0D, "Escape": 0xFF1B, "Tab": 0xFF09, "BackSpace": 0xFF08, "Delete": 0xFFFF,
    "Left": 0xFF51, "Up": 0xFF52, "Right": 0xFF53, "Down": 0xFF54,
    "Home": 0xFF50, "End": 0xFF57, "Page_Up": 0xFF55, "Page_Down": 0xFF56,
    "Shift": 0xFFE1, "Control": 0xFFE3, "Alt": 0xFFE9, "Super": 0xFFEB, "space": 0x20,
    **{f"F{n}": 0xFFBD + n for n in range(1, 13)},
}


def keysym(name):
    if name in KEYSYMS:
        return KEYSYMS[name]
    if len(name) == 1:
        return ord(name)  # latin-1 keysyms equal their code point
    raise KeyError(name)


class Sandbox:
    """One compositor + its input/screenshot plumbing. `size` is the virtual monitor."""

    def __init__(self, workdir, size="1280x800", scale=None, settings=""):
        self.dir = workdir
        self.settings = settings  # extra gsettings keyfile text, for example "[org/gnome/desktop/a11y/interface]\nhigh-contrast=true\n"
        self.size = size
        self.w, self.h = (int(v) for v in size.split("x"))
        self.scale = scale
        self.procs = []
        self.results = os.path.join(workdir, "results")
        self.env = None
        self.bus = None
        self.rd = None
        self.rd_session = None
        self.px = self.py = 0

    # ---- start / stop -------------------------------------------------------------------------
    def start(self):
        d = self.dir
        home = os.environ["HOME"]
        env = dict(os.environ)
        for k in ("DISPLAY", "WAYLAND_DISPLAY", "GDK_BACKEND", "BROADWAY_DISPLAY", "XDG_SESSION_ID",
                  "GNOME_SETUP_DISPLAY", "SSH_AUTH_SOCK", "XDG_CURRENT_DESKTOP"):
            env.pop(k, None)
        env.update({
            "GSETTINGS_BACKEND": "keyfile",
            "XDG_SESSION_TYPE": "wayland",
            "XDG_CURRENT_DESKTOP": "GNOME",
            "NO_AT_BRIDGE": "0",
            "GTK_A11Y": "atspi",
        })
        for k in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
            os.makedirs(env[k], exist_ok=True)
        self.env = env
        os.environ.update({k: env[k] for k in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME")})
        # keep a fixed, predictable look: light Adwaita, no animations, no screen blanking
        ks = os.path.join(env["XDG_CONFIG_HOME"], "glib-2.0", "settings")
        os.makedirs(ks, exist_ok=True)
        with open(os.path.join(ks, "keyfile"), "w") as f:
            f.write("[org/gnome/desktop/interface]\n"
                    "enable-animations=false\nenable-hot-corners=false\ncolor-scheme='default'\ntoolkit-accessibility=true\n"
                    "[org/gnome/desktop/session]\nidle-delay=uint32 0\n"
                    "[org/gnome/desktop/screensaver]\nlock-enabled=false\n"
                    "[org/gnome/desktop/lockdown]\ndisable-lock-screen=true\n"
                    "[org/gnome/desktop/a11y/applications]\nscreen-reader-enabled=false\n"
                    "[org/gnome/mutter]\ncheck-alive-timeout=uint32 0\n" + self.settings)
        os.environ.update(env)
        self.shell_log = open(os.path.join(d, "shell.log"), "wb")
        args = ["gnome-shell", "--headless", "--wayland", "--no-x11", "--virtual-monitor", self.size]
        self.shell = self._spawn(args, self.shell_log)
        self.display = self._wait_display()
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self._own_screenshot_name()
        self._enable_a11y()
        self._wait_shell_ready()
        self._remote_desktop()
        # a fresh session starts in the Activities overview: leave it before the app is launched
        time.sleep(1.0)
        self.key("Escape")
        time.sleep(0.8)

    def _spawn(self, args, log, env=None, **kw):
        p = subprocess.Popen(args, env=env or self.env, stdout=log, stderr=subprocess.STDOUT,
                             stdin=subprocess.DEVNULL, start_new_session=True, **kw)
        self.procs.append(p)
        return p

    def _wait_display(self, timeout=40):
        rt = self.env["XDG_RUNTIME_DIR"]
        t0 = time.time()
        while time.time() - t0 < timeout:
            if self.shell.poll() is not None:
                raise RuntimeError("gnome-shell exited: see shell.log")
            if os.path.exists(os.path.join(rt, "wayland-0")):
                return "wayland-0"
            time.sleep(0.2)
        raise RuntimeError("no wayland socket")

    def _call(self, name, path, iface, method, params=None, sig=None, timeout=15000):
        return self.bus.call_sync(name, path, iface, method, params, None,
                                  Gio.DBusCallFlags.NONE, timeout, None)

    def _wait_shell_ready(self, timeout=40):
        t0 = time.time()
        while time.time() - t0 < timeout:
            try:
                r = self._call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                               "NameHasOwner", GLib.Variant("(s)", ("org.gnome.Mutter.RemoteDesktop",)))
                s = self._call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                               "NameHasOwner", GLib.Variant("(s)", ("org.gnome.Shell.Screenshot",)))
                if r[0] and s[0]:
                    time.sleep(1.5)  # let the shell finish its start-up animation / layout
                    return
            except GLib.Error:
                pass
            time.sleep(0.3)
        raise RuntimeError("shell not ready")

    def _enable_a11y(self):
        try:
            self.bus.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.freedesktop.DBus.Properties", "Set",
                               GLib.Variant("(ssv)", ("org.a11y.Status", "IsEnabled", GLib.Variant("b", True))),
                               None, Gio.DBusCallFlags.NONE, 5000, None)
        except GLib.Error as e:
            print("a11y enable failed:", e, file=sys.stderr)
        # the registry is normally started by a systemd user unit, which does not exist here
        self.registry = self._spawn(["/usr/libexec/at-spi2-registryd", "--use-gnome-session"],
                                    open(os.path.join(self.dir, "atspi-registry.log"), "wb"))

    def _own_screenshot_name(self):
        # gnome-shell only answers org.gnome.Shell.Screenshot to two well-known callers (MediaKeys, the portal), which it
        # recognises by the bus name they own. In our private bus we may simply own that name.
        r = self._call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                       "RequestName", GLib.Variant("(su)", ("org.gnome.SettingsDaemon.MediaKeys", 0)))
        if r[0] != 1:
            raise RuntimeError("could not own the MediaKeys name")

    def stop(self):
        for p in reversed(self.procs):
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        t0 = time.time()
        for p in self.procs:
            try:
                p.wait(max(0.1, 5 - (time.time() - t0)))
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    # ---- apps ---------------------------------------------------------------------------------
    def launch(self, args, extra_env=None, log="app.log"):
        env = dict(self.env)
        env.update({"GDK_BACKEND": "wayland", "WAYLAND_DISPLAY": self.display})
        env.update(extra_env or {})
        lf = open(os.path.join(self.dir, log), "wb")
        return self._spawn(args, lf, env=env)

    # ---- input --------------------------------------------------------------------------------
    def _remote_desktop(self):
        rd = "org.gnome.Mutter.RemoteDesktop"
        r = self._call(rd, "/org/gnome/Mutter/RemoteDesktop", rd, "CreateSession")
        self.rd_session = r[0]
        self.rd = Gio.DBusProxy.new_sync(self.bus, Gio.DBusProxyFlags.NONE, None, rd, self.rd_session,
                                         rd + ".Session", None)
        self.rd.call_sync("Start", None, Gio.DBusCallFlags.NONE, 10000, None)
        # a new session parks the pointer in the top-left corner, which is the shell's hot corner
        self._n("NotifyPointerMotionRelative", "(dd)", self.w / 2, self.h / 2)
        time.sleep(0.3)

    def _n(self, method, sig, *vals):
        self.rd.call_sync(method, GLib.Variant(sig, vals), Gio.DBusCallFlags.NONE, 5000, None)

    def move_to(self, x, y, _reset=False, steps=4):
        """Absolute pointer position in logical px. The RemoteDesktop API only has relative motion
        without a ScreenCast stream, so jump into the bottom-right corner first (a known position
        that is not the shell's hot corner), then move relatively from there."""
        self._n("NotifyPointerMotionRelative", "(dd)", float(self.w * 2), float(self.h * 2))
        self.px, self.py = self.w - 0.5, self.h - 0.5
        dx, dy = x - self.px, y - self.py
        for _ in range(steps):  # a few small steps like a real mouse (enter/motion events)
            self._n("NotifyPointerMotionRelative", "(dd)", dx / steps, dy / steps)
            time.sleep(0.01)
        self.px, self.py = x, y
        time.sleep(0.05)

    def move_by(self, dx, dy):
        self._n("NotifyPointerMotionRelative", "(dd)", float(dx), float(dy))
        self.px += dx
        self.py += dy

    def button(self, btn=BTN_LEFT, down=True):
        self._n("NotifyPointerButton", "(ib)", btn, down)

    def click(self, x=None, y=None, btn=BTN_LEFT, count=1, gap=0.07):
        if x is not None:
            self.move_to(x, y)
        for i in range(count):
            self.button(btn, True)
            time.sleep(0.04)
            self.button(btn, False)
            if i + 1 < count:
                time.sleep(gap)
        time.sleep(0.15)

    def click_with(self, mod, x, y, btn=BTN_LEFT):
        """A click with a modifier key held (Control, Shift)."""
        self._n("NotifyKeyboardKeysym", "(ub)", keysym(mod), True)
        try:
            self.click(x, y, btn=btn)
        finally:
            self._n("NotifyKeyboardKeysym", "(ub)", keysym(mod), False)

    def double_click(self, x=None, y=None):
        self.click(x, y, count=2)

    def right_click(self, x=None, y=None):
        self.click(x, y, btn=BTN_RIGHT)

    def drag(self, x0, y0, x1, y1):
        self.move_to(x0, y0)
        self.button(BTN_LEFT, True)
        time.sleep(0.1)
        for i in range(1, 11):
            self.move_by((x1 - x0) / 10, (y1 - y0) / 10)
            time.sleep(0.03)
        self.button(BTN_LEFT, False)
        time.sleep(0.2)

    def scroll(self, clicks, x=None, y=None):
        """Wheel steps; positive = down."""
        if x is not None:
            self.move_to(x, y)
        for _ in range(abs(clicks)):
            self._n("NotifyPointerAxisDiscrete", "(ui)", 0, 1 if clicks > 0 else -1)
            time.sleep(0.05)
        time.sleep(0.2)

    def key(self, name, mods=()):
        for m in mods:
            self._n("NotifyKeyboardKeysym", "(ub)", keysym(m), True)
        k = keysym(name)
        self._n("NotifyKeyboardKeysym", "(ub)", k, True)
        time.sleep(0.03)
        self._n("NotifyKeyboardKeysym", "(ub)", k, False)
        for m in reversed(mods):
            self._n("NotifyKeyboardKeysym", "(ub)", keysym(m), False)
        time.sleep(0.12)

    def type(self, text):
        for ch in text:
            if ch == "\n":
                self.key("Return")
                continue
            shift = ch.isupper() or ch in '~!@#$%^&*()_+{}|:"<>?'
            self.key(ch, ("Shift",) if shift and False else ())
        time.sleep(0.2)

    # ---- screenshots --------------------------------------------------------------------------
    def screenshot(self, name):
        path = os.path.join(self.results, f"{name}.png")
        self.screenshot_to(path, cursor=True)
        return path

    def screenshot_to(self, path, cursor=False):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        r = self._call("org.gnome.Shell.Screenshot", "/org/gnome/Shell/Screenshot",
                       "org.gnome.Shell.Screenshot", "Screenshot",
                       GLib.Variant("(bbs)", (cursor, False, path)))
        if not r[0]:
            raise RuntimeError("screenshot failed")
