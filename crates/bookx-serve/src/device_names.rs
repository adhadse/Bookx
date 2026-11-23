//! Names of e-readers for people.
//!
//! KOReader tells Bookx what it knows about the device it runs on: `Device.model`. That is not a
//! marketing name. On Android it is `Build.PRODUCT` (`android.prop.product`, a codename such as
//! `leaf2`), on a Kobo it is `Kobo_<codename>` (`Kobo_nova`), on a Kindle `KindlePaperWhite5`, on a
//! PocketBook `PB629`. A Boox may even report a vendor prefix and a board name (`Onyx
//! Scubapilot-IDP`, issue #39). [`friendly_name`] turns what a device reported into the name on
//! the box, and **never invents one**: a model that is not in the table below is shown as the
//! device reported it (only tidied), and the user can rename the device in Bookx
//! ([`crate::activity::ActivityLog::rename`]).
//!
//! Sources of the table (all public, nothing guessed from memory):
//!
//! * Onyx/Boox: `koreader/android-luajit-launcher`, `DeviceInfo.kt`, which matches `Build.MODEL` /
//!   `Build.PRODUCT` / `Build.DEVICE` of each Onyx model and names it in a comment. KOReader sends
//!   `PRODUCT`; for most models `MODEL` and `PRODUCT` are the same word, but only the keys that
//!   KOReader matches are listed, so an entry matched there on `MODEL` only is a best effort.
//! * Kobo, Kindle, PocketBook, reMarkable: `koreader/koreader`, `frontend/device/<brand>/device.lua`
//!   (`model = "..."` and the comment above it).
//! * `scubapilot` and `scubapilot-idp`: not in KOReader's table. Reported by the owner of a Boox
//!   Leaf 2 in Bookx issue #39 (their device showed up as `Onyx Scubapilot-IDP`). Single report,
//!   not confirmed by Onyx documentation or a second device.
//!
//! No GTK, no I/O: a pure function with a table.

/// How a key may be matched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Match {
    /// Anywhere: the key is specific enough (`leaf2`, `kobonova`, `pb629`).
    Anywhere,
    /// Only when the device reported the Onyx/Boox vendor word too: the key alone is a common word
    /// (`page`, `note`, `max`), a user could have named a device that.
    WithVendor,
}

use Match::{Anywhere as A, WithVendor as V};

/// Onyx / Boox: `(normalised model, name, how it may be matched)`.
const ONYX: &[(&str, &str, Match)] = &[
    // reported in Bookx issue #39 by a Leaf 2 owner (not in KOReader's table)
    ("scubapilotidp", "Boox Leaf 2", A),
    ("scubapilot", "Boox Leaf 2", A),
    // KOReader DeviceInfo.kt
    ("leaf", "Boox Leaf", V),
    ("leaf2", "Boox Leaf 2", A),
    ("leaf5c", "Boox Leaf 5C", A),
    ("go6", "Boox Go 6", A),
    ("go62", "Boox Go 6 (Gen II)", A),
    ("go7", "Boox Go 7", A),
    ("gocolor7", "Boox Go Color 7", A),
    ("gocolor72", "Boox Go Color 7 (Gen II)", A),
    ("go103", "Boox Go 10.3", A),
    ("go1032lumi", "Boox Go 10.3 (Gen II)", A),
    ("noteair", "Boox Note Air", A),
    ("noteair2", "Boox Note Air 2", A),
    ("noteair2p", "Boox Note Air 2 Plus", A),
    ("noteair3", "Boox Note Air 3", A),
    ("noteair3c", "Boox Note Air 3C", A),
    ("noteair4c", "Boox Note Air 4C", A),
    ("noteair5c", "Boox Note Air 5C", A),
    ("note", "Boox Note", V),
    ("note3", "Boox Note 3", A),
    ("mcnote4", "Boox Note 4", A),
    ("note5", "Boox Note 5", A),
    ("notemax", "Boox Note Max", A),
    ("notepro", "Boox Note Pro", A),
    ("notes", "Boox Note S", V),
    ("notex2", "Boox Note X2", A),
    ("nova", "Boox Nova", V),
    ("nova2", "Boox Nova 2", A),
    ("nova3", "Boox Nova 3", A),
    ("nova3color", "Boox Nova 3 Color", A),
    ("novaair", "Boox Nova Air", A),
    ("novaair2", "Boox Nova Air 2", A),
    ("novaairc", "Boox Nova Air C", A),
    ("novapro", "Boox Nova Pro", A),
    ("page", "Boox Page", V),
    ("palma", "Boox Palma", A),
    ("palma2", "Boox Palma 2", A),
    ("palma2proc", "Boox Palma 2 Pro", A),
    ("poke2", "Boox Poke 2", A),
    ("poke3", "Boox Poke 3", A),
    ("poke4", "Boox Poke 4", A),
    ("poke4lite", "Boox Poke 4 Lite", A),
    ("poke5p", "Boox Poke 5", A),
    ("poke5s", "Boox Poke 5S", A),
    ("poke6", "Boox Poke 6", A),
    ("pokepro", "Boox Poke Pro", A),
    ("max", "Boox Max", V),
    ("max2pro", "Boox Max 2 Pro", A),
    ("tabultra", "Boox Tab Ultra", A),
    ("tabultrac", "Boox Tab Ultra C", A),
    ("tabultracpro", "Boox Tab Ultra C Pro", A),
    ("tabxc", "Boox Tab X C", A),
];

