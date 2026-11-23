# Read aloud: model research (enhancement 9)

Internal note. Written 2026-11-12/13, sources at the end. The owner's ask: the speech-dispatcher
voice is robotic; use a human-sounding model (XTTS v2 or "Fish Speech v5"), implement the
inference ourselves if there is no Rust crate, but ship nothing that is not allowed.

Status: finished, pending move to archive/ (still pointed to from enhancements.md or source comments) | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## Contents

- [1. Short answer](#1-short-answer)
- [2. "XTTS v2" and "Fish Speech v5": what exists](#2-xtts-v2-and-fish-speech-v5-what-exists)
- [3. Candidate models](#3-candidate-models)
- [4. Rust inference and audio](#4-rust-inference-and-audio)
- [5. Why XTTS / Fish Speech are not embedded](#5-why-xtts--fish-speech-are-not-embedded)
- [6. Measurements on this machine](#6-measurements-on-this-machine)
- [7. Open points / not done](#7-open-points--not-done)
- [Sources](#sources)

## 1. Short answer

| Rank | Choice | Why |
|---|---|---|
| 1 | **Kokoro-82M through ONNX Runtime, in the app** | Apache-2.0 weights, 82 M parameters, 92 MB (int8) or 326 MB (fp32) download, 8 languages (6 usable here), 54 voices, **measured 8 to 10 times faster than real time on one desktop CPU, 4 times on a single thread**. No GPU, no Python. Shipped as the default natural voice. |
| 2 | **Piper voices through ONNX Runtime, in the app** | MIT-era VITS models, 63 MB per voice, runs at 40x real time, 40+ languages (German, Dutch, Polish, Russian, Czech, Turkish ... which Kokoro lacks). Voice quality is clearly below Kokoro. Per-voice dataset licences (shown before download; one of the 14 curated voices is non-commercial). |
| 3 | **External server (OpenAI-compatible `/v1/audio/speech`, XTTS API server, Fish Speech API server)** | The only way to the best quality (XTTS v2, Fish Audio S2) without Bookx shipping Python or a licence it cannot honour. The user runs the server (docker / uv), Bookx talks HTTP. The weights' licences (XTTS: Coqui Public Model License; Fish S2-Pro: Fish Audio Research License) are non-commercial: acceptable for a person reading their own books, stated in the settings page and the wiki. |
| 4 | speech-dispatcher (kept, **removed in wave 4**, enhancement 20) | Zero download, robotic. It was the fallback when nothing else was ready; now Read Aloud offers to download Kokoro in the background instead, and the external server stays selectable. |

Not done: running XTTS v2 or Fish Speech inside Bookx (section 5).

## 2. "XTTS v2" and "Fish Speech v5": what exists

* **XTTS v2** (Coqui, 2023): GPT-style autoregressive model + HiFi-GAN decoder, 17 languages, voice
  cloning from 6 s of audio, about 1.8 GB of weights. Licence: **Coqui Public Model License 1.0.0**,
  "allows only non-commercial use of a machine learning model and its outputs". Coqui shut down in
  December 2023, nobody can sell you a commercial licence. The code (`coqui-tts`, fork by Idiap) is
  MPL-2.0. Needs PyTorch; real time needs a GPU (CPU is slower than real time); known to produce
  glitches/hallucinations on long inputs (chunk to sentences: which the reader does).
* **Fish Speech**: there is **no "v5"**. Releases on GitHub: v1.4 (Sept 2024), v1.5 (Dec 2024), v1.5.1
  (May 2025, the last tagged one), then the company renamed the models: **OpenAudio S1 / S1-mini**
  (2025), **Fish Audio S2 / S2-Pro** (4 B parameters, Qwen3-4B backbone, 80+ languages,
  Seed-TTS WER 0.54 % zh / 0.99 % en, 2026) and **Drama 3** (35B-A3B MoE, Oct 2026). Licences:
  v1.x weights CC-BY-NC-SA-4.0; S2-Pro weights **Fish Audio Research License** (research and
  non-commercial use free); repository code under the Fish Audio Community License. All
  non-commercial for the weights. Needs PyTorch + a GPU with roughly 12 to 24 GB for S2-Pro; the
  project provides an SGLang-based streaming server. The API server (`tools/api_server.py`,
  `POST /v1/tts`, JSON or msgpack, `format: wav`) is what Bookx talks to.
* Consequence: both are *model weights with non-commercial licences and a Python/GPU stack*. They
  fit "user runs a server", not "Bookx downloads and embeds the model".

## 3. Candidate models

Speed numbers: "RTF" = synthesis time / audio time (lower is better). Only the Kokoro and Piper rows
are measured by us (section 6); the rest is from the sources and our reading of them.

| Model | Weights licence | Size | Languages | Quality (subjective, from public rankings) | Speed | Streaming | ONNX/GGUF | Verdict |
|---|---|---|---|---|---|---|---|---|
| **Kokoro-82M v1.0** | Apache-2.0 | 82 M (92 MB int8 / 326 MB fp32 ONNX) | en (US/GB), es, fr, hi, it, pt-br, ja, zh | Artificial Analysis Elo 1064 (Nov 2026), level with NVIDIA Magpie, above Amazon Polly generative voices | **RTF 0.10 on 8 threads, 0.24 on 1 thread** (Ryzen 9 9950X) | per sentence (we chunk) | ONNX official export (`onnx-community`), no GGUF | **Chosen** |
| **Piper** (VITS) | engine MIT (old repo) / GPL-3 (piper1-gpl); voices per dataset licence (CC0, CC-BY, CC-BY-NC-SA, "Blizzard", ...) | 20 to 120 MB per voice | 40+ locales | clearly synthetic, pleasant, good for long listening | RTF 0.024 (de, medium, warm) | per sentence | ONNX | **Chosen as light / multilingual option** |
| StyleTTS 2 | MIT code; weights MIT, asks to disclose synthetic speech | ~190 MB | en (LJSpeech/LibriTTS) | very high for en | CPU 0.3 to 1 | no | PyTorch; community ONNX | Kokoro is its distilled descendant; skip |
| F5-TTS | code MIT, weights **CC-BY-NC** | 1.2 GB | en, zh (+community) | high, cloning | GPU wanted | chunked | ONNX ports exist | non-commercial; server only |
| Parler-TTS (mini/large) | Apache-2.0 | 880 M / 2.2 B | en | good, controllable by text prompt | slow on CPU (RTF > 1) | no | PyTorch (candle example exists) | too slow on CPU |
| OpenVoice v2 | MIT | tone converter ~130 MB (+ base TTS MeloTTS, MIT) | 6 | voice conversion, not a TTS alone | fast | no | PyTorch | not a drop-in |
| Orpheus 3B | Apache-2.0 on top of Llama 3.2 licence | 3 B (GGUF Q4 ~2 GB) | en (+research) | very natural, emotive tags | needs GPU for RTF < 1; CPU Q4 ~ 0.5 to 1.5 on big CPUs | yes (SNAC codec) | GGUF (llama.cpp) + SNAC | strong candidate **later**; needs llama.cpp + SNAC decoder |
| Chatterbox (+Turbo) | MIT | 500 M / 350 M | en; multilingual 23 | high; cloning; watermark (Perth) | GPU wanted | yes | ONNX exports exist | retired by Resemble in Aug 2026 (no maintenance); GPU; skip |
| Sesame CSM-1B | Apache-2.0 | 1 B (Llama backbone) | en | conversational | GPU | yes | PyTorch | skip (conversation oriented) |
| Dia 1.6B | Apache-2.0 | 1.6 B | en | dialogue, nonverbal tags | ~10 GB VRAM | no | PyTorch | skip |
| Qwen3-TTS 0.6B/1.7B | Apache-2.0 | 0.6 to 1.7 B | 10 | high, voice design | GPU wanted | yes | PyTorch | watch; no ONNX/GGUF yet that we verified |
| Magpie TTS Multilingual (NVIDIA) | NVIDIA Open Model License | 357 M | 12 | = Kokoro on the leaderboard | NeMo | | | NeMo/PyTorch only |
| XTTS v2 | **CPML (non-commercial)** | 1.8 GB | 17 | high, cloning, can glitch | GPU | yes | PyTorch (ONNX export is community, partial) | server only |
| Fish Audio S2-Pro | **Research licence** | 4 B | 80+ | best open model on the Humanness/AA lists | 12+ GB GPU | yes | PyTorch/SGLang | server only |

Take-away: nothing between "82 M parameters, runs anywhere" and "multi-billion parameter GPU model"
has both a permissive licence and a CPU-friendly runtime today, except Orpheus-GGUF (future work).

## 4. Rust inference and audio

| Need | Options looked at | Decision |
|---|---|---|
| Run ONNX models | **`ort` 2.0.0-rc.15** (bindings to Microsoft's ONNX Runtime, MIT/Apache-2.0; Rust ≥ 1.92); `tract-onnx` 0.23 (pure Rust, but incomplete operator coverage; Kokoro/StyleTTS ops (STFT, LSTM variants) are a risk and it is slower); `candle` 0.11 (no Kokoro/Piper port in the core; `any-tts` crate wraps Candle backends for newer models: not evaluated); `burn-onnx` (import-time code generation, same coverage risk); `sherpa-rs` (bindings to sherpa-onnx, Apache-2.0, runs Kokoro and Piper) | **`ort` with the `load-dynamic` feature**: the build links nothing and downloads nothing; the user downloads `libonnxruntime.so` (11 MB, MIT, pinned by hash) next to the models. Works first time on Kokoro and Piper. |
| Existing Rust TTS crates | `piper-rs` 0.2 (uses `espeak-rs`, which **bundles and links espeak-ng, GPL-3**), `kokoro-tiny`/`kokoro-micro`/`kokoro-en` (small, young, bundle model downloads and espeak) | Not used. Our own 300 lines drive the two models and keep the dependency graph and licence story ours. |
| Text to phonemes | `espeak-ng` (GPL-3.0-or-later, ~125 languages); `espeak-rs` (bundled espeak-ng); pure-Rust phonemizers: none that produces the espeak IPA flavour Kokoro and Piper were trained on at usable quality (CMUdict+rules for English only, would change the sound) | **Run the system `espeak-ng` program** (`-q --ipa -v LANG`), 3 ms per call. Bookx is GPL-3.0-or-later, espeak-ng is GPL-3.0-or-later: no licence conflict even if it were linked; running it as a separate program avoids bundling it at all. Not installed → the engine says so and falls back. Flatpak: needs espeak-ng in the manifest (TODO, see section 7). |
| Audio out | `cpal` 0.18 (needs ALSA/PulseAudio development headers: not present here, so not buildable/testable in this environment), `rodio` 0.23 (on top of cpal), `symphonia` (decoding only) | **Pipe raw s16le into `pacat` / `pw-cat` / `aplay`**, fed at the speed of playback, killed on stop. No native build dependency, exact stop, works with the PulseAudio socket of a Flatpak if the runtime ships the tool. A `cpal` sink behind the same `AudioSink` trait is a follow-up. |
| Output files | pure-Rust MP3/OGG encoders exist (`mp3lame-encoder` binds C; `vorbis_rs` binds C; `opus` binds C); WAV is trivial | WAV only. See section 7. |

How other apps do it (from memory, not re-checked): Foliate and Calibre call speech-dispatcher; Thorium Reader uses Web Speech;
KOReader uses espeak/Android TTS; Readest and Speechify-likes call cloud APIs; Home Assistant runs
Piper as a service; `sherpa-onnx` is what most "offline neural TTS in an app" projects (Next-gen
Kaldi, several Android readers) embed; Open WebUI and SillyTavern call OpenAI-compatible servers
(Kokoro-FastAPI, openedai-speech, XTTS API server). Bookx now does both of the last two ways.

## 5. Why XTTS / Fish Speech are not embedded

1. Licences: non-commercial weights cannot be bundled or downloaded by default into software others
   may redistribute (Flatpak, distros); the user can run them for themselves.
2. Runtime: both need PyTorch and a GPU; there is no maintained ONNX/GGUF path with equal quality
   (XTTS ONNX exports are partial community work; Fish S2 is a 4 B transformer + codec). Writing a
   Candle port of a 4 B model with a custom codec is weeks of work and CPU would not be real time.
3. Bundling Python is out of scope (the repo rule is Rust application code).

What we did instead: the **voice server engine** (documented docker/uv commands) speaking the
OpenAI audio API, the XTTS API server (`/tts_to_audio/`, `/speakers_list`) and the Fish API server
(`/v1/tts`).

## 6. Measurements on this machine

AMD Ryzen 9 9950X (16 cores, 32 threads), ONNX Runtime 1.31.1 CPU, release build, text of 14.4 s of
speech (3 sentences), second run (warm). Written by `cargo run --release -p bookx-tts --example speak`.

| Engine | Threads | Audio | Synthesis | RTF |
|---|---|---|---|---|
| Kokoro int8 (af_heart, en) | 8 (default) | 14.47 s | 1.47 s | **0.10** |
| Kokoro int8 | 4 | 14.47 s | 1.68 s | 0.12 |
| Kokoro int8 | 2 | 14.47 s | 2.28 s | 0.16 |
| Kokoro int8 | 1 | 14.47 s | 3.53 s | 0.24 |
| Kokoro fp32 (326 MB) | 8 | 14.40 s | 1.43 s | **0.10** (same speed as int8) |
| Kokoro int8, bm_george, speed 1.5 | 8 | 9.91 s | 0.90 s | 0.09 |
| Kokoro int8, French, one short sentence | 8 | 3.33 s | 0.82 s | 0.25 (short input: fixed costs dominate) |
| Piper de_DE-thorsten-medium | 8 | 7.12 s | 0.17 s | **0.024** (first run 0.124: model load/optimizer warm-up) |

* Model load: Kokoro 0.4 s, Piper 0.01 to 0.1 s (done in the worker thread, not on the UI thread).
* A laptop with 2 to 4 slow cores should be 2 to 3 times slower than the 1-thread line (RTF 0.5 to
  0.8): still real time, which is all gapless playback with look-ahead needs. Not measured by us on
  such a laptop.
* The audio is **not listened to** (the agent has no ears). Checked instead: sample rate 24000 /
  22050 Hz as the models declare, RMS 0.07 to 0.11, peak 0.34 to 0.94 (no clipping, not silent),
  speech-like envelope (80 % of 20 ms frames above the noise floor, six pauses of 0.2 to 0.54 s at
  the commas and sentence ends of the test text, about 3700 zero crossings per second as voiced
  speech has), duration matching the text (44 words in 14.5 s ≈ 180 words per minute).
  The int8 model peaks higher than fp32 (0.94 vs 0.71); a human should judge which they prefer.
* Download: 132 MB (ONNX Runtime + 54 voices + int8 model) took 163 s on the first try, 80 s on the
  second (325 MB fp32) on this connection; hashes verified; resume tested against a local server.

## 7. Open points / not done

* **Pure-Rust-free parts**: ONNX Runtime (downloaded, C++), espeak-ng (system program, C).
* **Flatpak**: the manifest has no espeak-ng and pacat; add the `espeak-ng` module and
  `--socket=pulseaudio` before shipping natural voices in the Flatpak. Not done (no way to build
  the Flatpak here).
* **Kokoro Japanese / Chinese** need misaki's grapheme-to-phoneme (Python/dictionaries); hidden in
  the UI. Hindi/Italian/Spanish/French/Portuguese go through espeak-ng as Kokoro's own pipeline does
  (its English path uses misaki + espeak fallback, so our English output may differ slightly from
  the reference implementation in a few words).
* **No number/abbreviation normalisation** beyond what espeak-ng does ("3.5" is read by espeak).
* **MP3/OGG export**: no pure-Rust encoder is available; only WAV is written (`Pcm::to_wav`);
  `ffmpeg` can convert. Reader export button not added.
* **Orpheus-GGUF** (best open permissive quality) would need llama.cpp bindings and the SNAC
  decoder; candidate for a later "Natural+" tier.
* **cpal sink**.
* **Pause** keeps the 100 to 350 ms already buffered in the sound server audible.
* **API key** for servers: only `$BOOKX_TTS_API_KEY`; the new `bookx-secrets` crate could hold it.
* **Fish Speech API** is built from its documentation and a third-party example (JSON body, WAV
  answer); not run against a real server here (no GPU).

## Sources

* Fish Speech repository and site (releases, licences): github.com/fishaudio/fish-speech,
  speech.fish.audio, docs.fish.audio/features/text-to-speech, huggingface.co/fishaudio/fish-speech-1
* Cartesia "Best open-source TTS models, compared by license" (Oct 2026), Modal and BentoML TTS
  roundups, Kokoro benchmark posts (heyneo.com, localaimaster.com)
* huggingface.co/hexgrad/Kokoro-82M, huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX,
  huggingface.co/rhasspy/piper-voices (model cards), huggingface.co/coqui/XTTS-v2
* crates.io: ort, tract-onnx, candle-core, burn-import, sherpa-rs, piper-rs, espeak-rs, kokoro-*, cpal, rodio
* github.com/microsoft/onnxruntime releases (v1.31.1), ort.pyke.io, daswer123/xtts-api-server
