// SPDX-License-Identifier: Apache-2.0
// Same contract as crates/openatatd/src/trigger/{mod,ime}.rs (ImeFilter).
// The Fcitx5 addon is an adapter; this is not a second detector.

#ifndef OPENATAT_FILTER_H
#define OPENATAT_FILTER_H

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <utility>

namespace openatat {

enum class FieldKind {
    Secure,
    AcceptsText,
    Other,
};

enum class ImeAction {
    Ignore,
    Continue,
    FireTrigger,
};

// Last two committed characters. Nothing before or after is kept.
class DetectionBuffer {
public:
    DetectionBuffer() = default;

    uint8_t len() const { return len_; }
    bool is_trigger() const;
    bool single_at() const;
    void clear();

    // Push committed text only. Returns true when the buffer is `@@`.
    bool push_committed(std::string_view text);

    std::pair<char32_t, char32_t> as_chars() const { return {chars_[0], chars_[1]}; }

private:
    void push_char(char32_t ch);

    char32_t chars_[2] = {0, 0};
    uint8_t len_ = 0;
};

inline bool is_at(char32_t ch) { return ch == U'@' || ch == U'＠'; }
inline char32_t normalize_at(char32_t ch) { return ch == U'＠' ? U'@' : ch; }

// Count trailing `@` / `＠` code points, capped at `max`.
int trailing_at_count(std::string_view text, int max = 2);

// Drop the last `n` UTF-8 code points (used to swallow `@@` from a commit).
std::string drop_trailing_codepoints(std::string_view text, int n);

// In-process IME filter. Canonical per-key sequence: field, then compose or commit.
class ImeFilter {
public:
    ImeAction on_field(FieldKind kind);
    ImeAction on_compose();
    ImeAction on_commit(std::string_view text);

    // Probe field every call. Compose must Ignore. Secure is never cached.
    ImeAction on_key(FieldKind kind, bool composing, const char *committed);

    void clear() { buffer_.clear(); }
    const DetectionBuffer &buffer() const { return buffer_; }

private:
    DetectionBuffer buffer_;
};

} // namespace openatat

#endif
