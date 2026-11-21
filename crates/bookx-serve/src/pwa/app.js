'use strict';
// Web client of the Bookx content server. Everything shown comes from the server's own catalogue
// (/opds); text is always set with textContent, never as markup.
(function () {
  const BASE = location.pathname.replace(/\/app\/?$/, '');
  const ATOM = 'http://www.w3.org/2005/Atom';
  const REL_ACQ = 'http://opds-spec.org/acquisition';
  const REL_THUMB = 'http://opds-spec.org/image/thumbnail';
  const BOOKS = 'bookx-books';
  const MAX_PAGES = 40;
  const $ = id => document.getElementById(id);
  let books = [];
  let next = BASE + '/opds/all';
  let tab = 'all';
  let saved = new Set();

  function status(text) { $('status').textContent = text; }

  function parseEntries(xml) {
    const doc = new DOMParser().parseFromString(xml, 'application/xml');
    const out = [];
    for (const e of doc.getElementsByTagNameNS(ATOM, 'entry')) {
      const text = n => { const x = e.getElementsByTagNameNS(ATOM, n)[0]; return x ? x.textContent.trim() : ''; };
      let download = '', thumb = '';
      for (const l of e.getElementsByTagNameNS(ATOM, 'link')) {
        const rel = l.getAttribute('rel'), href = l.getAttribute('href') || '';
        if (!href.startsWith('/')) continue;
        if (rel === REL_ACQ) download = href;
        if (rel === REL_THUMB) thumb = href;
      }
      const authors = Array.from(e.getElementsByTagNameNS(ATOM, 'author')).map(a => (a.textContent || '').trim()).filter(Boolean);
      if (download) out.push({ title: text('title') || 'Untitled', authors: authors.join(', '), download, thumb });
    }
    let nextHref = '';
    for (const l of doc.documentElement.children) {
      if (l.localName === 'link' && l.getAttribute('rel') === 'next') nextHref = l.getAttribute('href') || '';
    }
    return { out, next: nextHref.startsWith('/') ? nextHref : '' };
  }

  async function refreshSaved() {
    const cache = await caches.open(BOOKS);
    saved = new Set((await cache.keys()).map(r => new URL(r.url).pathname));
  }

  async function save(book) {
    status('Saving ' + book.title);
    try {
      const cache = await caches.open(BOOKS);
      await cache.add(book.download);
      if (book.thumb) { try { await cache.add(book.thumb); } catch (e) { /* the cover is optional */ } }
      status('Saved ' + book.title);
    } catch (e) {
      status('Could not save ' + book.title);
    }
    await refreshSaved();
    render();
  }

  async function remove(book) {
    const cache = await caches.open(BOOKS);
    await cache.delete(book.download);
    if (book.thumb) await cache.delete(book.thumb);
    await refreshSaved();
    render();
  }

  function row(book) {
    const li = document.createElement('li');
    if (book.thumb) {
      const img = document.createElement('img');
      img.src = book.thumb; img.alt = ''; img.loading = 'lazy';
      li.appendChild(img);
    }
    const meta = document.createElement('div');
    meta.className = 'meta';
    const t = document.createElement('div'); t.className = 't'; t.textContent = book.title;
    const a = document.createElement('div'); a.className = 'a'; a.textContent = book.authors;
    meta.append(t, a);
    li.appendChild(meta);
    const isSaved = saved.has(book.download);
    if (isSaved) {
      const open = document.createElement('a'); open.href = book.download; open.textContent = 'Open'; li.appendChild(open);
      const del = document.createElement('button'); del.textContent = 'Remove'; del.addEventListener('click', () => remove(book)); li.appendChild(del);
    } else if (navigator.onLine) {
      const get = document.createElement('button'); get.textContent = 'Save offline'; get.addEventListener('click', () => save(book)); li.appendChild(get);
    }
    return li;
  }

  function render() {
    const q = $('search').value.trim().toLowerCase();
    const list = $('list');
    list.textContent = '';
    for (const b of books) {
      if (tab === 'saved' && !saved.has(b.download)) continue;
      if (q && !(b.title + ' ' + b.authors).toLowerCase().includes(q)) continue;
      list.appendChild(row(b));
    }
    $('more').hidden = !next || tab === 'saved';
    $('tab-all').setAttribute('aria-pressed', String(tab === 'all'));
    $('tab-saved').setAttribute('aria-pressed', String(tab === 'saved'));
  }

  let pages = 0;
  async function loadMore() {
    if (!next || pages >= MAX_PAGES) return;
    try {
      const r = await fetch(next, { credentials: 'same-origin' });
      if (!r.ok) throw new Error(String(r.status));
      const p = parseEntries(await r.text());
      books = books.concat(p.out);
      next = p.next; pages++;
      status(navigator.onLine ? '' : 'Offline: showing what was loaded before');
    } catch (e) {
      next = '';
      status(navigator.onLine ? 'Could not load the library' : 'Offline: the library was not loaded before');
    }
    render();
  }

  async function start() {
    if ('serviceWorker' in navigator) {
      try { await navigator.serviceWorker.register('sw.js', { scope: (BASE || '') + '/' }); } catch (e) { /* works without */ }
    }
    await refreshSaved();
    $('search').addEventListener('input', render);
    $('more').addEventListener('click', loadMore);
    $('tab-all').addEventListener('click', () => { tab = 'all'; render(); });
    $('tab-saved').addEventListener('click', () => { tab = 'saved'; render(); });
    window.addEventListener('online', () => { status(''); render(); });
    window.addEventListener('offline', () => { status('Offline'); render(); });
    await loadMore();
    if (next && !$('search').value) { /* the rest is loaded on demand */ }
  }
  start();
})();