/// Kobo: KOReader's `Kobo_<codename>`, `(normalised, name)`.
const KOBO: &[(&str, &str)] = &[
    ("kobotrilogya", "Kobo Touch"),
    ("kobotrilogyb", "Kobo Touch"),
    ("kobotrilogyc", "Kobo Touch"),
    ("kobopixie", "Kobo Mini"),
    ("kobodaylight", "Kobo Aura One"),
    ("kobodahlia", "Kobo Aura H2O"),
    ("kobodragon", "Kobo Aura HD"),
    ("kobokraken", "Kobo Glo"),
    ("kobophoenix", "Kobo Aura"),
    ("kobosnow", "Kobo Aura H2O2"),
    ("kobosnowr2", "Kobo Aura H2O2"),
    ("kobostar", "Kobo Aura Edition 2"),
    ("kobostarr2", "Kobo Aura Edition 2"),
    ("koboalyssum", "Kobo Glo HD"),
    ("kobopika", "Kobo Touch 2.0"),
    ("kobonova", "Kobo Clara HD"),
    ("kobofrost", "Kobo Forma"),
    ("kobostorm", "Kobo Libra"),
    ("koboluna", "Kobo Nia"),
    ("koboeuropa", "Kobo Elipsa"),
    ("kobocadmus", "Kobo Sage"),
    ("koboio", "Kobo Libra 2"),
    ("kobogoldfinch", "Kobo Clara 2E"),
    ("kobocondor", "Kobo Elipsa 2E"),
    ("kobomonza", "Kobo Libra Colour"),
    ("kobospabw", "Kobo Clara BW"),
    ("kobospacolour", "Kobo Clara Colour"),
];

/// Kindle: KOReader's `Device.model`, `(normalised, name)`.
const KINDLE: &[(&str, &str)] = &[
    ("kindle2", "Kindle 2"),
    ("kindledxg", "Kindle DX Graphite"),
    ("kindle3", "Kindle 3"),
    ("kindle4", "Kindle 4"),
    ("kindletouch", "Kindle Touch"),
    ("kindlepaperwhite", "Kindle Paperwhite"),
    ("kindlepaperwhite2", "Kindle Paperwhite 2"),
    ("kindlebasic", "Kindle Basic"),
    ("kindlevoyage", "Kindle Voyage"),
    ("kindlepaperwhite3", "Kindle Paperwhite 3"),
    ("kindleoasis", "Kindle Oasis"),
    ("kindleoasis2", "Kindle Oasis 2"),
    ("kindleoasis3", "Kindle Oasis 3"),
    ("kindlebasic2", "Kindle Basic 2"),
    ("kindlepaperwhite4", "Kindle Paperwhite 4"),
    ("kindlebasic3", "Kindle Basic 3"),
    ("kindlepaperwhite5", "Kindle Paperwhite 5"),
    (
        "kindlepaperwhite5se",
        "Kindle Paperwhite 5 Signature Edition",
    ),
    ("kindlepaperwhite6", "Kindle Paperwhite 6"),
    ("kindlebasic4", "Kindle Basic 4"),
    ("kindlebasic5", "Kindle Basic 5"),
    ("kindlescribe", "Kindle Scribe"),
    ("kindlecolorsoft", "Kindle Colorsoft"),
    ("kindlescribecolorsoft", "Kindle Scribe Colorsoft"),
];

