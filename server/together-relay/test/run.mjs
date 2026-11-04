// Tests for the relay against a local copy (wrangler dev), or against the
// address in RELAY (for example ws://127.0.0.1:8799) if it is set.
//
//   npm install && npm test

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { join as joinPath } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import WebSocket from 'ws';

const HERE = fileURLToPath(new URL('..', import.meta.url));
const WINDOWS = process.platform === 'win32';
// This folder's wrangler, run by this same node: no shell and no `npx`
// (on Windows that is npx.cmd, which spawn cannot start by itself).
const WRANGLER = joinPath(HERE, 'node_modules', 'wrangler', 'bin', 'wrangler.js');
const PORT = 8790 + Math.floor(Math.random() * 90);
// Short graces so the tests do not wait two minutes.
const HOST_GRACE_MS = 3000;
const GUEST_GRACE_MS = 2000;

const hex = (n) => randomBytes(n).toString('hex');
const sha = (s) => createHash('sha256').update(s).digest('hex');

class Client {
  constructor(base, path, headers = {}) {
    this.queue = [];
    this.waiters = [];
    this.ws = new WebSocket(base + path, { headers });
    this.opened = new Promise((resolve, reject) => {
      this.ws.on('open', resolve);
      this.ws.on('unexpected-response', (_req, res) => reject(Object.assign(new Error(`HTTP ${res.statusCode}`), { status: res.statusCode })));
      this.ws.on('error', reject);
    });
    this.closed = new Promise((resolve) => this.ws.on('close', (code, reason) => resolve({ code, reason: String(reason) })));
    this.ws.on('message', (data) => {
      const text = String(data);
      const value = text === 'pong' ? 'pong' : JSON.parse(text);
      const waiter = this.waiters.findIndex((w) => w.match(value));
      if (waiter >= 0) this.waiters.splice(waiter, 1)[0].resolve(value);
      else this.queue.push(value);
    });
  }

  send(value) {
    this.ws.send(typeof value === 'string' ? value : JSON.stringify(value));
  }

  /// The next message with this op (or matching the function).
  next(op, timeout = 4000) {
    const match = typeof op === 'function' ? op : (v) => v && v.op === op;
    const found = this.queue.findIndex(match);
    if (found >= 0) return Promise.resolve(this.queue.splice(found, 1)[0]);
    return new Promise((resolve, reject) => {
      const waiter = { match, resolve };
      this.waiters.push(waiter);
      setTimeout(() => {
        const i = this.waiters.indexOf(waiter);
        if (i >= 0) {
          this.waiters.splice(i, 1);
          reject(new Error(`timed out waiting for ${typeof op === 'string' ? op : 'a message'}`));
        }
      }, timeout);
    });
  }

  /// Nothing with this op arrives for a while.
  async none(op, wait = 400) {
    await sleep(wait);
    assert.equal(this.queue.filter((v) => v && v.op === op).length, 0, `unexpected ${op}`);
  }

  close() {
    this.ws.close();
  }

  kill() {
    this.ws.terminate();
  }
}

async function refused(base, path, headers) {
  const client = new Client(base, path, headers);
  try {
    await client.opened;
  } catch (err) {
    return err.status;
  }
  client.close();
  return 101;
}

async function startHost(base, room = hex(16), token = hex(32)) {
  const host = new Client(base, `/v1/host/${room}`, { 'X-Ytmp-Host': token });
  await host.opened;
  const ready = await host.next('ready');
  return { host, room, token, ready };
}

async function advertise(host, lookup = hex(32), ttl = 600) {
  host.send({ op: 'advertise', lookup, ttl });
  const answer = await host.next('advertised');
  assert.equal(answer.lookup, lookup);
  return { lookup, ok: answer.ok };
}

async function join(base, lookup) {
  const guest = new Client(base, `/v1/join/${lookup}`);
  await guest.opened;
  const you = await guest.next('you');
  return { guest, you };
}

