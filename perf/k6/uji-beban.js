// Uji beban PoolRouter F0 (SHAA-205): profil K-22 dari ADR-0001 rev 15 §1.
// Hanya untuk lingkungan lokal Podman dengan data sintetis.
//
// Env (semua wajib kecuali yang punya default):
//   BASE_URL            URL gateway downstream, mis. http://poolrouter-gateway:8080
//   ENDPOINT_ID         UUID downstream endpoint sync atau async (diisi saat setup)
//   ENDPOINT_ID_ASYNC   UUID endpoint async, dipakai bila MODE=mix
//   POOLROUTER_API_KEY  API key downstream (dari environment, bukan repo)
//   MODE                sync | async | mix (default sync)
//   RATE                request/detik total (default 11.6 = K-22 rata-rata)
//   SYNC_SHARE          porsi sync pada MODE=mix (default 0.2, K-10 contoh 20%)
//   DURATION            durasi uji, mis. 10m (default 10m); semua durasi ikut dihitung
//   POLL_SAMPLE         porsi async yang di-poll sampai terminal (default 0.1)
//   POLL_TIMEOUT_S      batas tunggu terminal per sampel (default 90)
//
// Data: NIK acak 16 digit berawalan 3201 dibuat di sini; hasilnya tidak
// berasal dari data penduduk nyata.

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Trend } from 'k6/metrics';

const BASE_URL = __ENV.BASE_URL;
const ENDPOINT_ID = __ENV.ENDPOINT_ID;
const ENDPOINT_ID_ASYNC = __ENV.ENDPOINT_ID_ASYNC || __ENV.ENDPOINT_ID;
const API_KEY = __ENV.POOLROUTER_API_KEY;
const MODE = __ENV.MODE || 'sync';
const RATE = Number(__ENV.RATE || '11.6');
const SYNC_SHARE = Number(__ENV.SYNC_SHARE || '0.2');
const DURATION = __ENV.DURATION || '10m';
const POLL_SAMPLE = Number(__ENV.POLL_SAMPLE || '0.1');
const POLL_TIMEOUT_S = Number(__ENV.POLL_TIMEOUT_S || '90');

if (!BASE_URL || !ENDPOINT_ID || !API_KEY) {
  throw new Error('BASE_URL, ENDPOINT_ID, dan POOLROUTER_API_KEY wajib diisi dari environment');
}
if (!['sync', 'async', 'mix'].includes(MODE)) {
  throw new Error(`MODE tidak dikenal: ${MODE}`);
}

const syncLatency = new Trend('sync_latency_ms', true);
const asyncEnqueueLatency = new Trend('async_enqueue_ms', true);
const asyncEndToEnd = new Trend('async_end_to_end_ms', true);
const asyncTerminal = new Counter('async_terminal_total');
const asyncTimeout = new Counter('async_poll_timeout_total');
const http5xx = new Counter('http_5xx_total');
const http429 = new Counter('http_429_total');

const TERMINAL = ['done', 'failed', 'expired', 'cancelled'];

function syntheticNik() {
  let s = '3201';
  for (let i = 0; i < 12; i++) s += Math.floor(Math.random() * 10);
  return s;
}

function body() {
  return JSON.stringify({
    input: { NAMA_LGKP: 'Peserta Sintetis', NIK: syntheticNik() },
    metadata: { status_penerima_manfaat: 'sesuai' },
  });
}

function headers(extra) {
  return Object.assign({ 'Content-Type': 'application/json', 'X-API-Key': API_KEY }, extra || {});
}

function countStatus(res) {
  if (res.status === 429) http429.add(1);
  if (res.status >= 500) http5xx.add(1);
}

function doSync() {
  const res = http.post(`${BASE_URL}/v1/endpoints/${ENDPOINT_ID}/sync`, body(), {
    headers: headers(),
    tags: { mode: 'sync' },
  });
  countStatus(res);
  syncLatency.add(res.timings.duration);
  check(res, { 'sync: tidak ada 5xx': (r) => r.status < 500 });
}

function doAsync() {
  const res = http.post(`${BASE_URL}/v1/endpoints/${ENDPOINT_ID_ASYNC}/async`, body(), {
    headers: headers(),
    tags: { mode: 'async' },
  });
  countStatus(res);
  asyncEnqueueLatency.add(res.timings.duration);
  check(res, { 'async: enqueue diterima': (r) => r.status === 200 || r.status === 202 });
  if (Math.random() >= POLL_SAMPLE || res.status >= 300) return;

  const queueId = res.json('queue_id') || res.json('request_id');
  if (!queueId) return;
  const start = Date.now();
  while ((Date.now() - start) / 1000 < POLL_TIMEOUT_S) {
    const st = http.get(`${BASE_URL}/v1/requests/${queueId}`, { headers: headers(), tags: { mode: 'poll' } });
    countStatus(st);
    const status = st.json('status');
    if (TERMINAL.includes(status)) {
      asyncEndToEnd.add(Date.now() - start);
      asyncTerminal.add(1, { status });
      return;
    }
    sleep(0.25);
  }
  asyncTimeout.add(1);
}

function arrival(name, exec, rate) {
  return {
    executor: 'constant-arrival-rate',
    rate: Math.max(1, Math.round(rate)),
    timeUnit: '1s',
    duration: DURATION,
    preAllocatedVUs: Math.max(10, Math.ceil(rate * 2)),
    maxVUs: Math.max(200, Math.ceil(rate * 40)),
    exec,
    gracefulStop: '60s',
    tags: { scenario: name },
    // no startTime: the whole DURATION is measured; warm-up is a separate short run, not a delay

  };
}

export const options = {
  summaryTrendStats: ['avg', 'min', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
  scenarios: (() => {
    if (MODE === 'sync') return { sync: arrival('sync', 'runSync', RATE) };
    if (MODE === 'async') return { async: arrival('async', 'runAsync', RATE) };
    return {
      sync: arrival('sync', 'runSync', RATE * SYNC_SHARE),
      async: arrival('async', 'runAsync', RATE * (1 - SYNC_SHARE)),
    };
  })(),
};

export function runSync() {
  doSync();
}

export function runAsync() {
  doAsync();
}
