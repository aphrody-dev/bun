// Records what simdutf 9.0.0 (the version that WebKit vendored, behind the C++ that src/simdutf_sys replaced)
// returns for a set of inputs, into src/simdutf_sys/tests/fixtures.txt, which the crate's tests compare with.
//
//   bun scripts/simdutf-fixtures.ts            needs clang++ and the network (simdutf's release)
//
// Each line: an operation, its input in hex, its argument, then what simdutf returned, with the FNV-1a of each output.
import { $ } from "bun";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

const version = "v9.0.0";
const root = join(import.meta.dir, "..");
const work = join(root, "build", "simdutf-fixtures");
const out = join(root, "src", "simdutf_sys", "tests", "fixtures.txt");
await mkdir(work, { recursive: true });

for (const name of ["simdutf.cpp", "simdutf.h"]) {
  const path = join(work, `${version}-${name}`);
  if (!(await Bun.file(path).exists())) {
    const response = await fetch(`https://github.com/simdutf/simdutf/releases/download/${version}/${name}`);
    if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
    await Bun.write(path, response);
  }
  await Bun.write(join(work, name), Bun.file(path));
}

// The wrappers of the deleted src/simdutf_sys/bun-simdutf.cpp, around a reader of this script's inputs.
const driver = String.raw`
#include "simdutf.h"
#include <cstdio>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>
using namespace std;

static vector<uint8_t> unhex(const string& s) {
  vector<uint8_t> v;
  for (size_t i = 0; i + 1 < s.size(); i += 2) v.push_back(stoi(s.substr(i, 2), nullptr, 16));
  return v;
}
// FNV-1a of the bytes written.
static string hex(const void* p, size_t n) {
  uint64_t h = 0xcbf29ce484222325ull;
  for (size_t i = 0; i < n; i++) h = (h ^ ((const uint8_t*)p)[i]) * 0x100000001b3ull;
  char s[17];
  snprintf(s, sizeof s, "%016llx", (unsigned long long)h);
  return s;
}

static simdutf::result decode(const char* in, size_t len, char* out, size_t& outlen, simdutf::base64_options o) {
  size_t n = outlen;
  auto r = simdutf::base64_to_binary_safe(in, len, out, n, o, simdutf::last_chunk_handling_options::loose);
  if (r.error == simdutf::SUCCESS) { outlen = n; return {simdutf::SUCCESS, n}; }
  outlen = n;
  return r;
}

static int pem_decode(uint8_t* out, size_t* out_len, size_t max_out, const uint8_t* in, size_t in_len) {
  size_t written = max_out;
  auto res = simdutf::base64_to_binary_safe(reinterpret_cast<const char*>(in), in_len, reinterpret_cast<char*>(out), written, simdutf::base64_default);
  if (res.error != simdutf::error_code::SUCCESS) return 0;
  size_t padding = 0;
  for (size_t i = in_len; i > 0; i--) {
    uint8_t c = in[i - 1];
    if (c == '=') padding++;
    else if (c != '\n' && c != '\r' && c != ' ' && c != '\t' && c != '\f' && c != '\v') break;
  }
  if (padding != (3 - written % 3) % 3) return 0;
  *out_len = written;
  return 1;
}

static size_t pem_encode(char* out, size_t max_out, const uint8_t* in, size_t in_len) {
  constexpr size_t lineLength = 64;
  size_t needed = simdutf::base64_length_from_binary_with_lines(in_len, simdutf::base64_default, lineLength) + 1;
  if (in_len == 0 || needed > max_out) return 0;
  size_t written = simdutf::binary_to_base64_with_lines(reinterpret_cast<const char*>(in), in_len, out, lineLength, simdutf::base64_default);
  out[written++] = '\n';
  return written;
}

int main() {
  string line;
  while (getline(cin, line)) {
    size_t t1 = line.find('\t'), t2 = line.find('\t', t1 + 1);
    string op = line.substr(0, t1), h = line.substr(t1 + 1, t2 - t1 - 1), arg = line.substr(t2 + 1);
    vector<uint8_t> in = h == "-" ? vector<uint8_t>() : unhex(h);
    const char* p = (const char*)in.data();
    size_t n = in.size();
    cout << line;
    if (op == "utf8") {
      auto r = simdutf::validate_utf8_with_errors(p, n);
      cout << '\t' << simdutf::validate_utf8(p, n) << '\t' << r.error << '\t' << r.count;
      vector<char16_t> o(simdutf::utf16_length_from_utf8(p, n) + 1);
      auto c = simdutf::convert_utf8_to_utf16le_with_errors(p, n, o.data());
      cout << '\t' << simdutf::utf16_length_from_utf8(p, n) << '\t' << c.error << '\t' << c.count << '\t'
           << (c.error ? "-" : hex(o.data(), c.count * 2));
      auto a = simdutf::validate_ascii_with_errors(p, n);
      cout << '\t' << simdutf::validate_ascii(p, n) << '\t' << a.error << '\t' << a.count;
      size_t len8 = simdutf::utf8_length_from_latin1(p, n);
      vector<char> l(len8 + 1);
      size_t w = simdutf::convert_latin1_to_utf8(p, n, l.data());
      cout << '\t' << len8 << '\t' << hex(l.data(), w);
    } else if (op == "utf16") {
      vector<char16_t> u(n / 2);
      memcpy(u.data(), p, u.size() * 2);
      size_t len8 = simdutf::utf8_length_from_utf16le(u.data(), u.size());
      size_t len8r = simdutf::utf8_length_from_utf16le_with_replacement(u.data(), u.size()).count;
      vector<char> o(u.size() * 3 + 1);
      auto c = simdutf::convert_utf16le_to_utf8_with_errors(u.data(), u.size(), o.data());
      cout << '\t' << simdutf::validate_utf16le(u.data(), u.size()) << '\t' << len8 << '\t' << len8r << '\t'
           << c.error << '\t' << c.count << '\t' << (c.error ? "-" : hex(o.data(), c.count));
    } else if (op == "b64enc") {
      auto o = arg == "1" ? simdutf::base64_url : simdutf::base64_default;
      size_t len = simdutf::base64_length_from_binary(n, o);
      vector<char> buf(len + 1);
      size_t w = simdutf::binary_to_base64(p, n, buf.data(), o);
      cout << '\t' << len << '\t' << hex(buf.data(), w);
    } else if (op == "b64dec") {
      // For each room in the argument: standard, URL-safe, lenient.
      for (size_t at = 0; at < arg.size();) {
        size_t comma = arg.find(',', at);
        if (comma == string::npos) comma = arg.size();
        size_t room = stoul(arg.substr(at, comma - at));
        at = comma + 1;
        for (auto o : {simdutf::base64_default, simdutf::base64_url, simdutf::base64_default_or_url_accept_garbage}) {
          size_t outlen = room;
          vector<char> buf(outlen + 1);
          auto r = decode(p, n, buf.data(), outlen, o);
          cout << '\t' << r.error << '\t' << r.count << '\t' << (r.error ? "-" : hex(buf.data(), r.count));
        }
      }
    } else if (op == "pemdec") {
      size_t max_out = stoul(arg), out_len = 0;
      vector<uint8_t> buf(max_out + 1);
      int ok = pem_decode(buf.data(), &out_len, max_out, in.data(), n);
      cout << '\t' << ok << '\t' << (ok ? hex(buf.data(), out_len) : "-");
    } else if (op == "pemenc") {
      size_t max_out = stoul(arg);
      vector<char> buf(max_out + 1);
      size_t w = pem_encode(buf.data(), max_out, in.data(), n);
      cout << '\t' << hex(buf.data(), w);
    }
    cout << '\n';
  }
}
`;
await Bun.write(join(work, "driver.cpp"), driver);
const exe = join(work, "driver");
await $`clang++ -O2 -std=c++17 -I${work} ${join(work, "driver.cpp")} ${join(work, "simdutf.cpp")} -o ${exe}`;

