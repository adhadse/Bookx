# Read aloud: progress notes (enhancement 9)

Internal. Research and numbers: `docs/tts-research.md`. User docs: `wiki/docs/read-aloud.md`.

## What exists (2026-11-13)

* `crates/bookx-tts` (GTK-free): sentence splitting, chunking, language of a book, the `Engine` trait and the look-ahead
  `Session` (highlight on `Begin`, errors from the engine, skip/restart, sleep timer), `PipelineEngine` (synthesis worker
  thread + playback thread, generation counter for cancellation, gapless feeding of the sink, optional size-capped
  sentence cache), engines: `ServerSynth` (OpenAI-compatible,
  XTTS API server, Fish Speech API server), `Kokoro` and `Piper` over ONNX Runtime (`ort`, `load-dynamic`) with the system
  `espeak-ng` as phonemizer, catalogue of downloads (`models`, generated `catalog_data.rs`, pinned SHA-256), resumable
  hash-verified downloader (`download`), settings items (`items`), engine choice and fallback (`factory`), WAV export
  function (`export`, not in the UI).
* App: `src/tts.rs` (glue), `src/ui/reader_tts.rs` (popover: play/pause/stop, previous/next sentence, speed, pitch, voice,
  sleep timer, engine line, settings button), `src/ui/tts_settings.rs` (Preferences page "Text to Speech" registered through
  the registry: engine, speed, pitch, Kokoro model, cache size, test voice, downloads with licence dialog / progress /
  cancel / remove, voice server fields and connection test), reader integration in `src/ui/reader.rs`.
* Tests: 111 unit tests + 3 opt-in live tests in `crates/bookx-tts` (fake OpenAI server, fake range-capable download
  server, fake SSIP daemon, pipeline with a fake synthesizer, cache, catalogue, factory); in the app: `reader_tts` and
  `tts_settings` unit tests; UI scenarios `tts_read_aloud`, `tts_chapters`, `tts_controls`, `tts_sleep`,
  `tts_settings_page`, `tts_server` (real pipeline against `build-aux/tts-fake-server.py`); real-session scenario
  `read_aloud_flow` (`build-aux/session/scenarios_tts.py`, real pointer/keys, pixel check of the highlight).
* Regression found by the real-session screenshot: the settings page showed every group twice (a registration call
  landed in `ensure_registry`); fixed, covered by `first_registration` test, `groups=1` in `tts_settings_page` and the
  "one Voice heading" check in `read_aloud_flow`.

## Measured (this machine, see the research note for the table)

Kokoro int8 / fp32: RTF 0.10 with 8 threads, 0.24 with 1 thread; Piper de medium: 0.024. Real downloads of 132 MB and
325 MB verified against the pinned hashes. Real synthesis WAVs checked for sample rate, loudness and envelope only: **nobody
has listened to them**. First thing to do on a machine with speakers: `cargo run --release -p bookx-tts --example speak`
and compare `kokoro-int8` and `kokoro-fp32`, and judge the espeak-based English against Kokoro's own pipeline.

## Not tried on hardware / not done

* The real sink (`pacat`/`pw-cat`/`aplay`) was only run with a custom command and `aplay -D null`; the three argument lists
  are checked against the tools' `--help`, but no audible playback was performed. A real laptop (2 to 4 slow cores) was not
  available: the RTF there is an estimate.
* XTTS API server and Fish Speech API server clients are tested against fakes built from their documentation.
* Flatpak: needs `espeak-ng` (module) and `--socket=pulseaudio` plus a player (`pacat`) or a `cpal` sink. Not changed.
* `cpal` sink (no ALSA/Pulse development headers here).
* MP3/OGG export: needs an encoder, none is pure Rust. WAV only.
* Kokoro Japanese/Chinese (needs misaki G2P).
* Orpheus-3B GGUF as a future "even more natural" tier (llama.cpp + SNAC decoder).
* Speed above 2.0 is clamped inside Kokoro and Piper (the slider goes to 2.5).
* Pause, wave 4: the player takes back what the sink had buffered (kills `pacat`, as stop does) and writes the unheard
  part again on resume (`Player::drain` / `undrain`, with the event clock shifted). Tested with a simulated real-time
  sink; **not heard on a real sound card**, where `pacat`'s own start-up latency adds to the resume.
* Chapter gap, wave 4: `Engine::prefetch` makes the first two sentences of the next chapter when the last two of this one
  begin (`Reader::tts_prefetch_next`), kept in a small in-memory map that `speak` consults. Tested at engine level; the
  actual gap on hardware was not measured. A voice server slower than the remaining sentences still leaves a gap.