// `--limits`: only the per-address limits, with them on (as deployed).
const LIMITS = process.argv.includes('--limits');
const tests = [];
const limitTests = [];
const test = (name, fn) => tests.push({ name, fn });
const limitTest = (name, fn) => limitTests.push({ name, fn });

limitTest('without someone in the room, no TURN (and STUN only without a TURN key)', async (base) => {
  const { host } = await startHost(base);
  host.send({ op: 'ice' });
  const { servers } = await host.next('ice');
  assert.ok(servers.length >= 1 && servers.every((s) => s.urls.every((u) => /^stun:/.test(u))), JSON.stringify(servers));
  host.send({ op: 'end' });
});

limitTest('one address may not open rooms or guess codes without end', async (base) => {
  // wrangler.toml: 10 rooms and 20 code guesses a minute per address.
  const until429 = async (tries, open) => {
    let fine = 0;
    for (let i = 0; i < tries; i += 1) {
      const status = await open();
      if (status === 429) return fine;
      fine += 1;
    }
    throw new Error(`never refused in ${tries} tries`);
  };
  const rooms = await until429(11, () => refused(base, `/v1/host/${hex(16)}`, { 'X-Ytmp-Host': hex(32) }));
  assert.ok(rooms <= 10, `${rooms} rooms in a minute`);
  const guesses = await until429(21, () => refused(base, `/v1/join/${hex(32)}`));
  assert.ok(guesses <= 20, `${guesses} guesses in a minute`);
});

test('health and bad paths', async (base, http) => {
  assert.equal(await (await fetch(`${http}/v1/health`)).text(), 'ok');
  assert.equal((await fetch(`${http}/v1/nope`)).status, 404);
  assert.equal((await fetch(`${http}/v1/host/${hex(16)}`)).status, 426, 'a WebSocket is required');
  assert.equal(await refused(base, `/v1/host/NOT-HEX`), 404);
  assert.equal(await refused(base, `/v1/join/${hex(16)}`), 404, 'lookups are 64 hex characters');
});

test('the host owns its room', async (base) => {
  const { host, room, ready } = await startHost(base);
  assert.deepEqual(ready.guests, []);
  assert.equal(await refused(base, `/v1/host/${room}`, { 'X-Ytmp-Host': hex(32) }), 403, 'another token');
  assert.equal(await refused(base, `/v1/host/${room}`), 403, 'no token');
  host.send({ op: 'end' });
});

test('guests join by lookup, talk only to the host, and a sixth does not fit', async (base) => {
  const { host } = await startHost(base);
  assert.equal(await refused(base, `/v1/join/${hex(32)}`), 404, 'unknown lookup');
  const { lookup, ok } = await advertise(host);
  assert.ok(ok);

  const one = await join(base, lookup);
  assert.equal(one.you.member, 'g1');
  assert.match(one.you.ticket, /^[0-9a-f]{64}$/);
  assert.deepEqual(await host.next('joined'), { op: 'joined', member: 'g1' });

  one.guest.send({ op: 'send', data: { k: 'hello', box: 'x' } });
  assert.deepEqual(await host.next('msg'), { op: 'msg', from: 'g1', data: { k: 'hello', box: 'x' } });

  const two = await join(base, lookup);
  assert.equal(two.you.member, 'g2');
  await host.next('joined');
  const more = [];
  for (const member of ['g3', 'g4', 'g5']) {
    const extra = await join(base, lookup);
    assert.equal(extra.you.member, member);
    await host.next('joined');
    more.push(extra);
  }
  assert.equal(await refused(base, `/v1/join/${lookup}`), 409, 'host + 5 guests');
  for (const extra of more) {
    extra.guest.send({ op: 'leave' });
    await host.next('left');
  }

  // To everyone, and to one.
  host.send({ op: 'send', to: '*', data: { k: 'room', n: 1 } });
  assert.deepEqual((await one.guest.next('msg')).data, { k: 'room', n: 1 });
  assert.deepEqual((await two.guest.next('msg')).data, { k: 'room', n: 1 });
  host.send({ op: 'send', to: 'g2', data: { k: 'room', n: 2 } });
  assert.equal((await two.guest.next('msg')).data.n, 2);
  await one.guest.none('msg');

  // A guest cannot pretend to be the host or send to the other guest.
  two.guest.send({ op: 'send', to: 'g1', data: { k: 'x' } });
  assert.equal((await host.next('msg')).from, 'g2');
  await one.guest.none('msg');
  two.guest.send({ op: 'kick', member: 'g1' });
  await one.guest.none('left');

  // Keepalives are answered by the relay itself.
  one.guest.send('ping');
  assert.equal(await one.guest.next((v) => v === 'pong'), 'pong');

  host.send({ op: 'end' });
  assert.equal((await one.guest.next('ended')).reason, 'ended');
  assert.equal((await one.guest.closed).code, 4000);
  assert.equal((await two.guest.closed).code, 4000);
  assert.equal(await refused(base, `/v1/join/${lookup}`), 404, 'the code is gone with the room');
});

