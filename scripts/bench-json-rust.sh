#!/usr/bin/env bash
# Build + run the JSON parser criterion bench (src/parsers/benches/json_parse.rs), or with `--xml`
# the XML one (benches/xml_parse.rs): compiles the native pieces the parsers reach into one archive
# and points RUSTFLAGS at it. Needs `bun bd` once. `--test` runs the crate's unit tests instead.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -d vendor/mimalloc ]; then
  echo "error: vendor/ not populated — run \`bun bd\` (or \`bun run build\`) once first" >&2
  exit 1
fi

SUP=target/bench-json-cdeps
mkdir -p "$SUP"
export BUN_CODEGEN_DIR=${BUN_CODEGEN_DIR:-$PWD/build/debug/codegen}
if [ ! -f "$BUN_CODEGEN_DIR/build_options.rs" ]; then
  echo "error: $BUN_CODEGEN_DIR/build_options.rs not found — run \`bun bd\` once first" >&2
  exit 1
fi
CC=${CC:-cc}
CXX=${CXX:-c++}

build() {
  local out=$1
  shift
  if [ ! -f "$out" ] || [ "${*: -1}" -nt "$out" ]; then "$@" -o "$out"; fi
}
MI_FLAGS=(-O2 -fPIC -ftls-model=initial-exec -DNDEBUG -D_GNU_SOURCE -DMI_STATIC_LIB
  -DMI_SKIP_COLLECT_ON_EXIT=1 -DMI_DEFAULT_ALLOW_THP=0)
build "$SUP/mimalloc.o" $CC "${MI_FLAGS[@]}" -Ivendor/mimalloc/include -c vendor/mimalloc/src/static.c
# C/C++ XML parsers for benches/xml_parse.rs to compare against (optional).
XML_C_DEFS=()
XML_C_LIBS=()
PUGI_VERSION=1.14
if [ ! -f "$SUP/pugixml-$PUGI_VERSION/src/pugixml.cpp" ]; then
  curl -fsSL "https://github.com/zeux/pugixml/releases/download/v$PUGI_VERSION/pugixml-$PUGI_VERSION.tar.gz" | tar -xz -C "$SUP" || true
fi
if [ -f "$SUP/pugixml-$PUGI_VERSION/src/pugixml.cpp" ]; then
  build "$SUP/pugixml.o" $CXX -O3 -fPIC -std=c++17 -DNDEBUG -c "$SUP/pugixml-$PUGI_VERSION/src/pugixml.cpp"
  XML_C_DEFS+=(-DHAVE_PUGIXML "-I$SUP/pugixml-$PUGI_VERSION/src")
fi
if [ -f /usr/include/expat.h ]; then XML_C_DEFS+=(-DHAVE_EXPAT); XML_C_LIBS+=(-Clink-arg=-lexpat); fi
if [ -d /usr/include/libxml2 ]; then XML_C_DEFS+=(-DHAVE_LIBXML2 -I/usr/include/libxml2); XML_C_LIBS+=(-Clink-arg=-lxml2); fi
$CXX -O3 -fPIC -std=c++17 ${XML_C_DEFS[@]+"${XML_C_DEFS[@]}"} -c src/parsers/benches/support/xml_c_shim.cpp -o "$SUP/xml_c_shim.o"
XML_CFG=()
for d in ${XML_C_DEFS[@]+"${XML_C_DEFS[@]}"}; do
  case "$d" in -DHAVE_*) XML_CFG+=("--cfg" "$(echo "${d#-DHAVE_}" | tr A-Z a-z)") ;; esac
done

rm -f "$SUP/libbun_bench_cdeps.a"
ar rcs "$SUP/libbun_bench_cdeps.a" "$SUP"/*.o
ranlib "$SUP/libbun_bench_cdeps.a"

export MIMALLOC_PURGE_DELAY=${MIMALLOC_PURGE_DELAY:-2000}
export BUN_JSON_BENCH_FIXTURES=${BUN_JSON_BENCH_FIXTURES:-$PWD/bench/json-corpus}
CXXLIB=stdc++
[ "$(uname -s)" = Darwin ] && CXXLIB=c++
export BUN_XML_BENCH_FIXTURES=${BUN_XML_BENCH_FIXTURES:-$PWD/bench/xml-corpus}
export RUSTFLAGS="${RUSTFLAGS:-} ${XML_CFG[*]-} -Clink-arg=$PWD/$SUP/libbun_bench_cdeps.a ${XML_C_LIBS[*]-} -Clink-arg=-l$CXXLIB -Clink-arg=-lm -Clink-arg=-ldl -Clink-arg=-lpthread -Clink-arg=-lc"

if [ "${1:-}" = "--test" ]; then
  shift
  exec cargo test -p bun_parsers --lib --release "$@"
fi
BENCH=json_parse
if [ "${1:-}" = "--xml" ]; then
  shift
  BENCH=xml_parse
fi
exec cargo bench -p bun_parsers --bench "$BENCH" "$@"
