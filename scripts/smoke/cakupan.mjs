#!/usr/bin/env node
// Cek cakupan smoke (AC4, spec.md bagian 4). Node 24, tanpa dependensi.
//
// Sumber kebenaran: operationId admin-api.yaml + downstream-api.yaml, proc:* dari
// inventaris.md, dan rute dari ./cakupan-rute.mjs (modul milik S3/ARC-379). Dibandingkan
// dengan id anotasi `{type: 'inventaris', description: '<id>'}` hasil
// `playwright test --list --reporter=json`.
//
// Dipakai sebagai `node scripts/smoke/cakupan.mjs` (langkah pertama `make smoke` dan
// `make smoke-cakupan`).

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(DIR, '..', '..');

// Regex sama persis dengan bagian 1 spec.md: operationId OpenAPI berindentasi operasi
// (dua level di bawah `paths:`, yaitu enam spasi).
const OPERATION_ID_RE = /^      operationId: (\S+)$/gm;

// inventaris.md bagian "Proses (4)": proc:webserver, proc:controller, proc:worker,
// proc:backend-cli. Daftar literal, bukan hasil parsing, karena inventaris.md adalah
// dokumentasi naratif (tabel markdown), bukan sumber yang dibuat untuk di-parse.
export const PROC_IDS = Object.freeze([
  'proc:webserver',
  'proc:controller',
  'proc:worker',
  'proc:backend-cli',
]);

export function extractOperationIds(teks) {
  const hasil = [];
  for (const m of teks.matchAll(OPERATION_ID_RE)) {
    hasil.push(m[1]);
  }
  return hasil;
}

export function expectedIds({ openapiTexts, procIds, ruteIds }) {
  const ops = openapiTexts.flatMap((t) => extractOperationIds(t));
  return [...ops, ...procIds, ...ruteIds];
}

function pushAnnotationIds(test, out) {
  for (const a of test.annotations ?? []) {
    if (a.type === 'inventaris' && typeof a.description === 'string') {
      out.add(a.description);
    }
  }
}

function walkSuite(suite, out) {
  for (const spec of suite.specs ?? []) {
    for (const t of spec.tests ?? []) {
      pushAnnotationIds(t, out);
    }
  }
  for (const sub of suite.suites ?? []) {
    walkSuite(sub, out);
  }
}

export function testedIdsFromPlaywrightList(listJson) {
  const out = new Set();
  for (const suite of listJson.suites ?? []) {
    walkSuite(suite, out);
  }
  return out;
}

export function diffCoverage(expectedIdList, testedIdSet) {
  const expectedSet = new Set(expectedIdList);
  const missing = expectedIdList.filter((id) => !testedIdSet.has(id));
  const extra = [...testedIdSet].filter((id) => !expectedSet.has(id));
  return { missing, extra };
}

// S2 (ARC-378): modul rute milik S3 (ARC-379) belum ada. Supaya rute tidak pernah lolos
// cakupan secara diam-diam sebelum S3 merge (spec.md bagian 4), modul yang tidak ada
// adalah galat yang jelas, bukan daftar rute kosong.
export async function loadRuteIds(cakupanRutePath) {
  let mod;
  try {
    mod = await import(cakupanRutePath);
  } catch (err) {
    if (err?.code === 'ERR_MODULE_NOT_FOUND') {
      throw new Error('cakupan rute belum ada');
    }
    throw err;
  }
  if (!Array.isArray(mod.ruteIds)) {
    throw new Error(`${cakupanRutePath} tidak mengekspor ruteIds (array)`);
  }
  return mod.ruteIds;
}

function runPlaywrightList() {
  const config = path.join(REPO_ROOT, 'frontend', 'playwright.smoke.config.ts');
  const output = execFileSync(
    'npx',
    ['playwright', 'test', '--list', '--reporter=json', '-c', config],
    { cwd: path.join(REPO_ROOT, 'frontend'), encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  );
  return JSON.parse(output);
}

async function main() {
  const adminApi = readFileSync(path.join(REPO_ROOT, 'backend', 'openapi', 'admin-api.yaml'), 'utf8');
  const downstreamApi = readFileSync(
    path.join(REPO_ROOT, 'backend', 'openapi', 'downstream-api.yaml'),
    'utf8',
  );

  let ruteIds;
  try {
    ruteIds = await loadRuteIds(path.join(DIR, 'cakupan-rute.mjs'));
  } catch (err) {
    console.error(`cakupan gagal: ${err.message}`);
    process.exitCode = 1;
    return;
  }

  const expected = expectedIds({
    openapiTexts: [adminApi, downstreamApi],
    procIds: PROC_IDS,
    ruteIds,
  });

  const listJson = runPlaywrightList();
  const tested = testedIdsFromPlaywrightList(listJson);

  const { missing, extra } = diffCoverage(expected, tested);
  if (missing.length === 0 && extra.length === 0) {
    console.log(`cakupan lengkap: ${expected.length} id`);
    return;
  }

  if (missing.length > 0) {
    console.error(`id tanpa smoke test (${missing.length}):`);
    for (const id of missing) console.error(`  - ${id}`);
  }
  if (extra.length > 0) {
    console.error(`id smoke test tanpa pasangan di sumber kebenaran (${extra.length}):`);
    for (const id of extra) console.error(`  - ${id}`);
  }
  process.exitCode = 1;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
