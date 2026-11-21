# MCP (AI assistants)

Bookx can talk to AI assistants such as Claude Desktop, Claude Code, Cursor or opencode through the
[Model Context Protocol](https://modelcontextprotocol.io) (MCP). Once connected, you can ask things like
*“Which of my books mention the Silk Road?”*, *“Summarise chapter 3 of Dune”*, *“Turn my highlights in
this book into study notes”* or *“Suggest what to read next”*, and the assistant looks at your real library.

Nothing leaves your computer unless your assistant sends it to its own service: when you ask a question,
the assistant (not Bookx) decides what it sends to its provider. The excerpts it reads from your books
become part of that conversation. Use a local model if that matters to you.

## Two ways to connect

| | Command | Server inside Bookx |
|---|---|---|
| How it works | The assistant starts `bookx-cli mcp` itself | Bookx runs a small local server while it is open |
| Bookx has to be open | No | Yes |
| Can change the library | Only with `--allow-write`, and not while Bookx is open | If you switch it on in Preferences |
| Where to set it up | Your assistant's configuration | Preferences → MCP |

The **command** way is the simplest: it is read-only and works whether Bookx is open or not.

## Setting up from Preferences

<figure markdown="1">
![Preferences, MCP](assets/preferences-mcp-light.png#only-light)
![Preferences, MCP](assets/preferences-mcp-dark.png#only-dark)
<figcaption markdown="1">Preferences → MCP: the switch, the port, the token and the write permission.</figcaption>
</figure>


The MCP page is the last one in the Preferences list (main menu → **Preferences**, or ++ctrl+comma++). It has four
groups, top to bottom:

1. **Server inside Bookx** (off by default). Its first row shows whether the server is *Off* or *Listening* and on
   which address. Switch on **Let AI assistants connect** to start a server on this computer. It listens on `127.0.0.1`
   only, on the **Port** shown (8765 unless you change it; the port cannot be changed while the server runs, so switch
   the server off first), and every request needs the **access token**. The address can be copied once the server runs.
   Just below, **Allow changes to the library** (off by default) decides whether assistants may change anything; see
   [What assistants can change](#what-assistants-can-change). Switching it on asks for confirmation.
2. **Access token.** Bookx makes a random token the first time and keeps it in the system keyring (the
   page says so; without a keyring it is kept in Bookx's settings file). The row shows it hidden. The first button
   copies the token, the second, **Make a new token**, asks first, because every assistant that has the old token stops
   working until you paste the new settings into it; then it replaces the token (the server restarts) and says
   *New token made*.
3. **Connect an assistant.** For Claude Desktop, Claude Code, Cursor and opencode there are two buttons:
   **Command** copies the configuration that starts `bookx-cli`, **Server** copies the configuration for the
   server inside Bookx, token included. Paste it where the assistant keeps its MCP settings; Bookx confirms with
   *Copied the Claude Code command setting* (or *server setting*).
4. **Connected assistants** has one row, *Recent activity*, that shows which assistants talked to Bookx recently and how
   often.

### Claude Code

```bash
claude mcp add bookx -- /path/to/bookx-cli --library "$HOME/Documents/Bookx Library" mcp
# or, with the server inside Bookx running:
claude mcp add --transport http bookx http://127.0.0.1:8765/mcp --header "Authorization: Bearer YOUR_TOKEN"
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "bookx": {
      "command": "/path/to/bookx-cli",
      "args": ["--library", "/home/you/Documents/Bookx Library", "mcp"]
    }
  }
}
```

### Cursor

`~/.cursor/mcp.json` (or `.cursor/mcp.json` in a project):

```json
{ "mcpServers": { "bookx": { "command": "/path/to/bookx-cli", "args": ["--library", "/home/you/Documents/Bookx Library", "mcp"] } } }
```

With the server inside Bookx: `{ "mcpServers": { "bookx": { "url": "http://127.0.0.1:8765/mcp", "headers": { "Authorization": "Bearer YOUR_TOKEN" } } } }`.

### opencode

`opencode.json`:

```json
{ "mcp": { "bookx": { "type": "local", "command": ["/path/to/bookx-cli", "--library", "/home/you/Documents/Bookx Library", "mcp"], "enabled": true } } }
```

With the server inside Bookx: `{ "mcp": { "bookx": { "type": "remote", "url": "http://127.0.0.1:8765/mcp", "headers": { "Authorization": "Bearer YOUR_TOKEN" }, "enabled": true } } }`.

The Preferences buttons fill in the right paths for your installation, so prefer copying from there.

## From the command line

```bash
bookx-cli mcp                                  # stdio, read-only (what assistants start)
bookx-cli mcp --http 127.0.0.1:8765            # Streamable HTTP; prints a random token on stderr
bookx-cli mcp --http 127.0.0.1:8765 --token "$(openssl rand -hex 24)"
bookx-cli mcp --http 127.0.0.1:8765 --tls-cert cert.pem --tls-key key.pem
bookx-cli mcp --allow-write                    # let the assistant change the library
bookx-cli mcp --allow-write --allow-dir ~/Downloads/books   # …and import books from that folder
bookx-cli mcp --allow-write --allow-network    # …and look books up online (fetch_metadata)
```

The token can also come from the `BOOKX_MCP_TOKEN` environment variable. It must be at least 16 characters.
Without `--library` the default library is used, like every `bookx-cli` command.

!!! note "Keep it on this computer"
    The HTTP server is meant for `127.0.0.1`. If you bind it to another address, anyone who gets the token
    can read your books. Use TLS (`--tls-cert`, `--tls-key`) and a long token, and do not expose it to the internet.
    If other computers must reach it, put a proxy in front: [reverse proxy](reverse-proxy.md#mcp-behind-a-proxy).

## What assistants can do

Always available (read only):

| Tool | What it does |
|---|---|
| `search_books` | Search with Bookx's query language (`author:`, `tag:`, `series:`, `rating:>=4`, `status:reading`, …), sorted and paged |
| `get_book` | Everything about one book: metadata, formats, where each field came from, progress, shelves |
| `list_collections`, `list_series`, `list_authors`, `list_tags` | Browse the library |
| `get_toc` | Chapters of a book |
| `read_book_text` | Plain text of a chapter, a piece at a time, continuing across chapters |
| `search_in_book` | Find words, phrases or a regular expression inside one book, with snippets |
| `search_all_books_text` | Full-text search over all indexed books |
| `get_annotations`, `export_annotations` | Highlights, notes and bookmarks, filtered; exported as Markdown, text or JSON |
| `get_reading_stats`, `recent_books` | What you read and when |
| `find_duplicates`, `similar_books` | Housekeeping and recommendations |
| `check_book`, `convert_preview` | Validate a book; see what a conversion would lose |

Also offered: *resources* for books, covers and chapters (`bookx://book/ID`, `bookx://book/ID/cover`,
`bookx://book/ID/chapter/N`, `bookx://library/stats`) and *prompts*: summarise a chapter, reading
recommendations, study notes from highlights, find quotes about a topic.

### What assistants can change

Only when you allow it (the switch in Preferences, or `--allow-write`):

`add_highlight`, `add_note`, `set_rating`, `add_tags`, `remove_tags`, `mark_read`, `add_to_shelf`,
`update_metadata` (title, authors, series, publisher, language, description, date, tags),
`add_book_from_path` (only from folders you allowed) and `fetch_metadata` (only with network access allowed).

* Metadata edits and imports go through the same path as edits in Bookx, so **Edit → Undo** takes them back.
* Highlights and notes appear in the reader and can be deleted there.
* Nothing is ever deleted: there is no delete tool.
* Every change is written to `.bookx/mcp-audit.log` in the library folder (which tool, when, with which
  arguments; secrets are blanked).
* With the command (`--allow-write`), changes are refused while Bookx itself has the library open, so
  two programs never write at once. The server inside Bookx writes through the app's own rights.

!!! warning "Switch changes on only for assistants you trust"
    An assistant that can write can make many edits quickly. Undo works step by step. Take a backup
    (main menu → **Library Health** → **Backups**, see [Backup and restore](backup-and-restore.md)) before letting an assistant reorganise a large library.

## Safety and limits

* **Read only by default.** Write tools are not even listed until you allow them.
* **Your books are yours.** Copy-protected books (DRM) are never read: their text is refused, only their
  titles and metadata are visible. Answers include a reminder to quote briefly.
* **Nothing outside the library.** The server reads only the library and, for imports, folders you listed.
  It never shows where your library lives on disk, and it never exposes passwords or the keyring.
* **Small answers.** Lists are paged, a piece of text is at most 20,000 characters, an answer is never larger
  than 256 KB, calls that take over 30 seconds stop, and by default 240 calls per minute are allowed.
* **Local only.** The server inside Bookx binds `127.0.0.1`, requires the token (wrong tokens are slowed
  down), and refuses requests from web pages of other sites.
* **No telemetry.** Bookx sends nothing anywhere for this feature. `fetch_metadata` contacts Open Library,
  Google Books and Wikidata only if you started the server with `--allow-network`.

## Troubleshooting

* *The assistant says the server failed to start.* Run the command from a terminal:
  `bookx-cli --library "…" mcp` and send nothing; it should wait silently. If it prints an error, that is the reason
  (for example "no library at …": check the `--library` path).
* *“the library is open in …; writes are refused”.* Bookx has the library open. Close it, or switch on
  **Allow changes to the library** on Preferences → MCP for the server inside Bookx.
* *Full-text search finds nothing.* Only books whose text Bookx has indexed are searched; open the library in Bookx
  once and let it finish indexing. `search_in_book` always works.
* *Port already in use.* Switch the server off, then pick another **Port** on the MCP page of Preferences.

## Questions and answers

??? question "How do I connect Claude Desktop?"
    Open Preferences → MCP, and in **Connect an assistant** press **Command** on the Claude Desktop row. Paste what
    was copied into `claude_desktop_config.json` (see [Claude Desktop](#claude-desktop)) and restart Claude Desktop. The
    command way works whether Bookx is open or not.

??? question "How do I connect Cursor or opencode?"
    The same: **Connect an assistant** has a row for each, with **Command** and **Server**. Paste the result into
    `~/.cursor/mcp.json` for Cursor or `opencode.json` for opencode; examples are above under
    [Cursor](#cursor) and [opencode](#opencode).

??? question "How do I connect Claude Code?"
    Press **Command** (or **Server**) on its row; the copied text is the `claude mcp add` command to run in a
    terminal. See [Claude Code](#claude-code).

??? question "Which should I choose, the command or the server inside Bookx?"
    The command (`bookx-cli mcp`) is read-only, needs nothing switched on and works with Bookx closed. Choose the
    server (**Let AI assistants connect**) when you want assistants to change the library through the running app, or
    when the assistant can only talk to a web address.

??? question "What is the difference between stdio and HTTP?"
    *stdio* is the command way: the assistant starts `bookx-cli mcp` and talks to it directly. *HTTP* is the server
    inside Bookx (or `bookx-cli mcp --http`): the assistant connects to `http://127.0.0.1:8765/mcp` with the token in an
    `Authorization: Bearer` header.

??? question "How do I make a new access token?"
    Preferences → MCP → **Access token** → **Make a new token**. Bookx asks first, replaces the token and restarts the
    server. Every assistant that used the old token stops working until you copy its settings again from **Connect an
    assistant**. Do this if the token was ever shared by mistake.

??? question "How do I copy my token?"
    The token is hidden on the page; use the copy button on the **Access token** row. Never paste it into a chat or a
    shared file.

??? question "How do I keep assistants from changing my library?"
    Leave **Allow changes to the library** off (the default). Write tools are then not even listed to the assistant.
    The command way is always read-only unless you add `--allow-write` yourself.

??? question "How do I let an assistant edit my library?"
    Preferences → MCP → switch on **Allow changes to the library** and confirm with **Allow Changes**. Assistants can then
    add highlights and notes, rate, tag and shelve books and edit metadata, never delete. Take a backup first.

??? question "Can I undo what an assistant did?"
    Metadata edits and imports can be undone with **Edit → Undo** (or the Undo of Library Health), one step at a time.
    Highlights and notes can be deleted in the reader. Nothing is ever deleted by an assistant.

??? question "Where can I see what an assistant did?"
    Every change is written to `.bookx/mcp-audit.log` in your library folder, with the tool, the time and the arguments
    (secrets blanked). Preferences → MCP → **Recent activity** shows which assistants connected lately.

??? question "Can an assistant on another computer reach my library?"
    Not by default: the server answers on this computer only. For remote access put a proxy with HTTPS in front, as
    in [Reverse proxy](reverse-proxy.md#mcp-behind-a-proxy), and use a long token. Never expose it to the internet
    without TLS.

??? question "Can an assistant read a copy-protected book?"
    No. Books with DRM show only their title and metadata; their text is refused.

??? question "Does my library leave my computer?"
    Bookx sends nothing anywhere for this feature. But the text an assistant reads becomes part of your conversation with
    it, and its provider receives that. Use a local model if that matters.

??? question "Why does the assistant say it cannot find my books' text?"
    Full-text search only covers books Bookx has already indexed; open the library in Bookx once and let it finish.
    Asking the assistant to search inside one book always works.

??? question "How do I change the port?"
    Preferences → MCP: switch **Let AI assistants connect** off, change **Port**, switch it on again, then copy the
    **Server** settings again for each assistant, because they contain the port.

??? question "Do I need Bookx open for the assistant to work?"
    Not with the command way. With the server inside Bookx, yes: the server runs only while Bookx is open and the
    switch is on.

??? question "How do I let an assistant add books from a folder or look books up online?"
    That is only possible from the command line, with `--allow-write --allow-dir FOLDER` for imports and
    `--allow-network` for looking up metadata online. See [From the command line](#from-the-command-line).
