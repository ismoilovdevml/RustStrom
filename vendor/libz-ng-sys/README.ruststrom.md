# Vendored libz-ng-sys 1.1.33 (patched)

Sources of crates.io `libz-ng-sys` 1.1.33 with three changes:

1. `Cargo.toml` selects the crate's own `cc`-based build script (`zng/cc.rs`, maintained
   upstream in rust-lang/libz-sys) instead of `zng/cmake.rs`, and depends on `cc` instead of
   `cmake`.
2. `zng/cc.rs` also compiles `arch/generic/crc32_chorba_c.c`; without it zlib-ng 2.2 aborts with
   "Zlib-ng functable failed initialization!" on the first (de)compression.
3. `zng/cc.rs` builds only the portable generic code unless `ZLIB_NG_SIMD` is set: upstream
   passes `-mavx2`/`-mavx512*` to every source file (CMake only to the SIMD kernels), which can
   produce instructions the CPU running the binary does not have. The generic build defines
   `WITH_ALL_FALLBACKS` (otherwise x86_64 expects SSE2 kernels and the functable is incomplete).

Why: `pingora-core` 0.9 enables `flate2/zlib-ng`, whose upstream build needs a `cmake` binary.
With this patch RustStrom builds with only a C compiler (build hosts, Docker, static musl).
Remove the `[patch.crates-io]` entry in the root `Cargo.toml` to use the upstream crate again
(then `cmake` is required).