* Resume position across sessions is the reading position (the page follows the highlight); there is no separate stored
  sentence.
* Piper curated list is 14 voices; adding a voice means editing `build-aux/tts-catalog.py` and re-running it.
* The shared tree was broken by other agents several times during this work; the final test runs were made on a
  consistent snapshot copy of the tree (`/tmp/opencode/ws`, not part of the repository). Failures in editor, catalogue
  and goals scenarios/tests in that snapshot belong to those tracks.

## Wave 4 additions

* *Save as audio…*: popover buttons *Save Chapter / Book as Audio…* → `Cmd::SaveAudio` → `ui::reader_tts::save_audio` (file
  or folder chooser, worker thread, progress dialog with Cancel) → `tts_export::{plan,run}` →
  `bookx_tts::export::synthesize_to_wav` (streams to a `WavWriter`; the header sizes are patched at the end). The engine
  comes from `factory::open_synthesizer` (Kokoro / Piper / voice server; the basic voice gives an error that names the
  alternatives; `BOOKX_TTS_MOCK` gives a tone for the UI test). The whole book is one numbered WAV per chapter; chapter text
  for the book comes from the chapter documents, not from the canvas, so a sentence can differ from the highlighted one by
  canonicalisation (nbsp, soft hyphens).
* API key: `ServerPrefs.api_key`, stored in the keyring through `Config::secret_fields` (`tts-api-key`), field in the voice
  server group of the TTS settings; `Debug` of `ServerPrefs` never prints it.

## Wave 4: player card (enhancement 11) and first voice (enhancement 20)

* **Removed**: the speech-dispatcher engine (`speechd.rs`, the `Choice::Speechd` path, `rate_of_speed`/`pitch_of_factor`, the
  settings row, the Flatpak `xdg-run/speech-dispatcher` permission). `TtsPrefs.engine` is `kokoro` (default), `piper` or
  `server`; an old `speechd`/`auto` is migrated to `kokoro` at start (`tts::migrate_engine`, saved) and by `clamped()`. `Choice`
  has no basic voice any more: `factory::choose` returns `Err(NotReady::{NoVoice, NoEspeak, Language})` when nothing can read, and
  `open` / `open_synthesizer` fail with that error (downcast in the reader). Kokoro stays the fallback chain: Kokoro, then an
  installed Piper voice for the language, otherwise the error.
* **Player card**: `src/ui/tts_player.rs` (widget, an overlay of the page above the footer, `adw::Clamp` max 560 so it
  is narrow-window friendly, `adw::WrapBox` for the speed/voice/sleep/volume row, not a popover so a click on the page does not
  dismiss it) on the pure model `src/tts_player.rs` (phases, enabled buttons, progress <-> sentence mapping, speed steps).
  `Reader::tts_sync_player` pushes the model after every change. The seek bar jumps after the pointer rested 250 ms
  (`Session::seek`). Chapters: `Reader::tts_goto_chapter`. Volume is a gain applied to the samples in the pipeline player
  (`Engine::set_volume`, `TtsPrefs.volume`), so it works with every sink. The reader bars count the card as in use while the
  pointer is over it, the keyboard focus is in it or its voice list is open (`PlayerCard::in_use`, folded into `popover_open` of
  `chrome_busy`, bug 7 lesson). No animation (e-ink): visibility toggles only.
* **MPRIS** (`src/mpris.rs`, zbus 5 blocking, already in the tree through oo7): the player registers
  `org.mpris.MediaPlayer2.Bookx.instance<pid>` while the card is shown; Play/Pause/PlayPause/Stop/Next/Previous become
  `Control` values polled by the reader tick (Next/Previous = sentence); PlaybackStatus and Metadata (book, chapter) are
  updated. Tested against a private `dbus-daemon` in a unit test (skips itself without it) and on the UI scenarios' own
  session bus. **Not tried**: the GNOME shell media widget and a real media key (the real-session scenario drives the card
  with the pointer). Seek/SetPosition/Volume writes are accepted and ignored. `BOOKX_NO_MPRIS=1` turns it off.
* **First voice**: Read Aloud with no installed voice: first time a dialog (size, Kokoro licence note, "Always do this"); the choice
  `download_without_asking` is kept and is a switch in the settings. The download runs on a worker thread
  (`tts::download_default` -> `download::install_all`: resumable, hash-verified), the card shows *Preparing voice N%* with
  Cancel, and reading starts at the requested place as soon as it is done. Offline or any failure is classified
  (`bookx_tts::setup::classify`) and shown on the card with *Try again* (offline message names the external server). Missing
  `espeak-ng` shows the install command for the distribution from `/etc/os-release` with a copy button (none in the Flatpak,
  which ships it). Settings page: espeak row has the same copy button.
