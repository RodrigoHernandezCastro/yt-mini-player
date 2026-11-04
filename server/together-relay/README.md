# Watch together relay

The small server that lets YT Mini Player apps find each other for **Watch together**. It runs
on Cloudflare Workers with Durable Objects, which the free plan covers for a group of friends
(each room is one Durable Object; keepalives are answered without waking it up).

What it does and does not see:

- It passes messages between the host and up to five guests. What they say (the video, the
  position, names, the room key, the WebRTC offers and answers) is **end-to-end encrypted** by
  the apps (X25519 + AES-256-GCM).
- A shared window never goes through it: picture and sound travel straight between the apps
  (WebRTC). The relay only hands out the servers that help them find each other (`ice`).
- It never sees the invitation code: guests look the room up by
  `SHA-256("ytmp-together/lookup/v1" ‖ code)`, and that lookup expires with the code.
- It keeps nothing once a room ends, and logs nothing (`observability` is off).

## Deploy

You need a free Cloudflare account and Node.js 20 or newer.

```sh
cd server/together-relay
npm install
npx wrangler login
npx wrangler deploy
```

Wrangler prints the address, something like `https://ytmp-together.<your-subdomain>.workers.dev`.
The app wants it as `wss://ytmp-together.<your-subdomain>.workers.dev`:

- **Build it in:** set `YTMP_TOGETHER_RELAY` when building (or as a repository variable for the
  release workflow), and nobody has to type anything.
- **Or type it once:** in the app, *Watch together… → Server address*.

**Updating from 0.9:** run `npx wrangler deploy` again. YT Mini Player 0.10 needs this version
(rooms of six, the `ice` operation, bigger messages for the connection offers).

**Limits per address** (on by default, in `wrangler.toml`): one address may try 20 codes,
open 10 rooms and get 10 TURN logins a minute. Anyone can reach the relay, so without them a
stranger could open rooms by the thousand (using up the free plan's daily requests, which
stops Watch together for everyone) or just to collect TURN logins that you pay for. They use
Workers rate limiting; remove a `[[ratelimits]]` block to turn that limit off, or set the
variable `RATE_LIMITS = "off"` to turn them all off (the tests do).

### Optional: TURN, for strict networks

Window sharing connects the apps directly. That works on most home networks with the public
STUN servers the relay hands out, but some networks (mobile data, campus or office networks,
some routers) need a **TURN** relay in between. Cloudflare's is free up to 1,000 GB a month
(about 900 hours of one guest watching at full quality), then $0.05 per GB.

1. In the Cloudflare dashboard: **Realtime → TURN Server → Create**. Copy the **Turn Token ID**
   and the **API Token**.
2. Give them to the worker as secrets (they are never in the code or the repository):

   ```sh
   npx wrangler secret put TURN_KEY_ID
   npx wrangler secret put TURN_KEY_API_TOKEN
   ```

From then on, each member of a room gets short-lived TURN credentials (8 hours) along with the
STUN servers, and WebRTC uses them only when a direct connection fails. They only go to rooms
someone has joined (a host alone gets STUN, and asks again when its first guest is in), three
times per connection and 10 a minute per address. TURN sees only encrypted packets. In the
Cloudflare dashboard you can also set a usage alert for TURN.

## Protocol

`GET /v1/health` answers `ok`. The rest are WebSockets carrying JSON text:

| Path | Who | Headers |
| --- | --- | --- |
| `/v1/host/<room>` | the host (`<room>`: 32 hex characters it made up) | `X-Ytmp-Host: <64 hex>` (the first connection sets it) |
| `/v1/join/<lookup>` | a guest with a code | — |
| `/v1/rejoin/<room>` | a guest whose connection dropped | `X-Ytmp-Member`, `X-Ytmp-Ticket` |

Refusals come as HTTP statuses: `403` (wrong token or ticket), `404` (no such code or room),
`409` (full: host + 5 guests), `503` (the host is reconnecting).

From the host: `send {to: "*" | "g1", data}`, `kick {member}`, `advertise {lookup, ttl}`
(at most 40 minutes), `withdraw {lookup}`, `end`.
From a guest: `send {data}` (always to the host), `leave`.
From anyone in the room: `ice` (answered with `ice {servers}`: STUN, plus TURN with credentials
if the worker has a TURN key; port 53 left out).
From the relay: `ready {guests}`, `you {member, ticket, room}`, `joined {member, resumed?}`,
`left {member, reason}` (`left`, `kicked`, `dropped`, `gone`), `msg {from, data}`, `host-away`,
`host-back`, `advertised {lookup, ok}`, `ended {reason}`, `error {error}`.

Close codes: `4000` the room ended, `4001` removed by the host, `4002` replaced by a newer
connection of the same member. A guest's seat is kept for 60 s after its connection drops;
the room ends if the host is away for 120 s. Messages are text, at most 32 KiB, at most 120
per 10 s per connection. The text `ping` is answered `pong` (keepalive).

## Test

```sh
npm install
npm test          # starts wrangler dev on a free port and runs test/run.mjs
```

`RELAY=ws://127.0.0.1:8799 npm test` runs the same tests against a relay that is already up.
