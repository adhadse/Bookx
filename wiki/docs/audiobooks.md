# Audiobooks

Bookx keeps audiobooks in the library next to your e-books and plays them itself: chapters, speed, a sleep timer,
bookmarks with notes, and media keys. It plays DRM-free audio only; protected audiobooks (Audible's AAX and the like)
are not supported and never will be.

## Adding an audiobook

Click the **+** button at the top of the sidebar and choose **Add Audiobook**. It asks for a folder or a file:

| You have | Choose | What Bookx does |
|---|---|---|
| A folder of chapters (`01 - The Gate.mp3`, `02 - …`) | **Choose Folder** | Reads every audio file in it (also in sub-folders such as `CD1`, `CD2`), puts them in natural order (`Chapter 2` before `Chapter 10`) and makes one book |
| A single `.m4b` (or one long `.mp3`) | **Choose File** | Reads its chapters from the file |
| A `.zip` of chapters | **Choose File** | Unpacks the audio into Bookx's data folder (`~/.config/bookx/audiobooks/unpacked`) and makes one book |

Dragging a folder of audio, or an audio file or zip, onto the library window does the same.

The audio **stays where it is**: the library keeps a link, like the *Linked* folder layout. If you move the folder, open
the book's details and the audio shows as missing; move it back, or add it again. (Zips are the exception: their audio
is unpacked into Bookx's own folder.)

Formats: **MP3, M4B and M4A (AAC and ALAC), OGG Vorbis, FLAC, WAV**. **Opus** files are recognised and listed, but Bookx
cannot play them yet; the player says so. Files that are not audio (or are damaged) are skipped.

What Bookx reads from the files (tags, no network):

* title, author, **narrator** (the narrator or composer tag), series and series number, language, description;
* **chapters**: the chapter marks of an M4B (Nero `chpl` atoms and QuickTime chapter tracks), ID3 chapter frames of an
  MP3, `CHAPTERnnn` comments of an Ogg or FLAC file; without marks every file is one chapter;
* the **cover**: the picture inside the file, else a `cover.jpg`/`folder.png` in the folder.

The book in the library has a small headphones badge on its cover, the tag *Audiobook*, the format (`m4b`, `mp3`, …)
as a format of the book, and the length and narrator in the details panel. An audiobook is also a normal catalogue
entry: you can edit its metadata, add it to shelves, fetch metadata, and so on.

## Finding audiobooks

* The sidebar has an **Audiobooks** entry (it appears when you have one) with the number of books.
* `audiobook:yes` in the search box finds them (and e-books that have a linked audiobook, see below).
* `format:m4b`, `format:mp3` and so on find them by format; the *Formats* group of the sidebar lists the formats too.

## The player

<figure markdown="1">
![The audiobook player](assets/audiobook-player-light.png#only-light)
![The audiobook player](assets/audiobook-player-dark.png#only-dark)
<figcaption markdown="1">The player: chapter, position, −15 and +30 buttons, speed, sleep timer and volume.</figcaption>
</figure>


Open an audiobook (double-click, **Read**, or **Play in Bookx** in the details panel) and the player opens over the
library. Press the back arrow to return to the library: playback stops and your place is saved.

* the **cover**, the title, the author and narrator, the current **chapter** and *Chapter 3 of 12*;
* a **seek bar** for the whole book with the time listened and the time left;
* a line like *42 % · 3 h 12 min left* (the time left follows the speed you chose);
* **previous chapter**, **back 15 s**, **play/pause**, **forward 30 s**, **next chapter**. Previous chapter goes to the start
  of the chapter you are in, or to the chapter before when you are in its first three seconds;
* **speed** from 0.5× to 3× with presets; the pitch of the voice stays the same;
* a **sleep timer**: off, 5, 10, 15, 30, 45 minutes, 1 hour, or **end of chapter**. The remaining time shows next to the
  clock button. When it ends, playback stops (the end-of-chapter timer stops exactly at the chapter's end);
* a **volume** slider;
* **Chapters** (list icon in the header): all chapters with their length; pick one to jump there;
* **Bookmarks** (bookmark icon): type an optional note and press *Add* to keep the current position; pick a bookmark to
  jump to it, or delete it.

### Keyboard and media keys

| Key | Action |
|---|---|
| ++space++ | play / pause |
| ++left++ / ++right++ | back 15 s / forward 15 s (with ++shift++ 30 s) |
| ++page-up++ / ++page-down++ | previous / next chapter |

Bookx registers as a media player with the desktop (MPRIS): the **play/pause**, **next** and **previous** keys of your
keyboard, the GNOME shell media controls and `playerctl` work while the player is open. Next and previous move by
chapter.

### Resuming

Bookx remembers the place, the speed and the bookmarks of every audiobook. Opening a book again continues about two
seconds before where you stopped, at the speed you used. A book that was played to the end starts again from the
beginning. The place is also the *progress* of the book in the library: the percentage under the cover, *Continue
reading* on the home page, and the *Read* status of the book (finished at 98 %).

The saved state is in `~/.config/bookx/audiobooks/` (one small file per book).

## Listening statistics

Click the **Library** button in the header bar (next to *View options*) and choose *Reading Statistics*; its **Listening** group shows time listened today, in the last 7 days and in total, the audiobooks
finished this year, and the audiobooks you listened to most. Listening is counted by the clock (an hour at 2× is an
hour); it is kept apart from reading time.

## An audiobook for an e-book you own

An audiobook that is linked to a book (**Library** menu → *Audiobooks*, see [the library](library.md#audiobooks)) also plays in
Bookx: select the book and use **Play in Bookx** in the details panel. Its place and bookmarks are kept separately from
your reading place in the e-book.

The details panel also has **Continue in Audiobook**, which starts the audio at the place where you stopped reading, and
**Continue Reading**, which opens the e-book where you stopped listening. Chapters with the same title in the book and
the audio line the two up; elsewhere Bookx uses the same share of the text and of the audio, so the place can be off by a
few paragraphs or minutes. The two places are not kept in step by themselves: choose the row when you switch.

## What it does not do

* No protected audiobooks, no Audible account, no streaming.
* *Send to device* and *Export* of an audiobook use the small generated page that stands for it in the library, not the
  audio; copy the audio files yourself.
* No Opus playback yet.
* No sync of the listening place to other devices or a server (the reading sync of Bookx is for e-books).
* The sleep timer does not fade the volume out; the sound stops.
* Sound goes through `pacat` (PulseAudio/PipeWire), `pw-cat` or `aplay`, like read aloud does. If none is installed, the
  player shows an error when you press play.

## Questions and answers

??? question "How do I add an audiobook?"
    Click **+** at the top of the sidebar → *Add Audiobook*, then *Choose Folder* (chapters as separate files) or
    *Choose File* (an `.m4b`, a long `.mp3` or a `.zip`). You can also drag the folder or file onto the library.

??? question "How do I play it?"
    Double-click the audiobook, or select it and press *Read* (for a linked e-book, *Play in Bookx* in the details
    panel). The back arrow returns to the library and saves your place.

??? question "How do I set a sleep timer or change the speed?"
    Both are in the player: the speed button (0.5× to 3×) and the clock button (5 minutes to 1 hour, or end of chapter).

??? question "How do I add a bookmark?"
    Open the bookmark icon in the player's header, type an optional note and press *Add*. Pick a bookmark to jump back.

??? question "I already have the audiobook of an e-book. How do I connect them?"
    **Library** menu (next to *View options*) → *Audiobooks*. Bookx pairs audio it finds next to your books; untick wrong
    pairs and press *Link Selected*.

??? question "How do I find only the audiobooks?"
    Click **Audiobooks** in the sidebar or type `audiobook:yes` in the search bar.

??? question "Can I send an audiobook to my e-reader?"
    No: an e-reader receives only the small page that stands for it. Copy the audio files yourself.