* **Test hooks**: `BOOKX_TTS_MOCK_DOWNLOAD=<ms>|offline` (fake download, marker file `.mock-voice` in the store; the mock engine
  only opens after it), `BOOKX_TTS_MOCK_ESPEAK=missing`; reader dev commands `reader:tts-player`, `tts-player-click:<what>`,
  `tts-player-seek:<0..1>`, `tts-download:yes|always`.
* **Not verified on hardware**: the real 130 MB download through the card (the download code is the one the settings page
  used and its tests); the card with a screen reader; narrow phones. Pause/Resume of a real sink as before.

## Wave 4, round 3: drag bug, voice detection bug, download speed, restore

* **Cards were hard to move (root cause).** `GestureDrag` offsets are measured in the coordinates of the widget that is
  being moved, so every move changed the next offset and the card lagged behind the pointer (measured with the real
  pointer in 60 steps of 5 px: the card moved about half as far; worst error **155 px / 160 px**, mean 77 px). Margins were also used to move
  (re-measures the overlay). Fix: `ui/card_drag.rs`: position = fractions of the free room, placed by the overlay's
  `get-child-position` (only the card is allocated); the drag translates the pointer to overlay coordinates at every event and
  subtracts the grab point (pure math `tts_player::{rect_of, frac_of, card_corner}`). A second defect found on the way: the
  clamp's own 12 px margins sat inside the rectangle the overlay gave it (a constant 12 px jump at the first move). After:
  worst error **0.0 px** at every step for both cards, page canvas **0 layouts, 0 to 1 draws** per drag (dev hook
  `work-watch`, counters in `BookCanvas::dev_work`). Frame times were not measured (draw/layout counts and tracking error were).
  Real-session scenario `cards_drag_real`.
* **"Download a voice" shown although a voice was installed (root cause).** The owner's config had
  `kokoro_model: "fp32"` while only the small (int8) model was installed; detection looked for the preferred model only, so it
  found no voice. `kokoro_paths` now accepts any complete installed model (preferred first), and packs left in
  `~/.cache/bookx-tts` (older versions, live tests) are moved into the data folder once (`models::migrate_legacy`). Detection
  stays cheap: marker written after a hash-verified download plus file sizes; full hashes only at download.
* **Download speed (measured here, real sources, 2026-11-13).** Kokoro model from Hugging Face (92 MB): 25.5 s = 3.6 MB/s with
  the old client, the same as `curl` (single stream is already at the line speed there). The ONNX Runtime from GitHub releases
  (11 MB): **8 KB/s** with the old client (about 24 min), 20 KB/s per connection with `curl`, and four parallel ranges gave
  about 4 times that (the server limits each connection). So the time went to a throttled CDN and a single connection, not to
  buffers, hashing (one pass of 0.3 s) or progress callbacks. Done: parallel `Range` segments (8 connections for files over 8 MB,
  fallback to one stream when the server has no ranges), one shared connection pool, redirects resolved once, 256 KB buffers,
  the checksum computed while the bytes arrive, a saved segment plan for resume (`<file>.part.map`), throughput and time left in
  the progress text (`setup::Throughput`). No faster mirror was found (PyPI wheel 8 KB/s here; jsDelivr refused), so the runtime is
  still slow on a throttled network; **the parallel gain was not measured on the real GitHub download after the change** (only
  against a local server with a per-connection delay: 2 MB in 4 segments took less than 0.6 of the single-stream time, and curl
  showed the 4x on the real server). HTTP/2 is not available in `ureq`.
* **Voice download is an application job** (`src/voice_download.rs`): worker thread plus a main-loop timer, listed in the Background
  jobs panel (kind *Voice download*: percent, speed, time left, log, Cancel). It survives the reader; a *Voice ready* toast ends it.
  The settings page rows follow the same job; *Resume* appears there when `.part` files exist (also after a restart).
* **Cards: compact default, own X, independent, restored.** `src/reader_cards.rs` (pure: per-book `BookCards`, round trip,
  clamping, chapter resolution by path then index, offset beyond the end) stored in `Config.cards`; the reader saves on every
  state change (forced) and every 1.5 s while moving, and restores 0.5 s after the first page (`Input::RestoreCards`): cards parked
  (`Phase::Idle`, `rsvp::Parked`), the page at the card on top. Leaving a chapter parks the cards instead of closing them.
  Scenarios: `cards_restore*`, `cards_closed_stay_closed`, real-session `cards_restore_real` (real drags and clicks, library round
  trip, process restart, X).
