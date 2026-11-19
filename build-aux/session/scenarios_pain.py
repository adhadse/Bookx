"""Pain-point work with real input: the reading ruler keys. Imported by scenarios.py."""
import json
import os
import time

from harness import scenario


def _ruler(cfg_home):
    try:
        with open(os.path.join(cfg_home, "bookx", "config.json")) as f:
            return json.load(f).get("ruler", {})
    except (OSError, ValueError):
        return {}


@scenario
def reading_ruler_keys(c):
    """Alt+Down turns the reading ruler on from the keyboard, Alt+Up keeps it on, the palette turns it off."""
    c.start()
    c.open_book()
    time.sleep(1.5)
    home = os.environ["XDG_CONFIG_HOME"]
    c.check("the ruler is off by default", not _ruler(home).get("enabled", False))
    c.sb.key("Down", ("Alt",))
    time.sleep(1.0)
    c.check("Alt+Down turns the ruler on", _ruler(home).get("enabled") is True, str(_ruler(home)))
    c.sb.key("Up", ("Alt",))
    time.sleep(0.8)
    c.check("Alt+Up keeps it on", _ruler(home).get("enabled") is True)
    c.shot("ruler")
    # turning the page still works with the ruler on
    p0 = c.page_label()
    c.sb.key("Right")
    time.sleep(1.0)
    p1 = c.page_label()
    c.check("the page can still be turned with the ruler on", p0 and p1 and p1 != p0, f"{p0} -> {p1}")
