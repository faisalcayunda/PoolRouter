// Test untuk scripts/smoke/laporan.mjs (ARC-378, spec smoke-test bagian 3 langkah 5). Node test
// runner, tanpa dependensi.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { hasilDariLaporanPlaywright, hasilDariBarisProses, anotasi } from './laporan.mjs';

function laporanPlaywright(tests) {
  return { suites: [{ specs: [{ tests }] }] };
}

function t(id, { status = 'passed', peran, durasi = 10, pesan } = {}) {
  const annotations = [{ type: 'inventaris', description: id }];
  if (peran) annotations.push({ type: 'peran', description: peran });
  return {
    annotations,
    results: [{ status, duration: durasi, error: pesan ? { message: pesan } : undefined }],
  };
}

test('anotasi mengambil description dari type yang cocok, undefined bila tidak ada', () => {
  assert.equal(anotasi([{ type: 'inventaris', description: 'opA' }], 'inventaris'), 'opA');
  assert.equal(anotasi([{ type: 'inventaris', description: 'opA' }], 'peran'), undefined);
  assert.equal(anotasi(undefined, 'inventaris'), undefined);
});

test('hasilDariLaporanPlaywright: test tanpa anotasi inventaris diabaikan', () => {
  const report = laporanPlaywright([{ annotations: [], results: [{ status: 'passed' }] }]);
  assert.deepEqual(hasilDariLaporanPlaywright(report), []);
});

test('hasilDariLaporanPlaywright: passed/skipped/failed dipetakan ke lulus/lewat/gagal', () => {
  const report = laporanPlaywright([
    t('opA', { status: 'passed' }),
    t('opB', { status: 'skipped' }),
    t('opC', { status: 'failed', pesan: 'boom' }),
  ]);
  const hasil = hasilDariLaporanPlaywright(report);
  assert.deepEqual(
    hasil.map((h) => [h.id, h.status]),
    [
      ['opA', 'lulus'],
      ['opB', 'lewat'],
      ['opC', 'gagal'],
    ],
  );
  assert.equal(hasil[2].pesan, 'boom');
});

test('hasilDariLaporanPlaywright: jenis dari prefiks id (route:/proc:/lainnya=api)', () => {
  const report = laporanPlaywright([t('route:x'), t('proc:y'), t('opA')]);
  const hasil = hasilDariLaporanPlaywright(report);
  assert.deepEqual(
    hasil.map((h) => h.jenis),
    ['rute', 'proses', 'api'],
  );
});

test('hasilDariLaporanPlaywright: anotasi peran opsional dibawa, tanpa peran -> null', () => {
  const report = laporanPlaywright([t('opA', { peran: 'operator' }), t('opB')]);
  const hasil = hasilDariLaporanPlaywright(report);
  assert.equal(hasil[0].peran, 'operator');
  assert.equal(hasil[1].peran, null);
});

test('hasilDariLaporanPlaywright: suites bersarang ikut dijalani', () => {
  const report = {
    suites: [{ specs: [], suites: [{ specs: [{ tests: [t('opA')] }] }] }],
  };
  assert.deepEqual(
    hasilDariLaporanPlaywright(report).map((h) => h.id),
    ['opA'],
  );
});

test('hasilDariBarisProses: parse baris id/status/detik/pesan, baris kosong dilewati', () => {
  const teks = [
    'id=proc:webserver status=lulus detik=3',
    '',
    'id=proc:controller status=gagal detik=1 pesan=exit 1 tidak sehat',
  ].join('\n');
  const hasil = hasilDariBarisProses(teks);
  assert.deepEqual(hasil, [
    { id: 'proc:webserver', jenis: 'proses', peran: null, status: 'lulus', detik: 3, pesan: '' },
    {
      id: 'proc:controller',
      jenis: 'proses',
      peran: null,
      status: 'gagal',
      detik: 1,
      pesan: 'exit 1 tidak sehat',
    },
  ]);
});

test('hasilDariBarisProses: baris tanpa id diabaikan', () => {
  assert.deepEqual(hasilDariBarisProses('status=lulus detik=1\n'), []);
});