// xorshift64*, so that a run writes the same file.
let state = 0x9e3779b97f4a7c15n;
const random = (n: number) => {
  state ^= state >> 12n;
  state ^= (state << 25n) & 0xffffffffffffffffn;
  state ^= state >> 27n;
  return Number(((state * 0x2545f4914f6cdd1dn) & 0xffffffffffffffffn) >> 11n) % n;
};
const pick = <T>(items: T[]): T => items[random(items.length)];
const hex = (bytes: Iterable<number>) => Buffer.from([...bytes]).toString("hex") || "-";
const ascii = (n: number) => Array.from({ length: n }, () => 0x20 + random(0x5f));
const encodeUtf8 = (s: string) => [...Buffer.from(s, "utf8")];

const utf8Pieces: (() => number[])[] = [
  () => ascii(1 + random(3)),
  () => ascii(16 + random(50)),
  () => encodeUtf8(pick(["é", "ß", "ÿ", "߿", "ࠀ", "€", "日本", "￿", "퟿", "", "😀", "𐀀", "\u{10ffff}"])),
  () => [pick([0x80, 0xbf, 0xc0, 0xc1, 0xf5, 0xf8, 0xfc, 0xff, 0xfe])],
  () => pick([[0xc2], [0xe2, 0x82], [0xf0, 0x9f], [0xf0, 0x9f, 0x98], [0xe0]]),
  () => pick([[0xc0, 0x80], [0xe0, 0x80, 0x80], [0xe0, 0x9f, 0xbf], [0xf0, 0x80, 0x80, 0x80], [0xf0, 0x8f, 0xbf, 0xbf]]),
  () => pick([[0xed, 0xa0, 0x80], [0xed, 0xbf, 0xbf], [0xf4, 0x90, 0x80, 0x80], [0xf7, 0xbf, 0xbf, 0xbf]]),
  () => pick([[0xc2, 0x41], [0xe2, 0x41, 0x80], [0xe2, 0x82, 0x41], [0xf0, 0x9f, 0x41, 0x80], [0xf0, 0x9f, 0x98, 0x41]]),
  () => pick([[0x80, 0x80], [0xc2, 0x80, 0x80], [0xe2, 0x82, 0xac, 0xac]]),
];
const lines: string[] = [];
const utf8Inputs: number[][] = [[], ascii(64), ascii(15), encodeUtf8("héllo wörld ".repeat(20))];
for (let i = 0; i < 1200; i++) {
  const bytes: number[] = [];
  const count = 1 + random(i < 300 ? 4 : 16);
  const clean = random(3) === 0;
  for (let j = 0; j < count; j++) bytes.push(...pick(clean ? utf8Pieces.slice(0, 3) : utf8Pieces)());
  utf8Inputs.push(bytes);
}
// An error just before and just after each place where a vector of 16, 32 or 64 bytes ends.
for (const at of [15, 16, 31, 32, 33, 63, 64, 65, 127, 128]) {
  for (const bad of [[0x80], [0xc2], [0xe2, 0x82], [0xed, 0xa0, 0x80], [0xf0, 0x9f, 0x98]]) {
    utf8Inputs.push([...ascii(at), ...bad, ...ascii(20)]);
    utf8Inputs.push([...ascii(at - 1), 0xf0, 0x9f, 0x98, 0x80, ...bad]);
  }
}
for (const bytes of utf8Inputs) {
  lines.push(`utf8\t${hex(bytes)}\t-`);
}

