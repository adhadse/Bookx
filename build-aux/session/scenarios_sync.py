"""The Sync umbrella with real input in a real GNOME session: main menu -> Sync (title, state lines, buttons), the
Open Preferences -> Sync button, a switch changed in Preferences showing in the Sync window below it, and the
"Copy to the device" plugin button on a fake KOReader e-reader (no real Kobo). Screenshots in the results folder,
light/dark, 900x600 / 1280x800, English / German. Run: ONLY='sync_900_light_en ...' build-aux/session-tests.sh"""
import os
import re
import time

from harness import scenario

DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"

MENU = "Main menu|Hauptmenü"
SYNC_ENTRY = "^(Sync|Synchronisieren)$"
INTERACTIVE = ("push button", "toggle button", "check box", "radio button", "switch", "combo box", "spin button",
               "slider", "text", "entry", "menu button", "link")


def _seed(c):
    m = os.path.join(c.dir, "mounts", "KOBOeReader")
    os.makedirs(os.path.join(m, ".kobo"), exist_ok=True)
    os.makedirs(os.path.join(m, ".adds", "koreader", "plugins"), exist_ok=True)


def _labels(c):
    return [n.name for n in c.tree().nodes if n.showing and n.name]


def _click(c, role=None, rx=None, timeout=8):
    n = c.wait_node(role, rx=rx, timeout=timeout)
    if n:
        c.click_node(n)
        time.sleep(0.9)
    return n


def _inside(n, box):
    x, y, w, h = box
    return n.showing and n.box[2] > 0 and y + 40 <= n.box[1] and n.box[1] + n.box[3] <= y + h - 30 and x <= n.box[0] < x + w


def _scroll_to(c, box, rx, role=None, tries=14, step=4, min_frac=0.0):
    """Scrolls the top dialog at `box` until a control matching rx is inside it; returns that control. With two
    dialogs open (Preferences over the Sync window) the matches of the later one, the dialog on top, are used."""
    x, y, w, h = box
    for _ in range(tries):
        matches = [n for n in (c.find(role, rx=rx) or []) if n.showing and n.box[2] > 0]
        if matches and _inside(matches[-1], box):
            return matches[-1]
        c.sb.scroll(step, x + w * 0.62, y + h * 0.5)
        time.sleep(0.3)
    return None


def _press(c, box, rx, role="button", tries=14, step=4):
    """Scrolls to a control inside the dialog and clicks it."""
    n = _scroll_to(c, box, rx, role, tries=tries, step=step)
    if n:
        c.click_node(n)
        time.sleep(0.9)
    return n


