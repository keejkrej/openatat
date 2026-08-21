// SPDX-License-Identifier: Apache-2.0

#include "openatat_filter.h"

#include <vector>

namespace openatat {
namespace {

char32_t next_codepoint(const char *&it, const char *end) {
    if (it >= end) {
        return 0;
    }
    const auto c = static_cast<unsigned char>(*it++);
    if (c < 0x80) {
        return c;
    }
    int extra = 0;
    char32_t u = 0;
    if ((c & 0xE0) == 0xC0) {
        extra = 1;
        u = c & 0x1F;
    } else if ((c & 0xF0) == 0xE0) {
        extra = 2;
        u = c & 0x0F;
    } else if ((c & 0xF8) == 0xF0) {
        extra = 3;
        u = c & 0x07;
    } else {
        return 0xFFFD;
    }
    if (it + extra > end) {
        it = end;
        return 0xFFFD;
    }
    for (int i = 0; i < extra; ++i) {
        const auto cc = static_cast<unsigned char>(*it++);
        if ((cc & 0xC0) != 0x80) {
            return 0xFFFD;
        }
        u = (u << 6) | (cc & 0x3F);
    }
    return u;
}

std::vector<size_t> codepoint_offsets(std::string_view text) {
    std::vector<size_t> offs;
    offs.push_back(0);
    const char *it = text.data();
    const char *end = it + text.size();
    while (it < end) {
        next_codepoint(it, end);
        offs.push_back(static_cast<size_t>(it - text.data()));
    }
    return offs;
}

} // namespace

bool DetectionBuffer::is_trigger() const {
    return len_ == 2 && is_at(chars_[0]) && is_at(chars_[1]);
}

bool DetectionBuffer::single_at() const { return len_ == 1 && is_at(chars_[0]); }

void DetectionBuffer::clear() {
    chars_[0] = 0;
    chars_[1] = 0;
    len_ = 0;
}

void DetectionBuffer::push_char(char32_t ch) {
    ch = normalize_at(ch);
    switch (len_) {
    case 0:
        chars_[0] = ch;
        len_ = 1;
        break;
    case 1:
        chars_[1] = ch;
        len_ = 2;
        break;
    default:
        chars_[0] = chars_[1];
        chars_[1] = ch;
        len_ = 2;
        break;
    }
}

bool DetectionBuffer::push_committed(std::string_view text) {
    const char *it = text.data();
    const char *end = it + text.size();
    while (it < end) {
        const char32_t ch = next_codepoint(it, end);
        if (ch != 0) {
            push_char(ch);
        }
    }
    return is_trigger();
}

int trailing_at_count(std::string_view text, int max) {
    if (max <= 0 || text.empty()) {
        return 0;
    }
    std::vector<char32_t> cps;
    const char *it = text.data();
    const char *end = it + text.size();
    while (it < end) {
        const char32_t ch = next_codepoint(it, end);
        if (ch != 0) {
            cps.push_back(ch);
        }
    }
    int n = 0;
    for (auto i = static_cast<int>(cps.size()) - 1; i >= 0 && n < max; --i) {
        if (!is_at(cps[static_cast<size_t>(i)])) {
            break;
        }
        ++n;
    }
    return n;
}

std::string drop_trailing_codepoints(std::string_view text, int n) {
    if (n <= 0) {
        return std::string(text);
    }
    const auto offs = codepoint_offsets(text);
    const int cps = static_cast<int>(offs.size()) - 1;
    if (n >= cps) {
        return {};
    }
    return std::string(text.substr(0, offs[static_cast<size_t>(cps - n)]));
}

ImeAction ImeFilter::on_field(FieldKind kind) {
    if (kind == FieldKind::Secure) {
        buffer_.clear();
        return ImeAction::Ignore;
    }
    return ImeAction::Continue;
}

ImeAction ImeFilter::on_compose() {
    // Do not touch the two-character buffer while composing.
    return ImeAction::Ignore;
}

ImeAction ImeFilter::on_commit(std::string_view text) {
    if (buffer_.push_committed(text)) {
        buffer_.clear();
        return ImeAction::FireTrigger;
    }
    return ImeAction::Continue;
}

ImeAction ImeFilter::on_key(FieldKind kind, bool composing, const char *committed) {
    const ImeAction field = on_field(kind);
    if (field == ImeAction::Ignore) {
        return ImeAction::Ignore;
    }
    if (composing) {
        return on_compose();
    }
    if (committed != nullptr) {
        return on_commit(committed);
    }
    return field;
}

} // namespace openatat
