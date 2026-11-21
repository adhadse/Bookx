# Put Bookx behind a reverse proxy

Bookx can run small servers: the **OPDS server** of Sync (OPDS catalogue, web page, downloads and the `/bookx` sync
endpoint for reading position and KOReader highlights), `bookx-cli serve`, and the **MCP** server for AI assistants.
They are meant for a trusted home network. If you want to reach them from more devices, from a nicer address such as
`https://bookx.lan/bookx`, or from outside the house, put a **reverse proxy** in front of them and let only the proxy
talk to Bookx.

A reverse proxy is a small web server on your network that receives the requests and forwards them to Bookx. It gives
you one address, real HTTPS, logs, rate limits and one place for the firewall rules. It does **not** replace Bookx’s
own accounts and tokens: keep those switched on.

## The recommended setup

```text
 KOReader / OPDS app / browser           (only know https://bookx.lan)
              │  HTTPS
              ▼
   ┌───────────────────────┐
   │ reverse proxy (Caddy) │   bookx.lan → 192.168.1.10, certificate, limits
   └──────────┬────────────┘
              │  plain HTTP, 127.0.0.1:8080 (same machine) or the LAN (firewalled)
              ▼
   ┌───────────────────────┐
   │ Bookx server          │   accounts / tokens stay on
   └───────────────────────┘
```

