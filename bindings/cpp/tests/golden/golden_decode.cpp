/*
 * bindings/cpp/tests/golden/golden_decode.cpp — decode-only C++20 reader for the
 * frozen canon-codec golden fixtures; the fourth independent decoder after the
 * Rust reference, the C reader (bindings/c/tests/golden/), and the Python reader
 * (bindings/python/tests/test_golden_bytes.py). Closes the Phase 4 C++ deferral.
 *
 * Clean-room and dependency-free: a tiny recursive-descent JSON parser, a strict
 * lowercase-hex reader, and a schema-driven two-region decoder derived only from
 * docs/algorithms/canon-codec.md (fixed header in declaration order + natural
 * alignment; u32 count descriptor [+ u32 offset when > 1 variable member];
 * element-aligned contiguous bodies in declaration order; derived sample_len;
 * zeroed padding/gap bytes; strict UTF-8). No shared code with the codec under
 * test, no third-party dependency.
 *
 * It reads the committed fixtures directly:
 *   primitives/index.json  (+ <stem>.hex)          — 24 vectors
 *   composites/<stem>.json (+ <stem>.hex)          — 10 vectors
 *   descriptor/descriptor_golden.json              —  1 vector
 * and asserts every decoded value matches the fixture metadata byte-for-byte.
 *
 * REQ: REQ-BIND-013, REQ-TOOL-040, REQ-TOOL-042, REQ-TOOL-043, REQ-TOOL-044,
 *      REQ-CORE-083. Invariants: canon-codec.md I1 (GoldenIdentity), I9
 *      (IndependentVectors), I10 (Utf8Strict).
 *
 * Usage: golden_decode [fixtures-dir]   (default: tests/fixtures/golden)
 */
#include <algorithm>
#include <cctype>
#include <charconv>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <memory>
#include <string>
#include <string_view>
#include <system_error>
#include <utility>
#include <vector>