test('guests joining at the same moment get different seats, and never a sixth', async (base) => {
  const { host } = await startHost(base);
  const { lookup } = await advertise(host);
  // Eight at once: five get in (each with its own member id), three are told the room is full.
  const attempts = await Promise.allSettled(Array.from({ length: 8 }, () => join(base, lookup)));
  const joined = attempts.filter((a) => a.status === 'fulfilled').map((a) => a.value);
  const members = joined.map((j) => j.you.member);
  assert.equal(new Set(members).size, members.length, `no member id twice: ${members}`);
  assert.equal(joined.length, 5, `host + 5 guests: ${members}`);
  assert.ok(attempts.filter((a) => a.status === 'rejected').every((a) => a.reason.status === 409));
  for (const { guest } of joined) guest.close();
  host.send({ op: 'end' });
});

test('removing a guest closes it and frees its seat', async (base) => {
  const { host } = await startHost(base);
  const { lookup } = await advertise(host);
  const one = await join(base, lookup);
  await host.next('joined');
  host.send({ op: 'kick', member: 'g1' });
  assert.deepEqual(await host.next('left'), { op: 'left', member: 'g1', reason: 'kicked' });
  assert.equal((await one.guest.closed).code, 4001);
  assert.equal(
    await refused(base, `/v1/rejoin/${one.you.room}`, { 'X-Ytmp-Member': 'g1', 'X-Ytmp-Ticket': one.you.ticket }),
    403,
    'a removed guest cannot come back with its ticket',
  );
  const again = await join(base, lookup);
  assert.equal(again.you.member, 'g2');
  host.send({ op: 'end' });
});

test('a guest who leaves or drops: the seat, the ticket and the grace', async (base) => {
  const { host, room } = await startHost(base);
  const { lookup } = await advertise(host);
  const one = await join(base, lookup);
  assert.equal(one.you.room, room);
  await host.next('joined');

  one.guest.kill();
  assert.deepEqual(await host.next('left'), { op: 'left', member: 'g1', reason: 'dropped' });
  assert.equal(
    await refused(base, `/v1/rejoin/${room}`, { 'X-Ytmp-Member': 'g1', 'X-Ytmp-Ticket': hex(32) }),
    403,
    'a wrong ticket',
  );
  const back = new Client(base, `/v1/rejoin/${room}`, { 'X-Ytmp-Member': 'g1', 'X-Ytmp-Ticket': one.you.ticket });
  await back.opened;
  assert.equal((await back.next('you')).member, 'g1');
  assert.deepEqual(await host.next('joined'), { op: 'joined', member: 'g1', resumed: true });

  // Dropped and not back in time: the seat goes.
  back.kill();
  await host.next('left');
  assert.deepEqual(await host.next('left', GUEST_GRACE_MS + 4000), { op: 'left', member: 'g1', reason: 'gone' });

  const two = await join(base, lookup);
  await host.next('joined');
  two.guest.send({ op: 'leave' });
  assert.deepEqual(await host.next('left'), { op: 'left', member: 'g2', reason: 'left' });
  assert.equal((await two.guest.closed).code, 1000);
  host.send({ op: 'end' });
});

