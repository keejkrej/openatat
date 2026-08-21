// SPDX-License-Identifier: Apache-2.0
// Mirrors crates/openatatd/src/trigger/{mod,ime}.rs tests. No Fcitx5, no display.

#include "openatat_filter.h"
#include "openatat_socket.h"

#include <cstdlib>
#include <iostream>

namespace {

int g_failed = 0;

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond)) {                                                         \
            std::cerr << "FAIL " << __FILE__ << ":" << __LINE__ << " "         \
                      << #cond << "\n";                                        \
            ++g_failed;                                                        \
        }                                                                      \
    } while (0)

using openatat::DetectionBuffer;
using openatat::FieldKind;
using openatat::ImeAction;
using openatat::ImeFilter;

void test_detection_buffer() {
    {
        DetectionBuffer b;
        CHECK(!b.push_committed("hello"));
        CHECK(b.len() == 2);
        CHECK(b.as_chars() == std::make_pair(U'l', U'o'));
    }
    {
        DetectionBuffer b;
        CHECK(!b.push_committed("@"));
        CHECK(b.push_committed("@"));
    }
    {
        DetectionBuffer b;
        CHECK(!b.push_committed("x"));
        CHECK(!b.push_committed("@"));
        CHECK(b.push_committed("@"));
    }
    {
        DetectionBuffer b;
        CHECK(b.push_committed("@@"));
    }
    {
        DetectionBuffer b;
        CHECK(!b.push_committed("@"));
        CHECK(!b.is_trigger());
    }
    {
        DetectionBuffer b;
        CHECK(b.push_committed("＠＠"));
    }
    {
        DetectionBuffer b;
        CHECK(b.push_committed("＠@"));
    }
}

void test_ime_filter() {
    {
        ImeFilter ime;
        CHECK(ime.on_key(FieldKind::AcceptsText, true, "@@") == ImeAction::Ignore);
        CHECK(!ime.buffer().is_trigger());
        CHECK(ime.buffer().len() == 0);
    }
    {
        ImeFilter ime;
        CHECK(ime.on_key(FieldKind::AcceptsText, false, "@") == ImeAction::Continue);
        CHECK(ime.on_key(FieldKind::AcceptsText, false, "@") ==
              ImeAction::FireTrigger);
        CHECK(ime.buffer().len() == 0);
    }
    {
        ImeFilter ime;
        CHECK(ime.on_key(FieldKind::AcceptsText, false, "@") == ImeAction::Continue);
        CHECK(ime.on_key(FieldKind::Secure, false, "@") == ImeAction::Ignore);
        CHECK(ime.buffer().len() == 0);
        CHECK(ime.on_key(FieldKind::AcceptsText, false, "@") == ImeAction::Continue);
        CHECK(!ime.buffer().is_trigger());
    }
    {
        ImeFilter ime;
        CHECK(ime.on_key(FieldKind::AcceptsText, true, "@") == ImeAction::Ignore);
        CHECK(ime.on_key(FieldKind::AcceptsText, false, "@@") ==
              ImeAction::FireTrigger);
    }
    {
        ImeFilter ime;
        CHECK(ime.buffer().single_at() == false);
        ime.on_key(FieldKind::AcceptsText, false, "@");
        CHECK(ime.buffer().single_at());
    }
}

void test_trigger_json() {
    const std::string line =
        openatat::trigger_request_json("kitty", "0xabc", "DP-1");
    CHECK(line.find("\"cmd\":\"trigger\"") != std::string::npos);
    CHECK(line.find("\"source\":\"ime\"") != std::string::npos);
    CHECK(line.find("\"app_id\":\"kitty\"") != std::string::npos);
    CHECK(line.find("\"window_address\":\"0xabc\"") != std::string::npos);
    CHECK(line.find("\"output\":\"DP-1\"") != std::string::npos);
    CHECK(line.back() == '\n');
    (void)openatat::send_ime_trigger(); // missing socket must not crash
}

void test_swallow_helpers() {
    CHECK(openatat::trailing_at_count("@@") == 2);
    CHECK(openatat::trailing_at_count("hello@@") == 2);
    CHECK(openatat::trailing_at_count("hello@") == 1);
    CHECK(openatat::trailing_at_count("hello") == 0);
    CHECK(openatat::trailing_at_count("＠＠") == 2);
    CHECK(openatat::drop_trailing_codepoints("hello@@", 2) == "hello");
    CHECK(openatat::drop_trailing_codepoints("@@", 2).empty());
    CHECK(openatat::drop_trailing_codepoints("x@", 1) == "x");
}

} // namespace

int main() {
    test_detection_buffer();
    test_ime_filter();
    test_swallow_helpers();
    test_trigger_json();
    if (g_failed != 0) {
        std::cerr << g_failed << " check(s) failed\n";
        return 1;
    }
    std::cout << "openatat-filter-test: ok\n";
    return 0;
}
