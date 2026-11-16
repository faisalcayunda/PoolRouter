// Test untuk scripts/smoke/cakupan.mjs (AC4). Node test runner, tanpa dependensi.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  extractOperationIds,
  expectedIds,
  testedIdsFromPlaywrightList,
  diffCoverage,
  testedProcIdsFromSmokeSh,
  loadRuteIds,
  PROC_IDS,
} from './cakupan.mjs';

const DIR = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.join(DIR, 'fixtures');

const OPENAPI_LENGKAP = `
paths:
  /a:
    get:
      operationId: opA
  /b:
    post:
      operationId: opB
`;

function playwrightList(ids) {
  // Bentuk minimal yang menyerupai output `playwright test --list --reporter=json`:
  // suite -> specs -> tests -> annotations.
  return {
    suites: [
      {
        specs: ids.map((id) => ({
          tests: [
            {
              annotations: [{ type: 'inventaris', description: id }],
            },
          ],
        })),
      },
    ],
  };
}

test('extractOperationIds mengambil tiap operationId berindentasi operasi', () => {
  assert.deepEqual(extractOperationIds(OPENAPI_LENGKAP), ['opA', 'opB']);
});

test('extractOperationIds mengabaikan operationId berindentasi lain (bukan bagian 1)', () => {
  const teks = 'operationId: salahIndentasi\n      operationId: benar\n';
  assert.deepEqual(extractOperationIds(teks), ['benar']);
});

test('PROC_IDS memuat empat id proses dari inventaris.md', () => {
  assert.deepEqual(
    [...PROC_IDS].sort(),
    ['proc:backend-cli', 'proc:controller', 'proc:webserver', 'proc:worker'].sort(),
  );
});

test('expectedIds menggabungkan operasi, proses, dan rute', () => {
  const ids = expectedIds({
    openapiTexts: [OPENAPI_LENGKAP],
    procIds: ['proc:x'],
    ruteIds: ['route:/y'],
  });
  assert.deepEqual(ids, ['opA', 'opB', 'proc:x', 'route:/y']);
});

test('testedIdsFromPlaywrightList membaca anotasi inventaris dari struktur bersarang', () => {
  const list = playwrightList(['opA', 'route:/y']);
  const tested = testedIdsFromPlaywrightList(list);
  assert.deepEqual([...tested].sort(), ['opA', 'route:/y']);
});

test('diffCoverage: operasi tanpa smoke test gagal (id hilang disebut)', () => {
  const expected = expectedIds({ openapiTexts: [OPENAPI_LENGKAP], procIds: [], ruteIds: [] });
  const tested = testedIdsFromPlaywrightList(playwrightList(['opA']));
  const { missing, extra } = diffCoverage(expected, tested);
  assert.deepEqual(missing, ['opB']);
  assert.deepEqual(extra, []);
});

test('diffCoverage: test tanpa operasi di sumber kebenaran dilaporkan sebagai extra', () => {
  const expected = expectedIds({ openapiTexts: [OPENAPI_LENGKAP], procIds: [], ruteIds: [] });
  const tested = testedIdsFromPlaywrightList(playwrightList(['opA', 'opB', 'opSiluman']));
  const { missing, extra } = diffCoverage(expected, tested);
  assert.deepEqual(missing, []);
  assert.deepEqual(extra, ['opSiluman']);
});

test('diffCoverage murni: proc:* tanpa anotasi Playwright selalu dilaporkan "missing"', () => {
  // proc:* diverifikasi lewat shell G0 (scripts/smoke.sh), bukan test Playwright, jadi
  // diffCoverage murni (tanpa pengetahuan proc:*) akan selalu melaporkannya hilang kecuali
  // di-gabung dengan testedProcIdsFromSmokeSh (lihat test AC4 di bawah).
  const expected = expectedIds({
    openapiTexts: [OPENAPI_LENGKAP],
    procIds: ['proc:x'],
    ruteIds: [],
  });
  const tested = testedIdsFromPlaywrightList(playwrightList(['opA', 'opB']));
  const { missing } = diffCoverage(expected, tested);
  assert.deepEqual(missing, ['proc:x']);
});

test('testedProcIdsFromSmokeSh: mengambil tiap proc:x dari baris catat_proc', () => {
  const teks = [
    'catat_proc proc:webserver sh -c "..."',
    'catat_proc proc:controller "$ENGINE" exec "$c" controller healthcheck',
    'bukan_catat_proc proc:palsu lainnya',
  ].join('\n');
  assert.deepEqual([...testedProcIdsFromSmokeSh(teks)], ['proc:webserver', 'proc:controller']);
});

test('AC4: proc tanpa baris catat_proc di smoke.sh tetap dilaporkan hilang', () => {
  // Repro Klaus: proc:* harus tetap gagal (exit 1) bila baris catat_proc-nya dihapus dari
  // smoke.sh -- "tested" untuk proc:* diambil statis dari smoke.sh, bukan disaring buta.
  const expected = expectedIds({ openapiTexts: [''], procIds: PROC_IDS, ruteIds: [] });
  const testedDariSmokeSh = testedProcIdsFromSmokeSh('catat_proc proc:webserver sh -c "..."');
  const tested = new Set([...testedIdsFromPlaywrightList(playwrightList([])), ...testedDariSmokeSh]);
  const { missing } = diffCoverage(expected, tested);
  assert.deepEqual(missing, ['proc:controller', 'proc:worker', 'proc:backend-cli']);
});

test('AC4: proc dengan baris catat_proc di smoke.sh tidak dilaporkan hilang', () => {
  const expected = expectedIds({ openapiTexts: [''], procIds: PROC_IDS, ruteIds: [] });
  const teksSmokeSh = PROC_IDS.map((id) => `catat_proc ${id} sh -c "..."`).join('\n');
  const tested = new Set([
    ...testedIdsFromPlaywrightList(playwrightList([])),
    ...testedProcIdsFromSmokeSh(teksSmokeSh),
  ]);
  const { missing } = diffCoverage(expected, tested);
  assert.deepEqual(missing, []);
});

test('diffCoverage: cakupan lengkap tidak melaporkan apa pun', () => {
  const expected = expectedIds({ openapiTexts: [OPENAPI_LENGKAP], procIds: [], ruteIds: [] });
  const tested = testedIdsFromPlaywrightList(playwrightList(['opA', 'opB']));
  const { missing, extra } = diffCoverage(expected, tested);
  assert.deepEqual(missing, []);
  assert.deepEqual(extra, []);
});

test('loadRuteIds: modul cakupan-rute.mjs belum ada -> pesan "cakupan rute belum ada"', async () => {
  const pathTidakAda = path.join(FIXTURES, 'cakupan-rute-tidak-ada.mjs');
  await assert.rejects(() => loadRuteIds(pathTidakAda), /cakupan rute belum ada/);
});

test('loadRuteIds: modul ada dan mengekspor ruteIds -> dikembalikan', async () => {
  const pathAda = path.join(FIXTURES, 'cakupan-rute-ada.mjs');
  const ids = await loadRuteIds(pathAda);
  assert.deepEqual(ids, ['route:/fixture']);
});

test('loadRuteIds: modul ada tapi tidak mengekspor ruteIds -> galat lain (bukan "belum ada")', async () => {
  const pathSalah = path.join(FIXTURES, 'cakupan-rute-tanpa-ekspor.mjs');
  await assert.rejects(() => loadRuteIds(pathSalah), (err) => {
    assert.ok(!/cakupan rute belum ada/.test(err.message));
    return true;
  });
});
