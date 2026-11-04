#!/bin/sh
# RUSTC_WORKSPACE_WRAPPER lane Docker (SHAA-401). Tidak mengubah apa pun: isinya hanya menjalankan rustc.
# Gunanya path-nya: di-mount sebagai /lane/<kunci>/w.sh, dan cargo memasukkan wrapper ke hash unit crate workspace,
# jadi artefak crate workspace tiap lane (kunci) punya nama berbeda di target bersama. Dependensi tidak terpengaruh.
exec "$@"