/// PocketBook: KOReader's `Device.model`, `(normalised, name)`.
const POCKETBOOK: &[(&str, &str)] = &[
    ("pb515", "PocketBook Mini"),
    ("pb606", "PocketBook Basic 4"),
    ("pb611", "PocketBook Basic"),
    ("pbblux", "PocketBook Basic Lux"),
    ("pbblux2", "PocketBook Basic Lux 2"),
    ("pbblux3", "PocketBook Basic Lux 3"),
    ("pbblux4", "PocketBook Basic Lux 4"),
    ("pbverselite", "PocketBook Verse Lite"),
    ("pbtouch", "PocketBook Touch"),
    ("pbtouchlux", "PocketBook Touch Lux"),
    ("pbbasictouch", "PocketBook Basic Touch"),
    ("pbbasictouch2", "PocketBook Basic Touch 2"),
    ("pblux4", "PocketBook Touch Lux 4"),
    ("pbtouchlux5", "PocketBook Touch Lux 5"),
    ("pb629", "PocketBook Verse"),
    ("pbtouchhd", "PocketBook Touch HD"),
    ("pbtouchhdplus", "PocketBook Touch HD Plus"),
    ("pbcolor", "PocketBook Color"),
    ("pb634", "PocketBook Verse Pro"),
    ("pbverseprocolor", "PocketBook Verse Pro Color"),
    ("pbaqua", "PocketBook Aqua"),
    ("pbaqua2", "PocketBook Aqua 2"),
    ("pbultra", "PocketBook Ultra"),
    ("pb700", "PocketBook Era"),
    ("pberacolor", "PocketBook Era Color"),
    ("pb710", "PocketBook Era Lite"),
    ("pbinkpad3", "PocketBook InkPad 3"),
    ("pbinkpad3pro", "PocketBook InkPad 3 Pro"),
    ("pbinkpadcolor", "PocketBook InkPad Color"),
    ("pbinkpadcolor2", "PocketBook InkPad Color 2"),
    ("pbinkpadcolor3", "PocketBook InkPad Color 3"),
];

/// reMarkable: `/sys/devices/soc0/machine` as KOReader reads it, `(normalised, name)`.
const REMARKABLE: &[(&str, &str)] = &[
    ("remarkable20", "reMarkable 2"),
    ("remarkableferrari", "reMarkable Paper Pro"),
];

/// The words that name a vendor and add nothing: dropped from the front of a model.
const VENDOR_WORDS: &[&str] = &["onyx", "boox"];

/// Lower case, letters and digits only: `Scubapilot-IDP` is `scubapilotidp`, `Kobo_snow_r2` is
/// `kobosnowr2`, `Go Color 7` is `gocolor7`.
fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Control characters out, runs of white space made one, trimmed.
fn tidy(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits leading vendor words (`Onyx`, `Boox`, in any case, repeated) off a tidied name.
fn strip_vendor(s: &str) -> (bool, &str) {
    let mut rest = s;
    let mut vendor = false;
    loop {
        let word_end = rest
            .find(|c: char| c.is_whitespace() || c == '_' || c == '-')
            .unwrap_or(rest.len());
        let word = &rest[..word_end];
        if !word.is_empty() && VENDOR_WORDS.iter().any(|v| word.eq_ignore_ascii_case(v)) {
            vendor = true;
            rest = rest[word_end..]
                .trim_start_matches(|c: char| c.is_whitespace() || c == '_' || c == '-');
        } else {
            return (vendor, rest);
        }
    }
}

fn find<'a>(table: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, n)| *n)
}