1. Give the computer that runs Bookx a **fixed address** (see [A fixed address](#a-fixed-address)).
2. Give it a **name** (`bookx.lan`) in your router’s DNS or in a local DNS server such as Pi-hole.
3. Run the proxy on the same machine (simplest: Bookx listens on `127.0.0.1` only) or on another machine at the edge of
   your LAN.
4. Let the proxy end the HTTPS connection. Bookx itself can stay on plain HTTP on the loopback address.
5. Allow only the proxy to reach Bookx’s port (firewall). Clients never talk to Bookx directly.

## What Bookx serves

| Service | Path on the server | Login | Notes for the proxy |
|---|---|---|---|
| **Sync** (reading position, KOReader-compatible) | `/bookx/users/create`, `/bookx/users/auth`, `/bookx/syncs/progress`, `/bookx/syncs/progress/<document>` | account name and key in the `x-auth-user` / `x-auth-key` headers | Keep the `/bookx` prefix. Small JSON bodies. |
| **Sync** (KOReader highlights and notes, app only) | `/bookx/annotations/<md5>` | same account | Bodies up to 1 MiB. Needs *Sync highlights, notes and bookmarks* switched on in Preferences → Sync. |
| **Health** | `/healthcheck` (also `/bookx/healthcheck` when sync is on) | none | Fine for the proxy’s own health checks. Never cached. |
| **OPDS** catalogue | `/opds`, `/opds/...` | HTTP basic auth (user `bookx` in the app, `--user` in the CLI) | Links inside the catalogue are absolute paths from the root (`/opds/...`, `/download/...`, `/cover/...`), so do **not** publish it under a different prefix. |
| **Web page** and downloads | `/`, `/download/<id>.epub`, `/cover/<id>` | HTTP basic auth | Large downloads, `Range` requests (resume) must pass through. No buffering of whole files in the proxy. |
| **MCP** (AI assistants, `bookx-cli mcp --http`) | `/mcp` | `Authorization: Bearer <token>` | Separate port, separate server. Answers are single JSON bodies, no event stream. |

There is only one sync address: **`/bookx`**. There are no other sync paths and no sync routes at the server root.
In KOReader and in Bookx, use `https://bookx.lan/bookx` (or `http://…` on a trusted network).

!!! warning "Keep the `/bookx` prefix"
    Bookx answers `/bookx/users/auth`, not `/users/auth`. Forward the path **unchanged**. A proxy that strips the prefix
    gives you `404 Not found`.

## A fixed address

The address of the computer must not change, or the proxy and the devices lose it.

**Easiest: a DHCP reservation in your router.** Find the computer’s MAC address and tell the router to always give it the
same IP address (the menu is usually called *DHCP reservation*, *static lease* or *address reservation*). Nothing to
configure on the computer.

**Or a static address on the computer.** First see what it has now:

```bash
ip -4 addr show scope global      # the inet line, e.g. 192.168.1.10/24
ip route show default             # the router, e.g. via 192.168.1.1
```

With NetworkManager (replace the connection name, `nmcli connection show` lists them):

```bash
nmcli connection modify "Wired connection 1" \
  ipv4.method manual ipv4.addresses 192.168.1.10/24 \
  ipv4.gateway 192.168.1.1 ipv4.dns 192.168.1.1
nmcli connection up "Wired connection 1"
```

With Netplan (`/etc/netplan/01-bookx.yaml`, then `sudo netplan apply`):

```yaml
network:
  version: 2
  ethernets:
    enp3s0:
      dhcp4: false
      addresses: [192.168.1.10/24]
      routes: [{ to: default, via: 192.168.1.1 }]
      nameservers: { addresses: [192.168.1.1] }
```

Do not pick an address inside the router’s DHCP pool unless the router also reserves it.

**A name.** Add `bookx.lan → 192.168.1.10` in the router’s local DNS (or Pi-hole, AdGuard Home, dnsmasq: `address=/bookx.lan/192.168.1.10`).
Without a local DNS, Avahi/mDNS gives you `bookx.local` (`hostnamectl set-hostname bookx`, Avahi running). KOReader on an
e-reader does not always resolve `.local` names; if sync fails with a name, try the plain IP address first.

## Start Bookx for the proxy

**Command line** (exact options of `bookx-cli serve`):

```bash
bookx-cli serve --bind 127.0.0.1 --port 8080 --user me:secret --kosync ~/bookx-sync.json
```

`--bind 127.0.0.1` is the default, so the server is only reachable from the same machine. Use `--bind 192.168.1.10` (the
LAN address) only when the proxy runs on another machine, and then firewall the port. Leave `--tls-cert` and `--tls-key`
off when the proxy ends the HTTPS connection. `bookx-cli serve` does not offer the KOReader highlights-and-notes sync;
that part is only in the app.

**In the app** (main menu → Preferences → Sync, then the collapsed **Advanced** row): **Allow other devices on the network** decides whether the server is reachable from *this computer only* (off) or from *all
interfaces* (on). For a proxy on the same computer leave it off, leave HTTPS off and point the proxy at
`127.0.0.1` and the port shown there. If the proxy is on another machine, you must switch it on;
Bookx has no setting for a single LAN address, so use the computer’s firewall to allow only the proxy (see below).

**MCP**: `bookx-cli mcp --http 127.0.0.1:8765 --token "$(openssl rand -hex 24)"`. See [MCP](ai-assistants.md).

## Caddy (recommended)

Caddy gets and renews certificates by itself and needs very little configuration. On a plain LAN, `bookx.lan` is not a
public name, so Caddy uses its own internal certificate authority (see [HTTPS](#https)).

`/etc/caddy/Caddyfile`:

```caddyfile
bookx.lan {
    tls internal

    # Everything for Bookx: sync, OPDS, web page, downloads. The path is passed on unchanged.
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Forwarded-Proto {scheme}
        header_up X-Forwarded-For {remote_host}
        transport http {
            response_header_timeout 60s
        }
    }

    request_body {
        max_size 2MB
    }
    log {
        # do not log request bodies or credentials
        output file /var/log/caddy/bookx.log
    }
}
```

Caddy already sends `Host`, `X-Forwarded-For` and `X-Forwarded-Proto`; the two `header_up` lines only make that explicit.
To publish **only** the sync endpoint and keep the catalogue private, replace the `reverse_proxy` block by:

```caddyfile
    handle /bookx/* {
        reverse_proxy 127.0.0.1:8080
    }
    handle /healthcheck {
        reverse_proxy 127.0.0.1:8080
    }
    handle {
        respond "Not found" 404
    }
```

`handle` does not remove the prefix (`handle_path` would, so do not use it here). MCP, on its own port, gets its own
site, for example `mcp.bookx.lan { reverse_proxy 127.0.0.1:8765 }`.

## nginx

```nginx
server {
    listen 443 ssl;
    server_name bookx.lan;
    ssl_certificate     /etc/ssl/bookx/bookx.lan.crt;
    ssl_certificate_key /etc/ssl/bookx/bookx.lan.key;

    client_max_body_size 2m;           # Bookx itself accepts at most 1 MiB per request
    proxy_read_timeout   60s;
    proxy_send_timeout   60s;

    location /bookx/ {
        proxy_pass http://127.0.0.1:8080;      # NO path after the port: the prefix stays
        proxy_set_header Host              $host;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        access_log /var/log/nginx/bookx.log;   # default format has no bodies
    }

    location = /healthcheck { proxy_pass http://127.0.0.1:8080; }

    # optional: catalogue, web page, downloads
    location ~ ^/(opds|download|cover)(/|$) {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_request_buffering on;
        proxy_buffering off;                    # stream big downloads, keep Range working
    }

    location / { return 404; }                  # nothing else is published
}

server {                                         # never serve Bookx on a name you did not choose
    listen 443 ssl default_server;
    ssl_certificate     /etc/ssl/bookx/bookx.lan.crt;
    ssl_certificate_key /etc/ssl/bookx/bookx.lan.key;
    return 444;
}
```

The common mistake: `proxy_pass http://127.0.0.1:8080/;` (with a trailing slash) inside `location /bookx/` **removes** the
`/bookx/` prefix, and every request ends in a 404. Write it without the slash, as above.

Rate limiting is a good second layer: `limit_req_zone $binary_remote_addr zone=bookx:10m rate=10r/s;` and `limit_req
zone=bookx burst=40 nodelay;` in the `location` blocks.

## Traefik (Docker or file provider)

Only needed if you already use Traefik. Bookx itself is not in a container here; for a Bookx on the host use the file
provider:

```yaml
http:
  routers:
    bookx:
      rule: "Host(`bookx.lan`) && (PathPrefix(`/bookx`) || Path(`/healthcheck`))"
      entryPoints: [websecure]
      tls: {}
      service: bookx
  services:
    bookx:
      loadBalancer:
        servers:
          - url: "http://127.0.0.1:8080"
```

Do not add a `stripPrefix` middleware. For a Bookx container, put the same rule in the labels
(`traefik.http.routers.bookx.rule=…`, `traefik.http.services.bookx.loadbalancer.server.port=8080`).

## HTTPS

* **On the internet:** only with HTTPS and a real certificate (Let’s Encrypt through the proxy). Do not publish the
  catalogue or MCP without a login, and think twice about publishing them at all.
* **On your LAN:** HTTPS is best, plain HTTP is acceptable only on a network you fully trust and do not share with guests.
* **Certificates for a name that only exists at home:** either run a small **internal certificate authority** (Caddy’s
  `tls internal`, `step-ca`, `mkcert`) and install its root certificate on your devices, or use a real domain you own with a
  **DNS-01** Let’s Encrypt certificate (works without opening any port; Caddy and Traefik have plugins for most DNS
  providers; `bookx.example.org` points to the LAN address in your local DNS).
* **KOReader** on an e-reader does not accept a self-signed certificate that its system does not know. Prefer a
  certificate from a DNS-01 domain (trusted everywhere) or install your CA on the device. If that is too much work, use
  plain HTTP on the LAN to the proxy and keep the proxy off the internet.
* Bookx’s own library-server HTTPS (self-signed) is a different thing: with a proxy you can switch it off and let the
  proxy do the HTTPS.

## Accounts, headers and limits

* **Keep Bookx’s accounts on.** Do not turn off the library password under main menu → Preferences → Sync → Advanced or leave `bookx-cli serve` without `--user`
  because "the proxy has a login". Do not let the proxy add its own `Authorization` header for Bookx either. The sync
  account (the key that KOReader sends) must reach Bookx unchanged.
* **Pass `Host`, `X-Forwarded-For` and `X-Forwarded-Proto`.** Bookx ignores `X-Forwarded-*` unless you list your proxy as
  trusted (next point), so by default every request comes from the proxy’s address in Bookx’s own logic.
* **Trusted proxy addresses (optional).** Enter the address of your proxy (`127.0.0.1`, or a network such as `10.0.0.0/8`) in
  Preferences → Sync → Advanced → *Trusted proxy addresses*, or use `bookx-cli serve --trusted-proxy 127.0.0.1`. Bookx then
  reads `X-Forwarded-For` **only for connections from those addresses**, from the right: the last address that is not itself a
  trusted proxy is the client. The failed-login limit then counts each client on its own. A network of `/0` is refused. Never list an address that
  strangers can connect from.
* **Sub-path.** To publish Bookx under `https://example.com/books`, enter `/books` as *Sub-path behind a proxy* (or
  `--public-prefix /books`). Links in the web page and the OPDS catalogue then start with `/books`. The proxy may strip the
  prefix or keep it; both work. The sync address for KOReader becomes `https://example.com/books/bookx`.
* **Tell Bookx the name people use.** Bookx answers only to `Host` names it knows, so that a web page cannot reach it through
  a hostile DNS name (“DNS rebinding”). IP addresses, `localhost`, one-word names and names ending in `.local`, `.lan`,
  `.internal`, `.localhost` or `.home.arpa` always work. Any other name (`books.example.com`, `nas.fritz.box`) has to be
  listed: `bookx-cli serve --allow-host books.example.com`, `bookx-cli mcp --http … --allow-host books.example.com`, or
  *Allowed host names* in Preferences → Sync → Advanced (several names separated by commas), or
  `BOOKX_ALLOWED_HOSTS=books.example.com,nas.fritz.box` in the environment of the app (Flatpak: `flatpak override --user
  --env=BOOKX_ALLOWED_HOSTS=books.example.com com.adhadse.Bookx`). Requests from a browser page of another site (an
  `Origin` that is not the server itself) are refused as well; KOReader and OPDS apps send no `Origin`.
* **Failed logins are throttled per client address** (10 wrong passwords in a minute, then 429 for a minute). Without a
  trusted proxy that address is the proxy itself, so one person typing a wrong password can block everyone for a minute: list the
  proxy as trusted (see above), and do the fine-grained rate limiting in the proxy too.
* **Connections:** Bookx accepts 64 connections at once, 32 from one address. A proxy keeps few connections open, which
  fits. Do not configure hundreds of upstream keep-alive connections.
* **Body size:** Bookx refuses request bodies over 1 MiB (`413`). Sync requests are far smaller (annotations: up to 1 MiB, at most
  1000 highlights per request). Set the proxy limit to 2 MB at most. Bookx has no upload route for books or covers; downloads
  are the only large transfers, and they flow from Bookx to the client.
* **Timeouts:** Bookx closes a request head that takes over 10 s and a body that takes over 30 s. 60 s read/send
  timeouts in the proxy are fine. Keep timeouts of several minutes for the downloads only.
* **Do not cache.** Bookx already sends `Cache-Control: no-store` for everything behind a login. Do not enable a proxy
  cache for Bookx.
* **Only the names you chose:** serve Bookx on one `server_name` / host and answer anything else with an error, so that
  `http://192.168.1.10/` or a guessed name leads nowhere (the second nginx `server` above).
* **Logs:** access logs only have paths (`/bookx/annotations/<md5>`, `/download/12.epub`), not book titles. Do **not** enable
  request-body logging or debug logging: the bodies contain your highlights and notes, and basic-auth headers contain the
  password. Keep logs short-lived.
* **CORS** does not matter: KOReader and OPDS apps are not browsers. The MCP server refuses browser requests whose `Origin`
  is not the same computer unless you allow it with `--allow-origin`; a proxy must pass `Origin` unchanged.

## MCP behind a proxy

`bookx-cli mcp --http` serves a single endpoint, `POST /mcp`, protected by a bearer token. It answers every request with
one JSON body (no streaming), so proxy buffering is not a problem; if your proxy or assistant needs a stream, switch off
buffering (`proxy_buffering off;`, `flush_interval -1` in Caddy). Pass `Authorization`, `Mcp-Session-Id` and
`Mcp-Protocol-Version` unchanged.

* Never publish MCP on the internet. It reads your whole library (and changes it with `--allow-write`). If you must,
  restrict the proxy to the addresses of your assistant, use HTTPS, a long random token and keep it read-only.
* The proxy’s login does not replace the token, and the token is never in a URL.

## Firewall

Only the proxy should reach Bookx’s port. With `ufw`, for a proxy on `192.168.1.20` and Bookx on `192.168.1.10`:

```bash
sudo ufw default deny incoming
sudo ufw allow from 192.168.1.20 to any port 8080 proto tcp
sudo ufw allow from 192.168.1.0/24 to any port 443 proto tcp   # the proxy's own port, on the proxy machine
```

When both run on the same computer and Bookx binds to `127.0.0.1`, no rule is needed: nothing but the proxy can reach it.
Do not forward Bookx’s port, or the proxy’s, from the router to the internet unless you really mean to publish it.

## Check that it works

```bash
curl -i https://bookx.lan/healthcheck                  # 200 {"state":"OK"}
curl -i https://bookx.lan/bookx/healthcheck            # 200 (sync switched on)
curl -i https://bookx.lan/bookx/users/auth             # 401: login is required, so the path reaches Bookx
curl -u bookx:PASSWORD -I https://bookx.lan/opds       # 200 (if you publish the catalogue)
```

Use `http://` and `-k` only while testing. Then, in KOReader, set the server to `https://bookx.lan/bookx` (in Progress sync and in
Bookx’s plugin) and open a book to see whether the position is fetched.

## Troubleshooting

| What you see | Usual reason |
|---|---|
| `404 Not found` on `/bookx/...` | The proxy removed the prefix (nginx trailing slash, Traefik `stripPrefix`, Caddy `handle_path`), or sync is not switched on in Bookx. |
| `404` on `/bookx/annotations/...` only | *KOReader highlights and notes* is off, or Bookx runs as `bookx-cli serve` (no annotation sync there). |
| `403` `Host not allowed` | The proxy passes a name Bookx does not know. Add it with `--allow-host` or `BOOKX_ALLOWED_HOSTS` (see “Tell Bookx the name people use”). |
| `401` | Wrong account, or the proxy replaced/dropped the `x-auth-user`, `x-auth-key` or `Authorization` headers. |
| `429` | Too many wrong passwords in a minute; wait. Behind a proxy this affects every client. |
| `413` | The proxy or Bookx got a body over its limit (1 MiB for Bookx). |
| `502` / `Bad gateway` | Bookx is not running, listens on another port, or the proxy points at HTTPS while Bookx serves HTTP (or the reverse). |
| `503` | Bookx has too many connections open (limit 64). |
| KOReader says it cannot connect, browser works | Certificate not trusted by KOReader, a `.local` name it cannot resolve (try the IP), or a firewall between the e-reader and the proxy. |
| Works at home, not away | The router is not forwarding anything (that is the safe default). Use a VPN such as WireGuard or Tailscale instead of publishing Bookx. |
| MCP client hangs | Buffering in the proxy, or the token and `Mcp-Session-Id` headers are not passed through. |

!!! note "What is not covered"
    This page was written from Bookx’s code and the proxies’ documentation; the configurations were not run against a
    real KOReader device. Test with `curl` first, then with your reader, and tell us what needs fixing.
