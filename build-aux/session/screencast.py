"""A real screen recording of the sandboxed GNOME Shell session (used by the wiki videos, scenarios_wiki_video.py).

How it works, and why this and not GNOME Shell's own `org.gnome.Shell.Screencast`: the headless nested shell does not export
that service (it is not on the session bus), but Mutter's `org.gnome.Mutter.ScreenCast` is, and the portal-free API is open
to us on the private bus. So the recorder starts a private PipeWire daemon (+ session manager) in the sandbox's runtime
folder, asks Mutter for a screen-cast stream of an area (`RecordArea`, the pointer is not drawn), and records that stream
with GStreamer (`pipewiresrc`) at a constant frame rate into a nearly lossless intermediate Matroska file. That is a
continuous recording of what the compositor draws, not a series of screenshots. build-aux/wiki_video.py encodes it.
"""
import os
import signal
import subprocess
import time

from gi.repository import Gio, GLib

MUTTER = "org.gnome.Mutter.ScreenCast"


def gst_command(node, out, fps=30):
    """The gst-launch command that records PipeWire node `node` at a constant `fps` into `out` (Matroska, H.264, almost
    lossless; `keepalive-time` repeats the last picture while nothing on the screen changes so the time line has no gaps)."""
    return ["gst-launch-1.0", "-e", "pipewiresrc", f"path={node}", f"keepalive-time={int(1000 / fps)}", "always-copy=true", "!",
            "videoconvert", "!", "videorate", "!", f"video/x-raw,framerate={fps}/1", "!",
            "x264enc", "speed-preset=ultrafast", "pass=quant", "quantizer=10", "key-int-max=60", "!",
            "matroskamux", "!", "filesink", f"location={out}"]


class Screencast:
    def __init__(self, c, area=(0, 32, 1280, 768), fps=30):
        self.c = c
        self.area = area
        self.fps = fps
        self.procs = []
        self.session = None
        self.gst = None
        self.out = None

    def _spawn(self, args, log, env):
        p = subprocess.Popen(args, env=env, stdout=open(log, "wb"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                             start_new_session=True)
        self.procs.append(p)
        return p

    def start(self, out):
        c = self.c
        env = dict(c.sb.env)
        self.out = out
        self._spawn(["pipewire"], os.path.join(c.dir, "pipewire.log"), env)
        time.sleep(1.0)
        self._spawn(["pipewire-media-session"], os.path.join(c.dir, "pipewire-session.log"), env)
        time.sleep(1.0)
        bus = c.sb.bus
        call = lambda path, iface, method, params: bus.call_sync(  # noqa: E731
            MUTTER, path, f"{MUTTER}.{iface}" if iface else MUTTER, method, params, None, Gio.DBusCallFlags.NONE, 8000, None)
        self.session = call("/org/gnome/Mutter/ScreenCast", "", "CreateSession", GLib.Variant("(a{sv})", ({},)))[0]
        x, y, w, h = self.area
        stream = call(self.session, "Session", "RecordArea",
                      GLib.Variant("(iiiia{sv})", (x, y, w, h, {"cursor-mode": GLib.Variant("u", 0)})))[0]
        nodes = []
        sub = bus.signal_subscribe(MUTTER, f"{MUTTER}.Stream", "PipeWireStreamAdded", stream, None, Gio.DBusSignalFlags.NONE,
                                   lambda *a: nodes.append(a[5][0]))
        call(self.session, "Session", "Start", None)
        t0 = time.time()
        while not nodes and time.time() - t0 < 8:
            GLib.MainContext.default().iteration(False)
            time.sleep(0.02)
        bus.signal_unsubscribe(sub)
        if not nodes:
            raise RuntimeError("Mutter's screen cast gave no PipeWire node (see pipewire.log)")
        self.gst = self._spawn(gst_command(nodes[0], out, self.fps), os.path.join(c.dir, "gst-record.log"), env)
        # wait for the first bytes: the clip must not start before the recorder runs
        t0 = time.time()
        while time.time() - t0 < 10 and (not os.path.exists(out) or os.path.getsize(out) < 2000):
            time.sleep(0.1)
        if not os.path.exists(out) or os.path.getsize(out) < 2000:
            raise RuntimeError("the recorder wrote nothing (see gst-record.log)")

    def stop(self):
        if self.gst is not None:
            try:
                os.killpg(self.gst.pid, signal.SIGINT)
                self.gst.wait(20)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
            self.gst = None
        if self.session:
            try:
                self.c.sb.bus.call_sync(MUTTER, self.session, f"{MUTTER}.Session", "Stop", None, None,
                                        Gio.DBusCallFlags.NONE, 5000, None)
            except GLib.Error:
                pass
            self.session = None
        for p in reversed(self.procs):
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        self.procs = []