/// The name of the device for people.
///
/// * a model in the tables above gives its name (`leaf2` and `Onyx Scubapilot-IDP` give `Boox Leaf
///   2`, `Kobo_nova` gives `Kobo Clara HD`, `PB629` gives `PocketBook Verse`);
/// * anything else is returned as it was reported, only tidied (control characters out, white
///   space collapsed, a repeated vendor word dropped: `Onyx Onyx Foo` is `Onyx Foo`). It is never
///   guessed from a similar-looking model;
/// * an empty name stays empty (the caller shows "Unknown device").
///
/// A name the user typed (the device name field of the plugin, or a rename in Bookx) is only
/// changed when it is itself one of the codenames above, which is what the person meant.
pub fn friendly_name(reported: &str) -> String {
    let tidy_name = tidy(reported);
    if tidy_name.is_empty() {
        return String::new();
    }
    let (vendor, rest) = strip_vendor(&tidy_name);
    let key = normalise(rest);
    if !key.is_empty() {
        if let Some((_, name, how)) = ONYX.iter().find(|(k, _, _)| *k == key) {
            if *how == A || vendor {
                return (*name).to_string();
            }
        }
        for table in [KOBO, KINDLE, POCKETBOOK, REMARKABLE] {
            if let Some(name) = find(table, &key) {
                // `Kobo_nova` is not a name anybody types: the whole reported text is the key
                return name.to_string();
            }
        }
    }
    if vendor && !rest.is_empty() {
        // `Onyx Onyx Foo`, `Boox ONYX Foo`: one vendor word, as reported
        let first = tidy_name
            .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
            .next()
            .unwrap_or("");
        return format!("{first} {rest}");
    }
    tidy_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_from_issue_39_is_a_boox_leaf_2() {
        assert_eq!(friendly_name("Onyx Scubapilot-IDP"), "Boox Leaf 2");
        assert_eq!(friendly_name("Scubapilot-IDP"), "Boox Leaf 2");
        assert_eq!(friendly_name("onyx scubapilot"), "Boox Leaf 2");
    }

    #[test]
    fn koreader_on_android_sends_the_product() {
        for (reported, name) in [
            ("leaf2", "Boox Leaf 2"),
            ("Leaf2", "Boox Leaf 2"),
            ("Onyx Leaf2", "Boox Leaf 2"),
            ("ONYX BOOX Leaf2", "Boox Leaf 2"),
            ("go7", "Boox Go 7"),
            ("gocolor7", "Boox Go Color 7"),
            ("gocolor7_2", "Boox Go Color 7 (Gen II)"),
            ("go103", "Boox Go 10.3"),
            ("noteair3c", "Boox Note Air 3C"),
            ("palma2", "Boox Palma 2"),
            ("Palma2_Pro_C", "Boox Palma 2 Pro"),
            ("poke_pro", "Boox Poke Pro"),
            ("tabultrac", "Boox Tab Ultra C"),
            ("Onyx Page", "Boox Page"),
        ] {
            assert_eq!(friendly_name(reported), name, "{reported}");
        }
    }

    #[test]
    fn a_common_word_is_only_a_boox_when_the_device_said_onyx() {
        assert_eq!(friendly_name("Page"), "Page");
        assert_eq!(friendly_name("Note"), "Note");
        assert_eq!(friendly_name("Max"), "Max");
        assert_eq!(friendly_name("Onyx Max"), "Boox Max");
        assert_eq!(friendly_name("Onyx Note"), "Boox Note");
    }

    #[test]
    fn kobo_kindle_pocketbook_and_remarkable_models() {
        for (reported, name) in [
            ("Kobo_nova", "Kobo Clara HD"),
            ("Kobo_goldfinch", "Kobo Clara 2E"),
            ("Kobo_io", "Kobo Libra 2"),
            ("Kobo_monza", "Kobo Libra Colour"),
            ("Kobo_cadmus", "Kobo Sage"),
            ("Kobo_europa", "Kobo Elipsa"),
            ("Kobo_condor", "Kobo Elipsa 2E"),
            ("Kobo_snow_r2", "Kobo Aura H2O2"),
            ("Kobo_spaBW", "Kobo Clara BW"),
            ("KindlePaperWhite5", "Kindle Paperwhite 5"),
            ("KindleOasis3", "Kindle Oasis 3"),
            ("KindleScribe", "Kindle Scribe"),
            ("PB629", "PocketBook Verse"),
            ("PBInkPadColor3", "PocketBook InkPad Color 3"),
            ("reMarkable 2.0", "reMarkable 2"),
        ] {
            assert_eq!(friendly_name(reported), name, "{reported}");
        }
    }

    #[test]
    fn what_is_not_known_is_never_invented() {
        for reported in [
            "Kobo",
            "Kindle",
            "PocketBook",
            "KOReader",
            "Kobo Libra",
            "Bookx on laptop",
            "Pixel 7",
            "Kobo_neverheardof",
            "Scubapilot-X",
            "Leaf 3 Foo",
        ] {
            assert_eq!(friendly_name(reported), reported, "{reported}");
        }
    }

    #[test]
    fn an_unknown_boox_keeps_what_it_said_without_a_doubled_vendor() {
        assert_eq!(friendly_name("Onyx Foo-9"), "Onyx Foo-9");
        assert_eq!(friendly_name("Onyx Onyx Foo-9"), "Onyx Foo-9");
        assert_eq!(friendly_name("ONYX boox Foo-9"), "ONYX Foo-9");
    }

    #[test]
    fn white_space_and_control_characters_are_tidied() {
        assert_eq!(friendly_name("  Onyx\t Scubapilot-IDP \n"), "Boox Leaf 2");
        assert_eq!(friendly_name(" My \u{7} reader "), "My reader");
        assert_eq!(friendly_name(""), "");
        assert_eq!(friendly_name("   "), "");
        assert_eq!(friendly_name("Onyx"), "Onyx");
    }

    #[test]
    fn the_tables_have_no_duplicate_or_unnormalised_keys() {
        let mut seen = std::collections::HashSet::new();
        let keys = ONYX
            .iter()
            .map(|(k, _, _)| *k)
            .chain(KOBO.iter().map(|(k, _)| *k))
            .chain(KINDLE.iter().map(|(k, _)| *k))
            .chain(POCKETBOOK.iter().map(|(k, _)| *k))
            .chain(REMARKABLE.iter().map(|(k, _)| *k));
        for k in keys {
            assert_eq!(normalise(k), k, "{k} is not in normal form");
            assert!(seen.insert(k), "{k} twice");
        }
    }

    #[test]
    fn a_friendly_name_is_stable() {
        for reported in ["Onyx Scubapilot-IDP", "Kobo_nova", "Onyx Foo", "x"] {
            let once = friendly_name(reported);
            assert_eq!(friendly_name(&once), once, "{reported}");
        }
    }
}