test('the host drops and comes back; or does not, and the room ends', async (base) => {
  const { host, room, token } = await startHost(base);
  const { lookup } = await advertise(host);
  const one = await join(base, lookup);
  await host.next('joined');

  host.kill();
  await one.guest.next('host-away');
  assert.equal(await refused(base, `/v1/join/${lookup}`), 503, 'nobody can join while the host is away');
  const back = new Client(base, `/v1/host/${room}`, { 'X-Ytmp-Host': token });
  await back.opened;
  assert.deepEqual((await back.next('ready')).guests, ['g1']);
  await one.guest.next('host-back');

  // A second connection of the host replaces the first.
  const again = new Client(base, `/v1/host/${room}`, { 'X-Ytmp-Host': token });
  await again.opened;
  await again.next('ready');
  assert.equal((await back.closed).code, 4002);
  await one.guest.none('host-away');

  again.kill();
  await one.guest.next('host-away');
  assert.equal((await one.guest.next('ended', HOST_GRACE_MS + 4000)).reason, 'host-gone');
  assert.equal((await one.guest.closed).code, 4000);
  assert.equal(await refused(base, `/v1/join/${lookup}`), 404);
});

test('lookups belong to one room, expire and can be withdrawn', async (base) => {
  const a = await startHost(base);
  const b = await startHost(base);
  const { lookup } = await advertise(a.host);
  assert.equal((await advertise(b.host, lookup)).ok, false, 'taken by another room');
  assert.equal((await advertise(a.host, lookup, 900)).ok, true, 'the same room may renew it');

  a.host.send({ op: 'advertise', lookup: hex(32), ttl: 999999 });
  await a.host.none('advertised');

  const short = await advertise(a.host, hex(32), 1);
  await sleep(1500);
  assert.equal(await refused(base, `/v1/join/${short.lookup}`), 404, 'expired');

  a.host.send({ op: 'withdraw', lookup });
  await sleep(200);
  assert.equal(await refused(base, `/v1/join/${lookup}`), 404, 'withdrawn');
  assert.equal((await advertise(b.host, lookup)).ok, true, 'free again');
  a.host.send({ op: 'end' });
  b.host.send({ op: 'end' });
});

test('oversized, binary and floods', async (base) => {
  const { host } = await startHost(base);
  const { lookup } = await advertise(host);
  const one = await join(base, lookup);
  await host.next('joined');
  // A flood: the rest is dropped (and said once).
  for (let i = 0; i < 130; i += 1) one.guest.send({ op: 'send', data: { i } });
  assert.equal((await one.guest.next('error')).error, 'rate');
  await sleep(300);
  const got = host.queue.filter((v) => v.op === 'msg').length;
  assert.ok(got <= 120 && got >= 100, `${got} forwarded`);

  // A connection offer (a few KiB) goes through; something huge does not.
  await sleep(10_500);
  one.guest.send({ op: 'send', data: { k: 'room', box: 'x'.repeat(20 * 1024) } });
  assert.equal((await host.next((v) => v && v.op === 'msg' && v.data.box)).data.box.length, 20 * 1024);
  one.guest.send({ op: 'send', data: { big: 'x'.repeat(33 * 1024) } });
  assert.equal((await one.guest.closed).code, 1009);
  const two = await join(base, lookup);
  two.guest.ws.send(Buffer.from([1, 2, 3]));
  assert.equal((await two.guest.closed).code, 1003);
  host.send({ op: 'end' });
});

