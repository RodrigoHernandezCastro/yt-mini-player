// Watch together relay for YT Mini Player (Cloudflare Workers + Durable
// Objects).
//
// It only passes messages between the people in a room: what they say to
// each other is end-to-end encrypted by the apps, and the invitation code
// never reaches it (guests look the room up by a SHA-256 of the code).
//
//   GET /v1/health                  "ok"
//   GET /v1/host/<room>             WebSocket for the host (X-Ytmp-Host: token)
//   GET /v1/join/<lookup>           WebSocket for a guest who has a code
//   GET /v1/rejoin/<room>           WebSocket for a guest coming back
//                                   (X-Ytmp-Member, X-Ytmp-Ticket)
//
// A room is a Durable Object: the host plus up to five guests. Lookups live
// in Lobby objects that forget them when they expire. Nothing is logged.
//
// Window sharing: the pictures and sound go straight between the apps
// (WebRTC); the relay only hands out the servers that help them find each
// other (`ice`): public STUN, plus Cloudflare's TURN if this worker has the
// TURN_KEY_ID and TURN_KEY_API_TOKEN secrets (see README.md).

import { DurableObject } from 'cloudflare:workers';

const ROOM_ID = /^[0-9a-f]{32}$/;
const HEX64 = /^[0-9a-f]{64}$/;
const MEMBER = /^g[0-9]{1,6}$/;
const MAX_GUESTS = 5;
// The apps keep connection offers small; this leaves room to spare.
const MAX_MESSAGE = 32 * 1024;
// Per connection: this many messages per window, the rest are dropped.
const RATE_WINDOW_MS = 10_000;
const RATE_MAX = 120;
// A lookup lives at most this long (a code lasts 30 minutes, plus 5 of grace).
const MAX_TTL_S = 40 * 60;
const MAX_LOOKUPS = 8;
// ICE servers: how often one connection may ask, and how long TURN
// credentials last (longer than a long movie night).
const ICE_ASKS = 3;
const TURN_TTL_S = 8 * 60 * 60;
const STUN = [{ urls: ['stun:stun.cloudflare.com:3478', 'stun:stun.l.google.com:19302'] }];

