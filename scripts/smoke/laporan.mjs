#!/usr/bin/env node
// scripts/smoke/laporan.mjs (ARC-378, spec smoke-test bagian 3 langkah 5). Gabung reporter JSON
// Playwright (PLAYWRIGHT_JSON_OUTPUT_NAME, project api+ui) dan baris proses shell (G0) jadi satu
// smoke-report.json = {sha, hasil: [{id, jenis: api|rute|proses, peran, status: lulus|gagal|lewat,
// detik, pesan}]}. Id dari anotasi test `{type: 'inventaris', description: '<id>'}` (lihat
// frontend/smoke/fixtures.ts); peran opsional dari anotasi `{type: 'peran', description: '<peran>'}`.
//
//   node scripts/smoke/laporan.mjs <playwright-report.json> <proc-lines.txt> <sha> <output.json>
//
// proc-lines.txt: satu baris per proses `id=proc:x status=lulus|gagal detik=1.2 pesan=...`.
// Stdout mencetak `LULUS n/N` (spec bagian 3); exit 0 hanya bila N = jumlah hasil dan tanpa
// gagal/lewat.

import { readFileSync, writeFileSync } from 'node:fs';

export function anotasi(annotations, type) {
  const a = (annotations ?? []).find((x) => x.type === type);
  return a ? a.description : undefined;
}

function jenisDariId(id) {
  if (id.startsWith('route:')) return 'rute';
  if (id.startsWith('proc:')) return 'proses';
  return 'api';
}

/** Jalan rekursif lewat suites[].specs[]/suites[] bersarang reporter JSON Playwright. */
export function hasilDariLaporanPlaywright(report) {
  const hasil = [];
  const jalanSuite = (suite) => {
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests ?? []) {
        const id = anotasi(test.annotations, 'inventaris');
        if (!id) continue;
        const peran = anotasi(test.annotations, 'peran') ?? null;
        const results = test.results ?? [];
        const akhir = results[results.length - 1];
        const statusMentah = akhir?.status ?? 'skipped';
        const status =
          statusMentah === 'passed' ? 'lulus' : statusMentah === 'skipped' ? 'lewat' : 'gagal';
        const detik = results.reduce((acc, r) => acc + (r.duration ?? 0), 0) / 1000;
        const pesan = status === 'gagal' ? String(akhir?.error?.message ?? '').slice(0, 500) : '';
        hasil.push({ id, jenis: jenisDariId(id), peran, status, detik, pesan });
      }
    }
    for (const sub of suite.suites ?? []) jalanSuite(sub);
  };
  for (const suite of report.suites ?? []) jalanSuite(suite);
  return hasil;
}

/** Baris `id=... status=... detik=... pesan=...` dari G0 (scripts/smoke.sh). */
export function hasilDariBarisProses(teks) {
  const hasil = [];
  for (const baris of teks.split('\n')) {
    if (!baris.trim()) continue;
    const ambil = (k) => baris.match(new RegExp(`(?:^| )${k}=(\\S+)`))?.[1];
    const id = ambil('id');
    if (!id) continue;
    const status = ambil('status') ?? 'gagal';
    const detik = Number(ambil('detik') ?? '0');
    const pesan = baris.match(/ pesan=(.*)$/)?.[1] ?? '';
    hasil.push({ id, jenis: jenisDariId(id), peran: null, status, detik, pesan });
  }
  return hasil;
}

function main() {
  const [laporanPath, procPath, sha, outPath] = process.argv.slice(2);
  if (!laporanPath || !procPath || !sha || !outPath) {
    console.error(
      'pakai: node scripts/smoke/laporan.mjs <playwright-report.json> <proc-lines.txt> <sha> <output.json>',
    );
    process.exitCode = 2;
    return;
  }
  const report = JSON.parse(readFileSync(laporanPath, 'utf8'));
  const teksProses = readFileSync(procPath, 'utf8');
  const hasil = [...hasilDariLaporanPlaywright(report), ...hasilDariBarisProses(teksProses)];
  writeFileSync(outPath, JSON.stringify({ sha, hasil }, null, 2) + '\n');
  const lulus = hasil.filter((h) => h.status === 'lulus').length;
  console.log(`LULUS ${lulus}/${hasil.length}`);
  if (hasil.length === 0 || lulus !== hasil.length) process.exitCode = 1;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