namespace {

/* ------------------------------------------------------------------ file IO */

bool read_text(const std::string &path, std::string &out) {
    std::ifstream f(path, std::ios::binary);
    if (!f) {
        return false;
    }
    out.assign(std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>());
    return true;
}

int hex_nibble(char c) {
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    if (c >= 'a' && c <= 'f') {
        return c - 'a' + 10;
    }
    return -1; /* strict lowercase hex only (matches the frozen fixtures) */
}

bool hex_decode(std::string_view hex, std::vector<std::uint8_t> &out) {
    if (hex.size() % 2 != 0) {
        return false;
    }
    out.clear();
    out.reserve(hex.size() / 2);
    for (std::size_t i = 0; i < hex.size(); i += 2) {
        const int hi = hex_nibble(hex[i]);
        const int lo = hex_nibble(hex[i + 1]);
        if (hi < 0 || lo < 0) {
            return false;
        }
        out.push_back(static_cast<std::uint8_t>((hi << 4) | lo));
    }
    return true;
}

std::string_view trim(std::string_view s) {
    auto ws = [](char c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n'; };
    while (!s.empty() && ws(s.front())) {
        s.remove_prefix(1);
    }
    while (!s.empty() && ws(s.back())) {
        s.remove_suffix(1);
    }
    return s;
}

/* --------------------------------------------------------------- JSON (tiny) */

struct Json {
    enum class K { Null, Bool, Num, Str, Arr, Obj } k = K::Null;
    bool b = false;
    std::string str;               /* Str: decoded bytes; Num: raw token */
    std::vector<Json> arr;         /* array elements, or object values */
    std::vector<std::string> keys; /* object keys, parallel to arr; empty for arrays */

    const Json *find(std::string_view key) const {
        for (std::size_t i = 0; i < keys.size(); ++i) {
            if (keys[i] == key) {
                return &arr[i];
            }
        }
        return nullptr;
    }
};

class JsonParser {
  public:
    explicit JsonParser(std::string_view s) : s_(s) {}

    bool parse(Json &out) {
        skip_ws();
        if (!value(out)) {
            return false;
        }
        skip_ws();
        return pos_ == s_.size();
    }

  private:
    std::string_view s_;
    std::size_t pos_ = 0;

    void skip_ws() {
        while (pos_ < s_.size()) {
            const char c = s_[pos_];
            if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
                ++pos_;
            } else {
                break;
            }
        }
    }

    bool value(Json &out) {
        skip_ws();
        if (pos_ >= s_.size()) {
            return false;
        }
        const char c = s_[pos_];
        if (c == '{') {
            return object(out);
        }
        if (c == '[') {
            return array(out);
        }
        if (c == '"') {
            out.k = Json::K::Str;
            return string(out.str);
        }
        if (c == 't') {
            if (lit("true")) {
                out.k = Json::K::Bool;
                out.b = true;
                return true;
            }
            return false;
        }
        if (c == 'f') {
            if (lit("false")) {
                out.k = Json::K::Bool;
                out.b = false;
                return true;
            }
            return false;
        }
        if (c == 'n') {
            if (lit("null")) {
                out.k = Json::K::Null;
                return true;
            }
            return false;
        }
        return number(out);
    }

    bool lit(std::string_view w) {
        if (s_.substr(pos_, w.size()) == w) {
            pos_ += w.size();
            return true;
        }
        return false;
    }

    bool object(Json &out) {
        out.k = Json::K::Obj;
        ++pos_; /* { */
        skip_ws();
        if (pos_ < s_.size() && s_[pos_] == '}') {
            ++pos_;
            return true;
        }
        while (true) {
            skip_ws();
            std::string key;
            if (!string(key)) {
                return false;
            }
            skip_ws();
            if (pos_ >= s_.size() || s_[pos_] != ':') {
                return false;
            }
            ++pos_;
            Json v;
            if (!value(v)) {
                return false;
            }
            out.keys.push_back(std::move(key));
            out.arr.push_back(std::move(v));
            skip_ws();
            if (pos_ >= s_.size()) {
                return false;
            }
            if (s_[pos_] == ',') {
                ++pos_;
                continue;
            }
            if (s_[pos_] == '}') {
                ++pos_;
                return true;
            }
            return false;
        }
    }

    bool array(Json &out) {
        out.k = Json::K::Arr;
        ++pos_; /* [ */
        skip_ws();
        if (pos_ < s_.size() && s_[pos_] == ']') {
            ++pos_;
            return true;
        }
        while (true) {
            Json v;
            if (!value(v)) {
                return false;
            }
            out.arr.push_back(std::move(v));
            skip_ws();
            if (pos_ >= s_.size()) {
                return false;
            }
            if (s_[pos_] == ',') {
                ++pos_;
                continue;
            }
            if (s_[pos_] == ']') {
                ++pos_;
                return true;
            }
            return false;
        }
    }

    bool string(std::string &out) {
        if (pos_ >= s_.size() || s_[pos_] != '"') {
            return false;
        }
        ++pos_;
        out.clear();
        while (pos_ < s_.size()) {
            const char c = s_[pos_++];
            if (c == '"') {
                return true;
            }
            if (c != '\\') {
                out.push_back(c);
                continue;
            }
            if (pos_ >= s_.size()) {
                return false;
            }
            const char e = s_[pos_++];
            switch (e) {
            case '"':
                out.push_back('"');
                break;
            case '\\':
                out.push_back('\\');
                break;
            case '/':
                out.push_back('/');
                break;
            case 'b':
                out.push_back('\b');
                break;
            case 'f':
                out.push_back('\f');
                break;
            case 'n':
                out.push_back('\n');
                break;
            case 'r':
                out.push_back('\r');
                break;
            case 't':
                out.push_back('\t');
                break;
            case 'u':
                if (!utf8_escape(out)) {
                    return false;
                }
                break;
            default:
                return false;
            }
        }
        return false;
    }

    bool utf8_escape(std::string &out) {
        std::uint32_t cp = 0;
        if (!hex4(cp)) {
            return false;
        }
        if (cp >= 0xD800 && cp <= 0xDBFF) { /* high surrogate: need a low one */
            if (pos_ + 1 >= s_.size() || s_[pos_] != '\\' || s_[pos_ + 1] != 'u') {
                return false;
            }
            pos_ += 2;
            std::uint32_t lo = 0;
            if (!hex4(lo) || lo < 0xDC00 || lo > 0xDFFF) {
                return false;
            }
            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
        } else if (cp >= 0xDC00 && cp <= 0xDFFF) {
            return false; /* lone low surrogate */
        }
        append_utf8(out, cp);
        return true;
    }

    bool hex4(std::uint32_t &v) {
        if (pos_ + 4 > s_.size()) {
            return false;
        }
        v = 0;
        for (int i = 0; i < 4; ++i) {
            const int n = hex_nibble(s_[pos_ + static_cast<std::size_t>(i)]);
            if (n < 0) {
                return false;
            }
            v = (v << 4) | static_cast<std::uint32_t>(n);
        }
        pos_ += 4;
        return true;
    }

    static void append_utf8(std::string &out, std::uint32_t cp) {
        if (cp < 0x80) {
            out.push_back(static_cast<char>(cp));
        } else if (cp < 0x800) {
            out.push_back(static_cast<char>(0xC0 | (cp >> 6)));
            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
        } else if (cp < 0x10000) {
            out.push_back(static_cast<char>(0xE0 | (cp >> 12)));
            out.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
        } else {
            out.push_back(static_cast<char>(0xF0 | (cp >> 18)));
            out.push_back(static_cast<char>(0x80 | ((cp >> 12) & 0x3F)));
            out.push_back(static_cast<char>(0x80 | ((cp >> 6) & 0x3F)));
            out.push_back(static_cast<char>(0x80 | (cp & 0x3F)));
        }
    }

    bool number(Json &out) {
        const std::size_t start = pos_;
        if (pos_ < s_.size() && s_[pos_] == '-') {
            ++pos_;
        }
        bool any = false;
        while (pos_ < s_.size() && std::isdigit(static_cast<unsigned char>(s_[pos_]))) {
            ++pos_;
            any = true;
        }
        if (pos_ < s_.size() && s_[pos_] == '.') {
            ++pos_;
            while (pos_ < s_.size() && std::isdigit(static_cast<unsigned char>(s_[pos_]))) {
                ++pos_;
                any = true;
            }
        }
        if (any && pos_ < s_.size() && (s_[pos_] == 'e' || s_[pos_] == 'E')) {
            ++pos_;
            if (pos_ < s_.size() && (s_[pos_] == '-' || s_[pos_] == '+')) {
                ++pos_;
            }
            while (pos_ < s_.size() && std::isdigit(static_cast<unsigned char>(s_[pos_]))) {
                ++pos_;
            }
        }
        if (!any) {
            return false;
        }
        out.k = Json::K::Num;
        out.str.assign(s_.substr(start, pos_ - start));
        return true;
    }
};

bool num_u64(const Json &j, std::uint64_t &out) {
    if (j.k != Json::K::Num) {
        return false;
    }
    const char *b = j.str.data();
    const char *e = b + j.str.size();
    const auto r = std::from_chars(b, e, out);
    return r.ec == std::errc() && r.ptr == e;
}

bool num_i64(const Json &j, std::int64_t &out) {
    if (j.k != Json::K::Num) {
        return false;
    }
    const char *b = j.str.data();
    const char *e = b + j.str.size();
    const auto r = std::from_chars(b, e, out);
    return r.ec == std::errc() && r.ptr == e;
}

double num_double(const Json &j) { return std::strtod(j.str.c_str(), nullptr); }

/* ----------------------------------------------------------- type + schema */

enum class Prim { U8, I8, U16, I16, U32, I32, U64, I64, F32, F64, Bool };
enum class TK { Prim, FixedArray, Enum, Str, Bytes, Bounded };

struct Type {
    TK kind = TK::Prim;
    Prim prim = Prim::U8;
    std::shared_ptr<Type> elem; /* FixedArray / Bounded */
    std::uint64_t n = 0;        /* FixedArray count, Str/Bytes max, Bounded max */
};

std::uint64_t prim_size(Prim p) {
    switch (p) {
    case Prim::U8:
    case Prim::I8:
    case Prim::Bool:
        return 1;
    case Prim::U16:
    case Prim::I16:
        return 2;
    case Prim::U32:
    case Prim::I32:
    case Prim::F32:
        return 4;
    case Prim::U64:
    case Prim::I64:
    case Prim::F64:
        return 8;
    }
    return 0;
}

std::uint64_t type_size(const Type &t) {
    switch (t.kind) {
    case TK::Prim:
        return prim_size(t.prim);
    case TK::Enum:
        return 1;
    case TK::FixedArray:
        return type_size(*t.elem) * t.n;
    case TK::Str:
    case TK::Bytes:
    case TK::Bounded:
        return 4; /* descriptor; the body lives in the tail */
    }
    return 0;
}

std::uint64_t type_align(const Type &t) {
    switch (t.kind) {
    case TK::Prim:
        return prim_size(t.prim);
    case TK::Enum:
        return 1;
    case TK::FixedArray:
        return type_align(*t.elem);
    case TK::Str:
    case TK::Bytes:
    case TK::Bounded:
        return 4;
    }
    return 1;
}

bool is_var(const Type &t) {
    return t.kind == TK::Str || t.kind == TK::Bytes || t.kind == TK::Bounded;
}

std::uint64_t body_align(const Type &t) { return t.kind == TK::Bounded ? type_align(*t.elem) : 1; }
std::uint64_t body_elem_size(const Type &t) {
    return t.kind == TK::Bounded ? type_size(*t.elem) : 1;
}

std::uint64_t align_up(std::uint64_t x, std::uint64_t a) { return (x + (a - 1)) & ~(a - 1); }

bool parse_prim(std::string_view s, Prim &p) {
    if (s == "u8") {
        p = Prim::U8;
    } else if (s == "i8") {
        p = Prim::I8;
    } else if (s == "u16") {
        p = Prim::U16;
    } else if (s == "i16") {
        p = Prim::I16;
    } else if (s == "u32") {
        p = Prim::U32;
    } else if (s == "i32") {
        p = Prim::I32;
    } else if (s == "u64") {
        p = Prim::U64;
    } else if (s == "i64") {
        p = Prim::I64;
    } else if (s == "f32") {
        p = Prim::F32;
    } else if (s == "f64") {
        p = Prim::F64;
    } else if (s == "bool") {
        p = Prim::Bool;
    } else {
        return false;
    }
    return true;
}

bool parse_u64(std::string_view s, std::uint64_t &out) {
    if (s.empty()) {
        return false;
    }
    const auto r = std::from_chars(s.data(), s.data() + s.size(), out);
    return r.ec == std::errc() && r.ptr == s.data() + s.size();
}

bool parse_type(std::string_view s, Type &out) {
    s = trim(s);
    if (s.rfind("enum{", 0) == 0) {
        out.kind = TK::Enum;
        return true;
    }
    const auto lb = s.find('[');
    if (lb != std::string_view::npos) {
        if (s.back() != ']') {
            return false;
        }
        const std::string_view inner = trim(s.substr(lb + 1, s.size() - lb - 2));
        const std::string_view base = trim(s.substr(0, lb));
        const bool bounded = inner.rfind("<=", 0) == 0;
        const std::string_view num = trim(bounded ? inner.substr(2) : inner);
        std::uint64_t n = 0;
        if (!parse_u64(num, n)) {
            return false;
        }
        if (base == "string") {
            out.kind = TK::Str;
            out.n = n;
            return true;
        }
        if (base == "bytes") {
            out.kind = TK::Bytes;
            out.n = n;
            return true;
        }
        auto e = std::make_shared<Type>();
        if (!parse_type(base, *e)) {
            return false;
        }
        out.kind = bounded ? TK::Bounded : TK::FixedArray;
        out.elem = e;
        out.n = n;
        return true;
    }
    Prim p = Prim::U8;
    if (!parse_prim(s, p)) {
        return false;
    }
    out.kind = TK::Prim;
    out.prim = p;
    return true;
}

struct Field {
    std::string name;
    Type type;
};

bool parse_schema(std::string_view schema, std::vector<Field> &out) {
    std::size_t start = 0;
    while (start <= schema.size()) {
        const auto nl = schema.find('\n', start);
        const std::string_view line =
            trim(schema.substr(start, nl == std::string_view::npos ? nl : nl - start));
        start = (nl == std::string_view::npos) ? schema.size() + 1 : nl + 1;
        if (line.empty()) {
            continue;
        }
        const auto colon = line.find(':');
        if (colon == std::string_view::npos) {
            return false;
        }
        Field f;
        f.name.assign(trim(line.substr(0, colon)));
        if (!parse_type(trim(line.substr(colon + 1)), f.type)) {
            return false;
        }
        out.push_back(std::move(f));
    }
    return !out.empty();
}

/* ------------------------------------------------------------ decode + match */

std::uint64_t read_le(const std::uint8_t *p, std::size_t n) {
    std::uint64_t v = 0;
    for (std::size_t i = 0; i < n; ++i) {
        v |= static_cast<std::uint64_t>(p[i]) << (8 * i);
    }
    return v;
}

/* Strict UTF-8 (I10): rejects C0/C1 leads, overlong forms, surrogates,
 * > U+10FFFF, and truncated sequences. */
bool utf8_valid(const std::uint8_t *s, std::size_t n) {
    std::size_t i = 0;
    while (i < n) {
        const std::uint8_t c = s[i];
        if (c < 0x80u) {
            ++i;
        } else if (c >= 0xC2u && c <= 0xDFu) {
            if (i + 1 >= n || (s[i + 1] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 2;
        } else if (c == 0xE0u) {
            if (i + 2 >= n || (s[i + 1] & 0xE0u) != 0xA0u || (s[i + 2] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 3;
        } else if ((c >= 0xE1u && c <= 0xECu) || (c >= 0xEEu && c <= 0xEFu)) {
            if (i + 2 >= n || (s[i + 1] & 0xC0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 3;
        } else if (c == 0xEDu) {
            if (i + 2 >= n || (s[i + 1] & 0xE0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 3;
        } else if (c == 0xF0u) {
            if (i + 3 >= n || (s[i + 1] & 0xF0u) != 0x90u || (s[i + 2] & 0xC0u) != 0x80u ||
                (s[i + 3] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 4;
        } else if (c >= 0xF1u && c <= 0xF4u) {
            if (i + 3 >= n || (s[i + 1] & 0xC0u) != 0x80u || (s[i + 2] & 0xC0u) != 0x80u ||
                (s[i + 3] & 0xC0u) != 0x80u) {
                return false;
            }
            i += 4;
        } else {
            return false;
        }
    }
    return true;
}

bool match_prim(const Json &exp, const std::uint8_t *p, Prim prim) {
    switch (prim) {
    case Prim::U8: {
        std::uint64_t v = 0;
        return num_u64(exp, v) && v == p[0];
    }
    case Prim::I8: {
        std::int64_t v = 0;
        return num_i64(exp, v) && v == static_cast<std::int8_t>(p[0]);
    }
    case Prim::U16: {
        std::uint64_t v = 0;
        return num_u64(exp, v) && v == read_le(p, 2);
    }
    case Prim::I16: {
        std::int64_t v = 0;
        return num_i64(exp, v) && v == static_cast<std::int16_t>(read_le(p, 2));
    }
    case Prim::U32: {
        std::uint64_t v = 0;
        return num_u64(exp, v) && v == read_le(p, 4);
    }
    case Prim::I32: {
        std::int64_t v = 0;
        return num_i64(exp, v) && v == static_cast<std::int32_t>(read_le(p, 4));
    }
    case Prim::U64: {
        std::uint64_t v = 0;
        return num_u64(exp, v) && v == read_le(p, 8);
    }
    case Prim::I64: {
        std::int64_t v = 0;
        return num_i64(exp, v) && v == static_cast<std::int64_t>(read_le(p, 8));
    }
    case Prim::F32: {
        if (exp.k != Json::K::Num) {
            return false;
        }
        const std::uint32_t bits = static_cast<std::uint32_t>(read_le(p, 4));
        float f = 0.0f;
        std::memcpy(&f, &bits, sizeof f);
        return static_cast<double>(f) == static_cast<double>(static_cast<float>(num_double(exp)));
    }
    case Prim::F64: {
        if (exp.k != Json::K::Num) {
            return false;
        }
        const std::uint64_t bits = read_le(p, 8);
        double d = 0.0;
        std::memcpy(&d, &bits, sizeof d);
        return d == num_double(exp);
    }
    case Prim::Bool: {
        if (exp.k != Json::K::Bool || p[0] > 1) {
            return false;
        }
        return p[0] == (exp.b ? 1u : 0u);
    }
    }
    return false;
}

bool match_value(const Json &exp, const std::uint8_t *p, const Type &t);

bool match_body(const Json &exp, const std::uint8_t *p, const Type &t, std::uint64_t count) {
    switch (t.kind) {
    case TK::Str: {
        if (exp.k != Json::K::Str || exp.str.size() != count) {
            return false;
        }
        return utf8_valid(p, static_cast<std::size_t>(count)) &&
               std::memcmp(p, exp.str.data(), static_cast<std::size_t>(count)) == 0;
    }
    case TK::Bytes: {
        if (exp.k != Json::K::Arr || exp.arr.size() != count) {
            return false;
        }
        for (std::uint64_t i = 0; i < count; ++i) {
            std::uint64_t v = 0;
            if (!num_u64(exp.arr[static_cast<std::size_t>(i)], v) || v != p[i]) {
                return false;
            }
        }
        return true;
    }
    case TK::Bounded: {
        if (exp.k != Json::K::Arr || exp.arr.size() != count) {
            return false;
        }
        const std::uint64_t es = type_size(*t.elem);
        for (std::uint64_t i = 0; i < count; ++i) {
            if (!match_value(exp.arr[static_cast<std::size_t>(i)], p + i * es, *t.elem)) {
                return false;
            }
        }
        return true;
    }
    default:
        return false;
    }
}

bool match_value(const Json &exp, const std::uint8_t *p, const Type &t) {
    switch (t.kind) {
    case TK::Prim:
        return match_prim(exp, p, t.prim);
    case TK::Enum: {
        std::uint64_t v = 0;
        return num_u64(exp, v) && v <= 255 && p[0] == static_cast<std::uint8_t>(v);
    }
    case TK::FixedArray: {
        if (exp.k != Json::K::Arr || exp.arr.size() != t.n) {
            return false;
        }
        const std::uint64_t es = type_size(*t.elem);
        for (std::uint64_t i = 0; i < t.n; ++i) {
            if (!match_value(exp.arr[static_cast<std::size_t>(i)], p + i * es, *t.elem)) {
                return false;
            }
        }
        return true;
    }
    default:
        return false;
    }
}

/* Decode one composite: fixed header + descriptor block + element-aligned tail.
 * Validates the canonical packing exactly (offsets a pure function of counts,
 * derived sample_len == buffer length, zeroed gaps) then matches every value. */
bool decode_struct(const std::vector<std::uint8_t> &buf, const std::vector<Field> &fields,
                   const Json &values, std::string &err) {
    const std::uint64_t n = buf.size();
    std::size_t nvar = 0;
    for (const auto &f : fields) {
        if (is_var(f.type)) {
            ++nvar;
        }
    }

    std::vector<bool> covered(n, false);
    std::vector<std::uint64_t> foff(fields.size(), 0);

    std::uint64_t cursor = 0;
    for (std::size_t i = 0; i < fields.size(); ++i) {
        if (is_var(fields[i].type)) {
            continue;
        }
        cursor = align_up(cursor, type_align(fields[i].type));
        foff[i] = cursor;
        const std::uint64_t sz = type_size(fields[i].type);
        for (std::uint64_t b = cursor; b < cursor + sz; ++b) {
            if (b >= n) {
                err = "fixed field exceeds sample";
                return false;
            }
            covered[static_cast<std::size_t>(b)] = true;
        }
        cursor += sz;
    }

    const std::uint64_t desc_start = align_up(cursor, 4);
    std::uint64_t dc = desc_start;
    struct VarDesc {
        std::size_t fi;
        std::uint64_t count_off;
        std::uint64_t off_off;
        bool has_off;
        std::uint64_t off;
        std::uint64_t count;
    };
    std::vector<VarDesc> vars;
    for (std::size_t i = 0; i < fields.size(); ++i) {
        if (!is_var(fields[i].type)) {
            continue;
        }
        VarDesc v{};
        v.fi = i;
        v.count_off = dc;
        dc += 4;
        v.has_off = nvar > 1;
        v.off_off = v.has_off ? dc : 0;
        if (v.has_off) {
            dc += 4;
        }
        vars.push_back(v);
    }
    const std::uint64_t header_end = dc;
    if (header_end > n) {
        err = "header exceeds sample";
        return false;
    }
    for (std::uint64_t b = desc_start; b < header_end; ++b) {
        covered[static_cast<std::size_t>(b)] = true;
    }

    std::uint64_t tc = header_end;
    for (auto &v : vars) {
        const Type &t = fields[v.fi].type;
        const std::uint64_t cnt = read_le(buf.data() + v.count_off, 4);
        if (cnt > t.n) {
            err = "count exceeds declared max";
            return false;
        }
        v.count = cnt;
        const std::uint64_t off = align_up(tc, body_align(t));
        if (v.has_off) {
            const std::uint64_t stored = read_le(buf.data() + v.off_off, 4);
            if (stored != off) {
                err = "descriptor offset != canonical packing";
                return false;
            }
        }
        v.off = off;
        const std::uint64_t end = off + cnt * body_elem_size(t);
        if (end > n) {
            err = "body end exceeds sample";
            return false;
        }
        for (std::uint64_t b = off; b < end; ++b) {
            covered[static_cast<std::size_t>(b)] = true;
        }
        tc = end;
    }
    if (tc != n) {
        err = "derived sample_len != buffer length";
        return false;
    }
    for (std::uint64_t b = 0; b < n; ++b) {
        if (!covered[static_cast<std::size_t>(b)] && buf[static_cast<std::size_t>(b)] != 0) {
            err = "nonzero padding/gap byte";
            return false;
        }
    }

    for (std::size_t i = 0; i < fields.size(); ++i) {
        if (is_var(fields[i].type)) {
            continue;
        }
        const Json *exp = values.find(fields[i].name);
        if (exp == nullptr) {
            err = "missing value: " + fields[i].name;
            return false;
        }
        if (!match_value(*exp, buf.data() + foff[i], fields[i].type)) {
            err = "value mismatch: " + fields[i].name;
            return false;
        }
    }
    for (const auto &v : vars) {
        const Json *exp = values.find(fields[v.fi].name);
        if (exp == nullptr) {
            err = "missing value: " + fields[v.fi].name;
            return false;
        }
        if (!match_body(*exp, buf.data() + v.off, fields[v.fi].type, v.count)) {
            err = "value mismatch: " + fields[v.fi].name;
            return false;
        }
    }
    return true;
}

/* ------------------------------------------------------------------ runners */

struct Counts {
    std::size_t prim_pass = 0;
    std::size_t prim_total = 0;
    std::size_t comp_pass = 0;
    std::size_t comp_total = 0;
    std::size_t desc_pass = 0;
    std::size_t desc_total = 0;
};

void fail(const std::string &msg) {
    std::fprintf(stderr, "golden_decode: FAIL: %s\n", msg.c_str());
}

bool run_primitives(const std::string &dir, Counts &c) {
    std::string text;
    if (!read_text(dir + "/index.json", text)) {
        fail("cannot read " + dir + "/index.json");
        return false;
    }
    Json root;
    if (!JsonParser(text).parse(root)) {
        fail("primitives/index.json: parse error");
        return false;
    }
    const Json *vectors = root.find("vectors");
    if (vectors == nullptr || vectors->k != Json::K::Arr) {
        fail("primitives/index.json: no vectors[]");
        return false;
    }
    for (const auto &v : vectors->arr) {
        c.prim_total++;
        const Json *file = v.find("file");
        const Json *type = v.find("type");
        const Json *value = v.find("value");
        const Json *len = v.find("len");
        if (file == nullptr || type == nullptr || value == nullptr || len == nullptr) {
            fail("primitive entry missing metadata");
            return false;
        }
        std::uint64_t want_len = 0;
        Prim prim = Prim::U8;
        if (!num_u64(*len, want_len) || !parse_prim(type->str, prim)) {
            fail("primitive " + file->str + ": bad type/len");
            return false;
        }
        std::string hex;
        if (!read_text(dir + "/" + file->str + ".hex", hex)) {
            fail("cannot read " + file->str + ".hex");
            return false;
        }
        std::vector<std::uint8_t> bytes;
        if (!hex_decode(trim(hex), bytes) || bytes.size() != want_len) {
            fail("primitive " + file->str + ": bad hex/len");
            return false;
        }
        if (!match_prim(*value, bytes.data(), prim)) {
            fail("primitive " + file->str + ": decoded value mismatch");
            return false;
        }
        c.prim_pass++;
    }
    return true;
}

bool run_composites(const std::string &dir, Counts &c) {
    std::vector<std::string> paths;
    std::error_code ec;
    for (const auto &e : std::filesystem::directory_iterator(dir, ec)) {
        if (e.path().extension() == ".json") {
            paths.push_back(e.path().string());
        }
    }
    if (ec || paths.empty()) {
        fail("no composite .json fixtures under " + dir);
        return false;
    }
    std::sort(paths.begin(), paths.end());
    for (const auto &path : paths) {
        c.comp_total++;
        const std::string stem = std::filesystem::path(path).stem().string();
        std::string text;
        if (!read_text(path, text)) {
            fail("cannot read " + path);
            return false;
        }
        Json root;
        if (!JsonParser(text).parse(root)) {
            fail(stem + ".json: parse error");
            return false;
        }
        const Json *schema = root.find("schema");
        const Json *values = root.find("values");
        const Json *len = root.find("len");
        if (schema == nullptr || values == nullptr || len == nullptr) {
            fail(stem + ".json: missing schema/values/len");
            return false;
        }
        std::vector<Field> fields;
        if (!parse_schema(schema->str, fields)) {
            fail(stem + ": schema parse error");
            return false;
        }
        std::string hex;
        if (!read_text(dir + "/" + stem + ".hex", hex)) {
            fail("cannot read " + stem + ".hex");
            return false;
        }
        std::vector<std::uint8_t> bytes;
        if (!hex_decode(trim(hex), bytes)) {
            fail(stem + ": bad hex");
            return false;
        }
        std::uint64_t want_len = 0;
        if (!num_u64(*len, want_len) || want_len != bytes.size()) {
            fail(stem + ": len field != .hex length");
            return false;
        }
        std::string err;
        if (!decode_struct(bytes, fields, *values, err)) {
            fail(stem + ": " + err);
            return false;
        }
        c.comp_pass++;
    }
    return true;
}

/* Descriptor golden: a field table for a canonical ChannelDescriptor image.
 * Byte-identity checks: field hex length == field size, fields tile [0, size)
 * with no overlap, and every decoded field equals the frozen semantic value
 * (cross-checked against crates/drm-core/tests/descriptor_golden.rs). */
bool run_descriptor(const std::string &path, Counts &c) {
    c.desc_total = 1;
    std::string text;
    if (!read_text(path, text)) {
        fail("cannot read " + path);
        return false;
    }
    Json root;
    if (!JsonParser(text).parse(root)) {
        fail("descriptor_golden.json: parse error");
        return false;
    }
    const Json *fields = root.find("fields");
    const Json *dsize = root.find("descriptor_size");
    const Json *align = root.find("align");
    if (fields == nullptr || dsize == nullptr || align == nullptr || fields->k != Json::K::Arr) {
        fail("descriptor_golden.json: missing fields/descriptor_size/align");
        return false;
    }
    std::uint64_t dsize_v = 0;
    std::uint64_t align_v = 0;
    if (!num_u64(*dsize, dsize_v) || !num_u64(*align, align_v) || dsize_v != 152 || align_v != 8) {
        fail("descriptor_golden.json: unexpected descriptor_size/align");
        return false;
    }

    std::vector<std::uint8_t> img(dsize_v, 0);
    std::vector<bool> covered(dsize_v, false);
    for (const auto &f : fields->arr) {
        const Json *name = f.find("name");
        const Json *off = f.find("offset");
        const Json *size = f.find("size");
        const Json *hex = f.find("hex");
        if (name == nullptr || off == nullptr || size == nullptr || hex == nullptr) {
            fail("descriptor field missing metadata");
            return false;
        }
        std::uint64_t off_v = 0;
        std::uint64_t size_v = 0;
        if (!num_u64(*off, off_v) || !num_u64(*size, size_v)) {
            fail("descriptor field " + name->str + ": bad offset/size");
            return false;
        }
        std::vector<std::uint8_t> bytes;
        if (!hex_decode(hex->str, bytes) || bytes.size() != size_v) {
            fail("descriptor field " + name->str + ": hex length != size");
            return false;
        }
        if (off_v + size_v > dsize_v) {
            fail("descriptor field " + name->str + ": out of range");
            return false;
        }
        for (std::uint64_t i = 0; i < size_v; ++i) {
            if (covered[static_cast<std::size_t>(off_v + i)]) {
                fail("descriptor field " + name->str + ": overlaps another field");
                return false;
            }
            img[static_cast<std::size_t>(off_v + i)] = bytes[static_cast<std::size_t>(i)];
            covered[static_cast<std::size_t>(off_v + i)] = true;
        }
    }
    for (std::uint64_t b = 0; b < dsize_v; ++b) {
        if (!covered[static_cast<std::size_t>(b)]) {
            fail("descriptor table does not tile [0, descriptor_size)");
            return false;
        }
    }

    /* Frozen semantic values (mirrors descriptor_golden.rs golden_descriptor). */
    struct Expect {
        const char *name;
        std::uint64_t value;
    };
    static const Expect kScalars[] = {
        {"flags", 1},
        {"pad_header", 0},
        {"topic_handle", 7},
        {"topic_name_len", 10},
        {"overflow_policy", 1},
        {"pad0", 0},
        {"payload_capacity", 64},
        {"history_depth", 4},
        {"pool_size", 8},
        {"consumer_count", 3},
        {"required_count", 1},
        {"diagnostic_count", 1},
        {"pad_counts", 0},
        {"ccb_offset", 184},
        {"ring_offset", 248},
        {"consumer_offset", 264},
        {"slot_generation_offset", 288},
        {"payload_offset", 304},
        {"segment_size", 816},
    };
    for (const auto &f : fields->arr) {
        const std::string name = f.find("name")->str;
        const std::uint64_t off_v = [&] {
            std::uint64_t v = 0;
            (void)num_u64(*f.find("offset"), v);
            return v;
        }();
        const std::uint64_t size_v = [&] {
            std::uint64_t v = 0;
            (void)num_u64(*f.find("size"), v);
            return v;
        }();
        bool ok = false;
        if (name == "schema_hash") {
            ok = size_v == 16;
            for (std::uint64_t i = 0; ok && i < 16; ++i) {
                ok = img[static_cast<std::size_t>(off_v + i)] == static_cast<std::uint8_t>(i);
            }
        } else if (name == "layout_hash") {
            ok = size_v == 16;
            for (std::uint64_t i = 0; ok && i < 16; ++i) {
                ok = img[static_cast<std::size_t>(off_v + i)] == static_cast<std::uint8_t>(16 + i);
            }
        } else if (name == "reserved" || name.rfind("pad", 0) == 0) {
            ok = true;
            for (std::uint64_t i = 0; ok && i < size_v; ++i) {
                ok = img[static_cast<std::size_t>(off_v + i)] == 0;
            }
        } else {
            for (const auto &e : kScalars) {
                if (name == e.name) {
                    ok = size_v <= 8 &&
                         read_le(img.data() + off_v, static_cast<std::size_t>(size_v)) == e.value;
                    break;
                }
            }
        }
        if (!ok) {
            fail("descriptor field " + name + ": decoded value mismatch");
            return false;
        }
    }
    c.desc_pass = 1;
    return true;
}

} // namespace

int main(int argc, char **argv) {
    const std::string root = (argc > 1) ? argv[1] : "tests/fixtures/golden";

    Counts c;
    bool ok = true;
    ok = run_primitives(root + "/primitives", c) && ok;
    ok = run_composites(root + "/composites", c) && ok;
    ok = run_descriptor(root + "/descriptor/descriptor_golden.json", c) && ok;

    const std::size_t total_pass = c.prim_pass + c.comp_pass + c.desc_pass;
    const std::size_t total = c.prim_total + c.comp_total + c.desc_total;
    if (!ok || total_pass != total) {
        std::fprintf(stderr, "golden_decode: FAIL %zu/%zu vectors\n", total_pass, total);
        return 1;
    }
    std::printf("golden_decode: PASS primitives=%zu/%zu composites=%zu/%zu descriptor=%zu/%zu "
                "vectors=%zu/%zu\n",
                c.prim_pass, c.prim_total, c.comp_pass, c.comp_total, c.desc_pass, c.desc_total,
                total_pass, total);
    return 0;
}