const text = (status, body) =>
  new Response(body, { status, headers: { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' } });

async function sha256(value) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function randomHex(bytes) {
  return [...crypto.getRandomValues(new Uint8Array(bytes))].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function graceMs(env, name, fallback) {
  const value = Number(env[name]);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

// STUN always; TURN (short-lived credentials from Cloudflare Realtime) when
// the worker has a TURN key. Port 53 is left out: browsers give up on it
// slowly, and the apps gather every candidate before sending their offer.
async function iceServers(env) {
  if (!env.TURN_KEY_ID || !env.TURN_KEY_API_TOKEN) return STUN;
  try {
    const res = await fetch(`https://rtc.live.cloudflare.com/v1/turn/keys/${encodeURIComponent(env.TURN_KEY_ID)}/credentials/generate-ice-servers`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${env.TURN_KEY_API_TOKEN}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ ttl: TURN_TTL_S }),
    });
    if (!res.ok) return STUN;
    const body = await res.json();
    const servers = (Array.isArray(body.iceServers) ? body.iceServers : [])
      .map((server) => ({
        ...server,
        urls: (Array.isArray(server.urls) ? server.urls : [server.urls])
          .filter((u) => typeof u === 'string' && !/:53(\?|$)/.test(u)),
      }))
      .filter((server) => server.urls.length);
    return servers.length ? servers : STUN;
  } catch {
    return STUN;
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === '/v1/health') return text(200, 'ok');
    const parts = url.pathname.split('/');
    if (parts.length !== 4 || parts[0] !== '' || parts[1] !== 'v1') return text(404, 'not found');
    const [, , kind, id] = parts;
    if (request.method !== 'GET' || (request.headers.get('Upgrade') || '').toLowerCase() !== 'websocket') {
      return text(426, 'websocket expected');
    }

    // The room learns the role from the worker only: whatever a client sends
    // under these names is dropped.
    const forward = (room, role, extra = {}) => {
      const headers = new Headers(request.headers);
      for (const name of [...headers.keys()]) {
        if (name.toLowerCase().startsWith('x-relay-')) headers.delete(name);
      }
      headers.set('X-Relay-Role', role);
      headers.set('X-Relay-Room', room);
      for (const [name, value] of Object.entries(extra)) headers.set(name, value);
      const stub = env.ROOM.get(env.ROOM.idFromName(room));
      return stub.fetch(new Request(request.url, { method: 'GET', headers }));
    };

    if (kind === 'host' && ROOM_ID.test(id)) return forward(id, 'host');
    if (kind === 'rejoin' && ROOM_ID.test(id)) return forward(id, 'rejoin');
    if (kind === 'join' && HEX64.test(id)) {
      // Optional: a Workers rate limiting binding against guessing.
      if (env.JOIN_LIMIT) {
        const key = request.headers.get('CF-Connecting-IP') || 'unknown';
        const { success } = await env.JOIN_LIMIT.limit({ key });
        if (!success) return text(429, 'slow down');
      }
      const room = await env.LOBBY.get(env.LOBBY.idFromName(id)).resolve();
      if (!room) return text(404, 'no such code');
      return forward(room, 'guest');
    }
    return text(404, 'not found');
  },
};

// ---------------------------------------------------------------------------
// Lobby: lookup → room, until it expires.
// ---------------------------------------------------------------------------

export class Lobby extends DurableObject {
  async claim(room, expires) {
    const current = await this.ctx.storage.get('entry');
    if (current && current.room !== room && current.expires > Date.now()) return false;
    await this.ctx.storage.put('entry', { room, expires });
    await this.ctx.storage.setAlarm(expires);
    return true;
  }

  async resolve() {
    const current = await this.ctx.storage.get('entry');
    return current && current.expires > Date.now() ? current.room : null;
  }

  async release(room) {
    const current = await this.ctx.storage.get('entry');
    if (current && current.room === room) await this.forget();
  }

  async alarm() {
    await this.forget();
  }

  async forget() {
    await this.ctx.storage.deleteAlarm();
    await this.ctx.storage.deleteAll();
  }
}

// ---------------------------------------------------------------------------
// Room: the host and up to five guests.
// ---------------------------------------------------------------------------

export class Room extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Keepalives are answered without waking the room up.
    this.ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
    this.hostGrace = graceMs(env, 'HOST_GRACE_MS', 120_000);
    this.guestGrace = graceMs(env, 'GUEST_GRACE_MS', 60_000);
    this.rates = new Map();
    this.iceAsks = new Map();
  }

  // --- Connecting ---------------------------------------------------------

  // A storage call keeps other events out of the room until it returns;
  // hashing (crypto.subtle) does not. So each connection hashes first and
  // only then reads, checks and writes the room's storage, with nothing in
  // between that lets a kick, an alarm or another connection in: otherwise
  // two guests could get the same member id (or a sixth seat), a guest
  // kicked while coming back could have its seat written back, and a host
  // could connect to a room that had just ended.
  async fetch(request) {
    const role = request.headers.get('X-Relay-Role');
    const room = request.headers.get('X-Relay-Room') || '';
    if (role === 'host') return this.connectHost(request, room);
    if (role === 'guest') return this.connectGuest();
    if (role === 'rejoin') return this.rejoin(request);
    return text(400, 'bad request');
  }

  async connectHost(request, room) {
    const token = request.headers.get('X-Ytmp-Host') || '';
    if (!HEX64.test(token)) return text(403, 'bad token');
    const hash = await sha256(token);
    let meta = await this.ctx.storage.get('meta');
    if (!meta) {
      meta = { room, hostHash: hash, next: 1 };
      await this.ctx.storage.put('meta', meta);
    } else if (meta.hostHash !== hash) {
      return text(403, 'not the host');
    }
    for (const old of this.sockets('host')) this.close(old, 4002, 'replaced');
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, ['host']);
    server.serializeAttachment({ role: 'host', member: 'host' });
    const away = await this.ctx.storage.get('away');
    await this.ctx.storage.delete('away');
    const guests = [...new Set(this.guestSockets().map((ws) => this.member(ws)))];
    this.send(server, { op: 'ready', guests });
    if (away) for (const ws of this.guestSockets()) this.send(ws, { op: 'host-back' });
    await this.schedule();
    return new Response(null, { status: 101, webSocket: client });
  }

  async connectGuest() {
    const ticket = randomHex(32);
    const ticketHash = await sha256(ticket);
    const meta = await this.ctx.storage.get('meta');
    if (!meta) return text(404, 'no such room');
    const host = this.host();
    if (!host) return text(503, 'host away');
    const seats = (await this.ctx.storage.get('seats')) || {};
    if (Object.keys(seats).length >= MAX_GUESTS) return text(409, 'room full');
    const member = `g${meta.next}`;
    meta.next += 1;
    seats[member] = { ticket: ticketHash, until: 0 };
    await this.ctx.storage.put({ meta, seats });
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, [member]);
    server.serializeAttachment({ role: 'guest', member });
    this.send(server, { op: 'you', member, ticket, room: meta.room });
    this.send(host, { op: 'joined', member });
    return new Response(null, { status: 101, webSocket: client });
  }

  async rejoin(request) {
    const member = request.headers.get('X-Ytmp-Member') || '';
    const ticket = request.headers.get('X-Ytmp-Ticket') || '';
    const valid = MEMBER.test(member) && HEX64.test(ticket);
    const ticketHash = valid ? await sha256(ticket) : null;
    const meta = await this.ctx.storage.get('meta');
    if (!meta) return text(404, 'no such room');
    const seats = (await this.ctx.storage.get('seats')) || {};
    const seat = valid ? seats[member] : null;
    if (!seat || seat.ticket !== ticketHash) return text(403, 'no seat');
    for (const old of this.sockets(member)) this.close(old, 4002, 'replaced');
    seat.until = 0;
    await this.ctx.storage.put('seats', seats);
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, [member]);
    server.serializeAttachment({ role: 'guest', member });
    this.send(server, { op: 'you', member, ticket, room: meta.room });
    const host = this.host();
    if (host) this.send(host, { op: 'joined', member, resumed: true });
    else this.send(server, { op: 'host-away' });
    await this.schedule();
    return new Response(null, { status: 101, webSocket: client });
  }

  // --- Messages -----------------------------------------------------------

  async webSocketMessage(ws, message) {
    if (typeof message !== 'string') return this.close(ws, 1003, 'text only');
    if (message.length > MAX_MESSAGE) return this.close(ws, 1009, 'too big');
    if (!this.allow(ws)) return;
    let msg;
    try {
      msg = JSON.parse(message);
    } catch {
      return;
    }
    if (!msg || typeof msg !== 'object') return;
    const attachment = ws.deserializeAttachment() || {};
    // Anyone in the room may ask for the ICE servers (a few times).
    if (msg.op === 'ice') return this.ice(ws);
    if (attachment.role === 'host') await this.fromHost(ws, msg);
    else if (attachment.role === 'guest') await this.fromGuest(ws, attachment.member, msg);
  }

  async ice(ws) {
    const asked = (this.iceAsks.get(ws) || 0) + 1;
    this.iceAsks.set(ws, asked);
    if (asked > ICE_ASKS) return;
    this.send(ws, { op: 'ice', servers: await iceServers(this.env) });
  }

  async fromHost(ws, msg) {
    const meta = await this.ctx.storage.get('meta');
    if (!meta) return;
    switch (msg.op) {
      case 'send': {
        if (!msg.data || typeof msg.data !== 'object') return;
        const out = { op: 'msg', from: 'host', data: msg.data };
        const targets = msg.to === '*' ? this.guestSockets() : MEMBER.test(msg.to) ? this.sockets(msg.to) : [];
        for (const guest of targets) this.send(guest, out);
        return;
      }
      case 'kick': {
        if (!MEMBER.test(msg.member)) return;
        const seats = (await this.ctx.storage.get('seats')) || {};
        if (!seats[msg.member]) return;
        delete seats[msg.member];
        await this.ctx.storage.put('seats', seats);
        for (const guest of this.sockets(msg.member)) this.close(guest, 4001, 'removed');
        this.send(ws, { op: 'left', member: msg.member, reason: 'kicked' });
        await this.schedule();
        return;
      }
      case 'advertise': {
        const ttl = Number(msg.ttl);
        if (!HEX64.test(msg.lookup) || !Number.isInteger(ttl) || ttl < 1 || ttl > MAX_TTL_S) return;
        const lobby = this.env.LOBBY.get(this.env.LOBBY.idFromName(msg.lookup));
        const ok = await lobby.claim(meta.room, Date.now() + ttl * 1000);
        if (ok) {
          const lookups = ((await this.ctx.storage.get('lookups')) || []).filter((l) => l !== msg.lookup);
          lookups.push(msg.lookup);
          // The oldest ones have expired by now.
          await this.ctx.storage.put('lookups', lookups.slice(-MAX_LOOKUPS));
        }
        this.send(ws, { op: 'advertised', lookup: msg.lookup, ok });
        return;
      }
      case 'withdraw': {
        const lookups = (await this.ctx.storage.get('lookups')) || [];
        if (!lookups.includes(msg.lookup)) return;
        await this.env.LOBBY.get(this.env.LOBBY.idFromName(msg.lookup)).release(meta.room);
        await this.ctx.storage.put('lookups', lookups.filter((l) => l !== msg.lookup));
        return;
      }
      case 'end':
        await this.end('ended');
        return;
      default:
    }
  }

  async fromGuest(ws, member, msg) {
    switch (msg.op) {
      case 'send': {
        if (!msg.data || typeof msg.data !== 'object') return;
        const host = this.host();
        if (host) this.send(host, { op: 'msg', from: member, data: msg.data });
        return;
      }
      case 'leave': {
        const seats = (await this.ctx.storage.get('seats')) || {};
        if (seats[member]) {
          delete seats[member];
          await this.ctx.storage.put('seats', seats);
          const host = this.host();
          if (host) this.send(host, { op: 'left', member, reason: 'left' });
        }
        this.close(ws, 1000, 'bye');
        await this.schedule();
        return;
      }
      default:
    }
  }

  // --- Connections that end -------------------------------------------------

  async webSocketClose(ws, code, reason) {
    this.rates.delete(ws);
    this.iceAsks.delete(ws);
    this.close(ws, 1000, 'closed');
    await this.lost(ws);
  }

  async webSocketError(ws) {
    this.rates.delete(ws);
    this.iceAsks.delete(ws);
    await this.lost(ws);
  }

  async lost(ws) {
    const meta = await this.ctx.storage.get('meta');
    if (!meta) return;
    const attachment = ws.deserializeAttachment() || {};
    if (attachment.role === 'host') {
      // Replaced by a newer connection: nothing changes.
      if (this.host(ws)) return;
      if (!(await this.ctx.storage.get('away'))) await this.ctx.storage.put('away', Date.now());
      for (const guest of this.guestSockets()) this.send(guest, { op: 'host-away' });
      await this.schedule();
    } else if (attachment.role === 'guest') {
      const member = attachment.member;
      if (this.sockets(member).some((other) => other !== ws)) return;
      const seats = (await this.ctx.storage.get('seats')) || {};
      const seat = seats[member];
      // Removed or said goodbye: the seat is gone already.
      if (!seat || seat.until) return;
      seat.until = Date.now() + this.guestGrace;
      await this.ctx.storage.put('seats', seats);
      const host = this.host();
      if (host) this.send(host, { op: 'left', member, reason: 'dropped' });
      await this.schedule();
    }
  }

  async alarm() {
    const meta = await this.ctx.storage.get('meta');
    if (!meta) return;
    const now = Date.now();
    const away = await this.ctx.storage.get('away');
    if (away && now - away >= this.hostGrace) {
      await this.end('host-gone');
      return;
    }
    const seats = (await this.ctx.storage.get('seats')) || {};
    let changed = false;
    for (const [member, seat] of Object.entries(seats)) {
      if (seat.until && seat.until <= now) {
        delete seats[member];
        changed = true;
        const host = this.host();
        if (host) this.send(host, { op: 'left', member, reason: 'gone' });
      }
    }
    if (changed) await this.ctx.storage.put('seats', seats);
    await this.schedule();
  }

  // The next moment something expires.
  async schedule() {
    const times = [];
    const away = await this.ctx.storage.get('away');
    if (away) times.push(away + this.hostGrace);
    const seats = (await this.ctx.storage.get('seats')) || {};
    for (const seat of Object.values(seats)) if (seat.until) times.push(seat.until);
    if (times.length) await this.ctx.storage.setAlarm(Math.min(...times));
    else await this.ctx.storage.deleteAlarm();
  }

  async end(reason) {
    const lookups = (await this.ctx.storage.get('lookups')) || [];
    const meta = await this.ctx.storage.get('meta');
    // Forgotten first: the closes that follow find nothing to update.
    await this.ctx.storage.deleteAlarm();
    await this.ctx.storage.deleteAll();
    for (const ws of this.ctx.getWebSockets()) {
      this.send(ws, { op: 'ended', reason });
      this.close(ws, 4000, reason);
    }
    if (meta) {
      await Promise.all(lookups.map((l) => this.env.LOBBY.get(this.env.LOBBY.idFromName(l)).release(meta.room)));
    }
  }

  // --- Helpers --------------------------------------------------------------

  sockets(tag) {
    return this.ctx.getWebSockets(tag).filter((ws) => ws.readyState === WebSocket.OPEN);
  }

  host(except) {
    return this.sockets('host').find((ws) => ws !== except) || null;
  }

  guestSockets() {
    return this.ctx
      .getWebSockets()
      .filter((ws) => ws.readyState === WebSocket.OPEN && (ws.deserializeAttachment() || {}).role === 'guest');
  }

  member(ws) {
    return (ws.deserializeAttachment() || {}).member;
  }

  send(ws, value) {
    try {
      ws.send(JSON.stringify(value));
    } catch {
      // Closing already.
    }
  }

  close(ws, code, reason) {
    try {
      ws.close(code, reason);
    } catch {
      // Closed already.
    }
  }

  allow(ws) {
    const now = Date.now();
    let rate = this.rates.get(ws);
    if (!rate || now - rate.start >= RATE_WINDOW_MS) {
      rate = { start: now, count: 0, warned: false };
      this.rates.set(ws, rate);
    }
    rate.count += 1;
    if (rate.count <= RATE_MAX) return true;
    if (!rate.warned) {
      rate.warned = true;
      this.send(ws, { op: 'error', error: 'rate' });
    }
    return false;
  }
}