const units = (...u: number[]) => u;
const utf16Pieces: (() => number[])[] = [
  () => ascii(1 + random(3)),
  () => ascii(4 + random(40)),
  () => units(pick([0xe9, 0xff, 0x100, 0x7ff, 0x800, 0x20ac, 0xd7ff, 0xe000, 0xffff])),
  () => units(0xd83d, 0xde00),
  () => units(pick([0xd800, 0xdbff, 0xdc00, 0xdfff])),
  () => units(0xde00, 0xd83d),
];
const utf16Inputs: number[][] = [[], ascii(4), ascii(64)];
for (let i = 0; i < 800; i++) {
  const u: number[] = [];
  const count = 1 + random(16);
  for (let j = 0; j < count; j++) u.push(...pick(utf16Pieces)());
  utf16Inputs.push(u);
}
for (const u of utf16Inputs) lines.push(`utf16\t${hex(new Uint8Array(new Uint16Array(u).buffer))}\t-`);

const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const b64Pieces: (() => string)[] = [
  () => Array.from({ length: 1 + random(12) }, () => alphabet[random(64)]).join(""),
  () => pick([" ", "\n", "\r\n", "\t", "\f", "\v"]),
  () => pick(["=", "==", "==="]),
  () => pick(["-", "_", "*", "!", "\x00", "\x80", "é"]),
];
const b64Inputs: string[] = ["", "=", "==", "QQ", "QQ=", "QQ==", "QUI", "QUI=", "QUJD", "Q", "QQ==QQ==", " QQ== ", "QR==", "QUJ="];
for (let i = 0; i < 400; i++) {
  const bytes = Array.from({ length: random(60) }, () => random(256));
  let text = Buffer.from(bytes).toString(random(2) ? "base64" : "base64url");
  if (random(2)) text = text.replace(/=+$/, "");
  const edits = random(4);
  for (let j = 0; j < edits; j++) {
    const at = random(text.length + 1);
    text = text.slice(0, at) + pick(b64Pieces)() + text.slice(at);
  }
  b64Inputs.push(text);
}
for (const text of b64Inputs) {
  const bytes = [...Buffer.from(text, "latin1")];
  const maximal = Math.ceil(bytes.length / 4) * 3;
  const sizes = [...new Set([maximal, Math.max(0, maximal - 1), Math.max(0, maximal - 3), bytes.length >> 1])];
  lines.push(`b64dec\t${hex(bytes)}\t${sizes.join(",")}`);
}
for (let i = 0; i < 150; i++) {
  const bytes = Array.from({ length: i < 10 ? i : random(120) }, () => random(256));
  lines.push(`b64enc\t${hex(bytes)}\t0`, `b64enc\t${hex(bytes)}\t1`);
}
for (let i = 0; i < 60; i++) {
  const bytes = Array.from({ length: i < 5 ? i : random(300) }, () => random(256));
  const needed = bytes.length ? Math.ceil(bytes.length / 3) * 4 + Math.ceil((Math.ceil(bytes.length / 3) * 4) / 64) : 0;
  for (const size of [needed, needed - 1, needed + 5]) lines.push(`pemenc\t${hex(bytes)}\t${Math.max(0, size)}`);
  let text = Buffer.from(bytes).toString("base64").replace(/.{64}/g, "$&\n") + "\n";
  if (random(4) === 0) text = text.replace(/=/g, "");
  if (random(6) === 0) text = text.replace(/\n/g, "\r\n");
  if (random(8) === 0) text += pick(["=", " ", "\v", "*"]);
  const pem = [...Buffer.from(text, "latin1")];
  for (const size of [bytes.length, bytes.length + 2, Math.max(0, bytes.length - 1)]) lines.push(`pemdec\t${hex(pem)}\t${size}`);
}

const proc = Bun.spawn([exe], { stdin: new Blob([lines.join("\n") + "\n"]), stdout: "pipe", stderr: "inherit" });
const text = await new Response(proc.stdout).text();
if ((await proc.exited) !== 0) throw new Error("the driver failed");
const header = `# simdutf ${version}, by scripts/simdutf-fixtures.ts. op, input, argument, results (FNV-1a of outputs).\n`;
await Bun.write(out, header + text);
console.log(`${out}: ${text.split("\n").length - 1} cases, ${(text.length / 1024) | 0} KiB`);