test('ICE servers for window sharing: STUN without a TURN key, a few times per connection', async (base) => {
  const { host } = await startHost(base);
  const { lookup } = await advertise(host);
  const one = await join(base, lookup);
  await host.next('joined');
  for (const who of [host, one.guest]) {
    who.send({ op: 'ice' });
    const { servers } = await who.next('ice');
    assert.ok(servers.length >= 1);
    assert.ok(servers.every((s) => s.urls.every((u) => /^stun:/.test(u))), JSON.stringify(servers));
    assert.ok(servers.every((s) => !s.username && !s.credential), 'no TURN without its key');
  }
  for (let i = 0; i < 4; i += 1) one.guest.send({ op: 'ice' });
  let answers = 0;
  for (;;) {
    try {
      await one.guest.next('ice', 600);
      answers += 1;
    } catch {
      break;
    }
  }
  assert.equal(answers, 2, 'three per connection in all');
  host.send({ op: 'end' });
});

test('the relay never sees codes, only lookups', async () => {
  // What the app sends is SHA-256("ytmp-together/lookup/v1" ‖ code).
  assert.equal(sha('ytmp-together/lookup/v1PTQ7K20NPW5J'), '0314231bc05e38b86467399c7b5a597df14639b1cc0f690c7e937f072f77a01c');
});

// The local relay, while it runs (stopped however the tests end).
let child = null;

// Stops wrangler and the workerd it started. Elsewhere they share a process
// group (one signal reaches both); Windows has none, so the whole tree goes
// by id, or workerd would keep running (and holding the port) afterwards.
function stopRelay() {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  if (WINDOWS) {
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
  } else {
    try {
      process.kill(-child.pid, 'SIGINT');
    } catch {
      child.kill('SIGINT');
    }
  }
  child = null;
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    stopRelay();
    process.exit(130);
  });
}

async function startRelay() {
  if (!existsSync(WRANGLER)) throw new Error(`wrangler is not installed in ${HERE} (run npm install first)`);
  child = spawn(
    process.execPath,
    // The tests open dozens of rooms a minute from one address: the
    // per-address limits are off, except for the run that checks them.
    [WRANGLER, 'dev', '--ip', '127.0.0.1', '--port', String(PORT), '--var', `HOST_GRACE_MS:${HOST_GRACE_MS}`, '--var', `GUEST_GRACE_MS:${GUEST_GRACE_MS}`, ...(LIMITS ? [] : ['--var', 'RATE_LIMITS:off'])],
    {
      cwd: HERE,
      stdio: ['ignore', 'pipe', 'pipe'],
      // Its own process group (not on Windows, where it would only open a console window).
      detached: !WINDOWS,
      windowsHide: true,
      env: { ...process.env, WRANGLER_SEND_METRICS: 'false' },
    },
  );
  let log = '';
  let failure = null;
  child.stdout.on('data', (d) => (log += d));
  child.stderr.on('data', (d) => (log += d));
  child.on('error', (err) => (failure = err));
  const deadline = Date.now() + 60_000;
  for (;;) {
    try {
      if ((await fetch(`http://127.0.0.1:${PORT}/v1/health`)).ok) return `ws://127.0.0.1:${PORT}`;
    } catch {
      // Not up yet.
    }
    // No point waiting a minute for one that could not start or already ended.
    const ended = failure || (child.exitCode !== null ? `wrangler dev exited with code ${child.exitCode}` : null);
    if (ended || Date.now() > deadline) {
      console.error(log);
      throw new Error(`wrangler dev did not start${ended ? `: ${ended}` : ''}`);
    }
    await sleep(300);
  }
}

async function main() {
  const base = process.env.RELAY || (await startRelay());
  const http = base.replace(/^ws/, 'http');
  let failed = 0;
  const chosen = LIMITS ? limitTests : tests;
  for (const { name, fn } of chosen) {
    try {
      await fn(base, http);
      console.log(`ok - ${name}`);
    } catch (err) {
      failed += 1;
      console.log(`FAIL - ${name}\n  ${err.stack}`);
    }
  }
  console.log(failed ? `${failed} failed` : `all ${chosen.length} passed`);
  return failed;
}

main().then(
  (failed) => {
    stopRelay();
    process.exit(failed ? 1 : 0);
  },
  (err) => {
    stopRelay();
    console.error(err);
    process.exit(1);
  },
);