* **Not done / not verified**: window sizes that differ between sessions are covered by the fraction math only (unit tests), not by a
  session test; the sentence text itself is not compared in the real session (AT-SPI hides it), the sentence number is.
  A real media key / shell widget, a touch screen and screen readers were not tried.

## Reader focus while playing (wave 4, round 4)

* Not a new mode: the mockup's "1.b Reader focus" (the chrome hidden while reading, shown on interaction) is the existing bar fade.
  Playing is now an input: `chrome_visibility::{fade_now, idle_delay_ms}` (1.5 s playing, 4 s otherwise, pure, unit tests with an
  injected clock). `ChromeCtl` in `reader.rs` holds the playing flag, the pointer height and the time of the last interaction; the
  existing `chrome_busy` still decides what counts as use. While playing, the fade also hides the title and the status line (CSS
  `chrome-playing`), so only the page and the cards are left, as in the mockup.
* Removed again (the owner's clarification): the focus mode toggle (header button, menu entry, palette command, Shift+F11), the
  focus buttons on the cards, the layout changes and the focus state. No config or translation strings remain.
* **Defect found by the real session**: a resting pointer still counted as an interaction, because GTK repeats the last motion
  event whenever what is under the pointer changes (the highlight moves, a card redraws), so the bars never faded while reading.
  Only a pointer that really moved (other coordinates) is an interaction now. Also by design: after closing the read-aloud popover
  with Esc the header button keeps the keyboard focus ring and the bars stay (keyboard focus in a bar), same as before.
* Mockup check (`mockups/src/bookx-reader.svg`, "1.b Reader focus"): the focus state is the bare page. The non-focus bottom panel
  there shows "63 of 807" on the left and "Exploring the System   8 minutes to complete" on the right; Bookx shows "9 of 185" on the
  left and "CHAPTER I. Down the Rabbit-Hole · 7 minutes to complete" on the right with the Standard footer preset (same fields,
  the time to finish is written "N minutes to complete", as in the mockup). While the bars are faded for non-playing reading, Bookx
  keeps the title and a centred status line (existing behaviour, unchanged); only while playing is nothing left.
* Tests: `chrome_visibility` unit tests, Broadway `reader_focus_playing`, `reader_focus_speed_reading` (dev hooks `chrome-clock:<ms>`,
  `chrome-touch`, `chrome-pointer:<y>`), real session `reader_focus_real` (real pointer, screenshots `focus-bars-start`, `focus-away`).

## ONNX Runtime origins (wave 4, round 5)

* **Measured on this machine (2026-11-13), through the app's own downloader** (`cargo run --release -p bookx-tts --example dl_bench`,
  `BENCH_ONLY=1`, `BENCH_ORIGIN=github` for the old origin): the runtime from the GitHub release alone took **52.2 s** (0.2 MB/s; with
  the new segmented downloader; the old single-stream client was at 8 KB/s, about 24 min for 11 MB, measured earlier); from the NuGet origin
  **5.7 s** (2.0 MB/s). The owner's own `curl` numbers were 0.72 MB/s (GitHub) and 4.6 MB/s (conda CDN).
* **conda-forge was looked at and not used.** The package named in the request, `onnxruntime-1.31.1-py312...conda`, is the *Python*
  package: its biggest member is `onnxruntime_pybind11_state.so`, there is no `libonnxruntime.so` in it. The package that has the C library
  is `onnxruntime-cpp` (8.57 MB for linux-64, build `h49c692b_2_cpu`; the smallest linux-aarch64 is 7.99 MB), but its library is linked
  against conda's own `libprotobuf.so.35.1.0`, `libabsl_*.so.2608.0.0` and `libre2.so.11` (checked with `readelf -d`: `NEEDED`
  entries that exist only as other conda packages, versions that do not match a normal Linux system), so it does not load outside a conda
  environment. Making it work would mean downloading and unpacking three more conda packages plus a zstd decoder and trusting their ABI
  together; not done. No new dependency (`ruzstd`) was added for it.