def sync_case(c, size, dark, lang):
    w, h = size
    c.start(books=("alice",), settings=DARK if dark else "", size=f"{w}x{h}", seed=_seed,
            extra_env={"BOOKX_LANG": lang, "BOOKX_DEV_SCRIPT": "wait:600,share:port0"})
    c.wait_node("table cell", rx="Alice")
    time.sleep(2)
    tag = f"{lang}-{'dark' if dark else 'light'}-{w}"
    en = lang == "en"
    opened = c.wait_node("toggle button", rx=MENU, timeout=3)
    if not opened:  # narrow window: the menu is in the sidebar, which is an overlay there
        _click(c, "toggle button", "^(Sidebar|Seitenleiste)$")
    opened = _click(c, "toggle button", MENU)
    if not opened:
        c.note("TREE\n" + c.tree().dump())
    c.check("the main menu opens", opened)
    c.shot(f"{tag}-menu")
    # the device banner has a "Sync" button too: the menu entry is the lower one, inside the popover
    entry = None
    for _ in range(10):
        found = [n for n in (c.find("button", rx=SYNC_ENTRY) or []) if n.showing and n.box[2] > 0]
        entry = max(found, key=lambda n: n.box[1]) if found else None
        if entry and (len(found) == 1 and entry.box[1] > 100 or len(found) > 1):
            break
        time.sleep(0.5)
    if entry:
        c.click_node(entry)
        time.sleep(1.0)
    c.check("the menu entry is called Sync (not Share Library)", entry)
    d = c.wait_node("dialog", timeout=8)
    if not d:
        c.shot(f"{tag}-no-dialog")
        c.note("TREE\n" + c.tree().dump())
    c.check("the Sync window opens", d)
    if not d:
        return
    time.sleep(1.2)
    c.check("its title is Sync", re.match(SYNC_ENTRY, d.name or ""), d.name)
    labels = _labels(c)
    c.shot(f"{tag}-sync-window")
    # first screen: status card, one big button, three choice cards, no KOSync, Advanced collapsed
    c.check("the status card says Bookx sync is off", any(re.match(r"Bookx[ -]sync is off|Bookx-Sync ist aus", l, re.I)
                                                          for l in labels), str(labels[:30]))
    c.check("it has the one big button Set up Bookx sync",
            c.find("button", rx=r"^(Set up Bookx sync|Bookx-Sync einrichten)$"))
    c.check("it has the plain sentence about KOReader over Wi-Fi and USB",
            any("Wi-Fi" in l or "WLAN" in l for l in labels if len(l) > 60) and any("USB" in l for l in labels if len(l) > 60))
    use = [l for l in labels if re.match(r"(Use this|Nutzen Sie dies)", l)]
    c.check("three choice cards say when to use them", len(use) >= 3, str(use))
    c.check("each card has its button",
            c.find("button", rx=r"^(Start setup|Einrichtung starten)") and c.find("button", rx=r"^(Open device window|Geräte-Fenster öffnen)")
            and c.find("button", rx=r"^(Start library server|Bibliotheksserver starten)"))
    c.check("no KOSync label on the first screen", not [l for l in labels if "KOSync" in l])
    c.check("no /kosync text", not [l for l in labels if "/kosync" in l])
    c.check("Advanced is there", c.find(None, rx=r"^(Advanced|Erweitert)"))
    nodes = [n for n in c.tree().nodes if n.showing and n.box[2] > 0]
    dx, dy, dw, dh = sbox = d.box
    c.check("nothing sticks out of the window", not [n for n in nodes if n.depth > d.depth and n.box[0] + n.box[2] > dx + dw + 2
                                                    and n.role != "scroll bar"])
    c.check("every control is named", not [n for n in nodes if n.role in INTERACTIVE and not n.name and n.depth > d.depth])
    # the Wi-Fi card is a compact stepper: every step opens to its text
    h2 = _press(c, sbox, r"^(Step|Schritt) 2 (of|von) 4", "button")
    c.check("the Wi-Fi card has a stepper whose step 2 opens", h2)
    time.sleep(0.5)
    c.shot(f"{tag}-sync-window-step2")
    c.check("step 2 shows its text and the Open Preferences button",
            any("not the Share Library password" in l or "nicht das Passwort" in l for l in _labels(c)))
    _press(c, sbox, r"^(Step|Schritt) 2 (of|von) 4", "button")
    c.sb.scroll(-40, dx + dw * 0.62, dy + dh * 0.5)
    # the third card starts the library server and says so
    btn = _press(c, sbox, r"^(Start library server|Bibliotheksserver starten)")
    c.check("the library server card starts the server", btn)
    time.sleep(1.0)
    c.check("the card now offers Stop library server", c.find("button", rx=r"^(Stop library server|Bibliotheksserver stoppen)"))
    c.shot(f"{tag}-sync-window-opds-on")
    _press(c, sbox, r"^(Stop library server|Bibliotheksserver stoppen)")
    # Advanced
    adv = _scroll_to(c, sbox, r"^(Advanced|Erweitert)")
    if adv:
        c.click_node(adv)
        time.sleep(0.8)
        c.sb.scroll(8, dx + dw * 0.62, dy + dh * 0.5)
        time.sleep(0.5)
        c.shot(f"{tag}-sync-window-advanced")
        c.sb.scroll(-30, dx + dw * 0.62, dy + dh * 0.5)

    # Start setup opens Preferences -> Sync (the checklist)
    c.sb.scroll(-40, dx + dw * 0.62, dy + dh * 0.5)
    time.sleep(0.4)
    c.check("Start setup opens Preferences", _press(c, sbox, r"^(Start setup|Einrichtung starten)"))
    time.sleep(1.2)
    pd = [n for n in c.tree().nodes if n.role == "dialog" and n.showing][-1]
    dx, dy, dw, dh = pd.box
    c.shot(f"{tag}-prefs-sync")
    labels = _labels(c)
    STEP = r"^(Step|Schritt) %d (of|von) 4"
    c.check("the checklist is a stepper: four numbered step headers",
            all(c.find("button", rx=STEP % n) for n in (1, 2, 3, 4)), str(labels[:40]))
    c.check("step 1 is open: its switch is there",
            c.find(None, rx=r"^(Bookx sync server|Bookx-Sync-Server)$"))
    c.check("no /kosync text on the page", not [l for l in labels if "/kosync" in l])
    nodes = [n for n in c.tree().nodes if n.showing and n.box[2] > 0]
    c.check("nothing sticks out of the Preferences window (fit)", not [n for n in nodes if n.depth > pd.depth
            and n.box[0] + n.box[2] > dx + dw + 2 and n.role != "scroll bar"])
    c.check("every control is named", not [n for n in nodes if n.role in INTERACTIVE and not n.name and n.depth > pd.depth])
    # step 1: the switch; step 2 opens by itself afterwards
    sw = _scroll_to(c, pd.box, r"^(Bookx sync server|Bookx-Sync-Server)$", step=-3, tries=4)
    c.check("step 1 switch exists", sw)
    if sw:
        c.click_node(sw)
        time.sleep(1.6)
        c.shot(f"{tag}-prefs-step1-on")
    # step 2: the account (open by itself now)
    c.shot(f"{tag}-prefs-account")
    labels = _labels(c)
    c.check("step 2 opened by itself and shows User name and Password fields",
            any(re.match(r"(User name|Benutzername)", l) for l in labels) and any(re.match(r"(Password|Passwort)", l) for l in labels), str(labels[:40]))
    c.check("it has Register and Log in buttons",
            c.find("button", rx=r"^(Register|Registrieren)") and c.find("button", rx=r"^(Log in|Anmelden)"))
    c.check("the step text says it is not the Share Library password",
            any("not the Share Library password" in l or "nicht das Passwort" in l for l in labels))
    # step 3: the plugin, copied onto the fake e-reader
    h3 = _scroll_to(c, pd.box, STEP % 3, "button", step=3)
    if h3:
        c.click_node(h3)
        time.sleep(1.0)
    cp = _scroll_to(c, pd.box, r"^(Copy to the device|Auf das Gerät kopieren)", "button", step=3)
    c.check("step 3 opens and has the Copy to the device button", cp)
    if cp:
        c.click_node(cp)
        time.sleep(1.5)
        plug = os.path.join(c.dir, "mounts", "KOBOeReader", ".adds", "koreader", "plugins", "bookx.koplugin", "main.lua")
        c.check("the plugin was copied onto the fake KOReader device", os.path.isfile(plug), plug)
        c.shot(f"{tag}-prefs-plugin-copied")
    # step 4 and the default of the notes switch
    h4 = _scroll_to(c, pd.box, STEP % 4, "button", step=3)
    if h4:
        c.click_node(h4)
        time.sleep(0.8)
        c.shot(f"{tag}-prefs-step4")
    c.sb.scroll(-40, dx + dw * 0.62, dy + dh * 0.5)
    time.sleep(0.4)
    notes = _scroll_to(c, pd.box, r"^(Sync highlights, notes and bookmarks|Markierungen, Notizen und Lesezeichen synchronisieren)")
    c.check("the highlights, notes and bookmarks switch is there", notes)
    if notes:
        c.shot(f"{tag}-prefs-notes-default")
        sw_nodes = [n for n in (c.find(None, rx=r"^(Sync highlights, notes and bookmarks|Markierungen, Notizen und Lesezeichen synchronisieren)") or [])
                    if n.showing]
        c.check("it is switched on by default", any(n.checked for n in sw_nodes), str([(n.role, n.checked) for n in sw_nodes]))
    # Advanced expander of the page
    adv = _scroll_to(c, pd.box, r"^(Advanced|Erweitert)", step=6)
    c.check("Advanced is at the bottom of the page", adv)
    if adv:
        c.click_node(adv)
        time.sleep(0.8)
        c.sb.scroll(6, dx + dw * 0.62, dy + dh * 0.5)
        time.sleep(0.5)
        c.shot(f"{tag}-prefs-advanced")
        labels = _labels(c)
        c.check("Advanced shows the technical fields (port, HTTPS)", any(re.match(r"(Port|Use HTTPS|HTTPS verwenden)", l) for l in labels),
                str(labels[:40]))
    c.sb.key("Escape")
    time.sleep(1.0)
    labels = _labels(c)
    c.check("the Sync window below followed: Bookx sync is ready",
            any(re.match(r"Bookx[ -]sync is ready|Bookx-Sync ist bereit", l, re.I) for l in labels), str(labels[:30]))
    c.shot(f"{tag}-sync-window-ready")
    c.check("the app is alive", c.is_alive())


def _register():
    for size, dark, lang in (((900, 600), False, "en"), ((900, 600), True, "de"), ((1280, 800), False, "de"),
                             ((1280, 800), True, "en")):
        def run(c, size=size, dark=dark, lang=lang):
            sync_case(c, size, dark, lang)
        run.__name__ = f"sync_{size[0]}_{'dark' if dark else 'light'}_{lang}"
        scenario(run)


_register()