* **What was done instead: NuGet.** `Microsoft.ML.OnnxRuntime` 1.31.1 (a 160 MB zip for all platforms on a fast CDN that does Range requests)
  holds the *same official* `libonnxruntime.so` (same SHA-256 as the GitHub archive's, checked by the catalogue script) as
  `runtimes/linux-x64/native/libonnxruntime.so` (and `linux-arm64`). Bookx reads the zip's directory with Range requests (the `zip`
  crate on a small `RangeReader`, 1 to 3 requests of 256 KB), downloads only that member's 11 MB (parallel segments, resumable) and inflates it
  while hashing; the pinned SHA-256 and size of the *extracted* library and the zip's CRC decide. GitHub stays as the fallback.
* **Origins** (`models::Origin`, `FileSpec::fallbacks`, generic, so the Hugging Face models can get mirrors the same way): tried in order;
  an origin that has another behind it must connect within 8 s, answer within 15 s and keep 100 KB/s over 5 s (`Tuning::with_fallback_behind`,
  `Watchdog`), else it is dropped for the next (the last is waited for however slow). A partial download is kept only when it belongs to the
  origin that is tried (`claim_part`: same hash, or same URL and member). The job log says "Switching to the next origin ...".
* **Pins**: `ORT_LIB_*` (hash and size of the extracted library, one for both origins), written into the install marker (detection stays
  cheap: marker plus size; markers of earlier versions with the archive hash are still accepted). `.tgz` extraction takes regular files only,
  refuses `..`/absolute paths, refuses a member of another size than pinned before writing it, and deletes a library whose hash differs.
  A library placed by hand in the folder with the pinned hash is used without any download (offline install).
* The library from the NuGet origin was loaded and used by the real Kokoro live test (`BOOKX_TTS_LIVE=1`, three books in a row) on this
  machine (x86_64). **aarch64 was not run** (no such machine); its member and hash were pinned from the same package.
* Script: `python3 build-aux/tts-catalog.py --only-ort` rewrites only the runtime lines (it asserts that NuGet and GitHub ship identical
  libraries); the models keep their pinned revision.

## Card layout manager (wave 4, round 6)

* `src/card_layout.rs` is pure geometry (no GTK, 12 unit tests): stored positions are *wishes* (fractions of the free room, as
  before; `None` when equal to the card's default place), `layout()` turns wishes + popover rectangles into final rectangles.
  Unmoved cards: both open and nobody moved = side by side, gap 12 px, centred as a group at the bottom, stacked when too wide.
  Placement order = rank (dragged card, then the one moved last, then the other): the first keeps its wish, the next goes to the
  nearest free spot (beside, level with the blocker, above, below; coarse-grid least-overlap fallback in tiny windows). Touching is
  not overlapping; a gap of 12 px is kept, so the result is stable (idempotent, no jitter) and a card returns to its wish when the
  obstacle goes, because everything is recomputed from the wishes at every allocation.
* `src/ui/card_drag.rs` owns the manager (`install`, `sync`, `avoid(popover)`): runs inside `get-child-position`, so it costs only the
  cards' allocation. Popovers registered with `avoid()` (selection popover with its dictionary/Wikipedia/note pages, footnote popup)
  count as obstacles; their rectangle is *estimated* (child size + 16 px on the side the popover opens, flipped when it would leave the
  overlay, as GTK does), because GTK4 gives no popover geometry in the toplevel's coordinates.
* Header and footer are outside the overlay (ToolbarView does not draw content under them), so cards cannot overlap the chrome; the
  layout still supports obstacle rectangles for bars if that ever changes (unit-tested).
* Slides: 200 ms ease-out cubic driven by a 16 ms `glib` timer, **not** `adw::TimedAnimation`: the frame clock does not tick in the
  headless Broadway tests (an `adw` animation never advanced there). Instant in e-ink mode, with `gtk-enable-animations` off, or
  before the overlay is mapped; the user's own drag is never animated (`dragging` card is rank 0 and `shown = target`).
* Not verified: popover rectangle estimates against GTK's real popover placement on Wayland (only that cards clear the estimate and
  the real session screenshots look right); touch dragging; reduced-motion through the GNOME accessibility setting (it maps to
  `gtk-enable-animations`, read at every slide start).
* Tests: unit tests in `card_layout.rs`; Broadway `cards_layout_side_by_side`, `cards_layout_yield`, `cards_layout_popover`,
  `cards_layout_restore`, `cards_layout_eink` (dev hook `reader:cards-rects`, `build-aux/cards_geom.py`); real session
  `cards_no_overlap_real` (real drags, screenshots `cards-side-by-side`, `cards-dragged-over`, `cards-restored`). `tts_multi_book` is
  timing sensitive when all 47 scenarios run at once (failed once in 3 full runs, passes alone 3 of 3).
