// SPDX-License-Identifier: Apache-2.0

#include "openatat_socket.h"

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#ifdef _WIN32
bool openatat::send_ime_trigger(std::string_view, std::string_view,
                                std::string_view) {
    return false;
}
std::string openatat::trigger_socket_path() { return {}; }
#else
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

namespace openatat {
namespace {

std::string json_escape(std::string_view s) {
    std::string out;
    out.reserve(s.size() + 8);
    for (unsigned char c : s) {
        switch (c) {
        case '"':
            out += "\\\"";
            break;
        case '\\':
            out += "\\\\";
            break;
        case '\n':
            out += "\\n";
            break;
        case '\r':
            out += "\\r";
            break;
        case '\t':
            out += "\\t";
            break;
        default:
            if (c < 0x20) {
                char buf[8];
                std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                out += buf;
            } else {
                out += static_cast<char>(c);
            }
            break;
        }
    }
    return out;
}

void append_optional(std::string &json, bool &first, const char *key,
                     std::string_view value) {
    if (value.empty()) {
        return;
    }
    if (!first) {
        json += ',';
    }
    first = false;
    json += '"';
    json += key;
    json += "\":\"";
    json += json_escape(value);
    json += '"';
}

} // namespace

std::string trigger_socket_path() {
    const char *runtime = std::getenv("XDG_RUNTIME_DIR");
    if (runtime == nullptr || runtime[0] == '\0') {
        return {};
    }
    return std::string(runtime) + "/openatat/trigger.sock";
}

std::string trigger_request_json(std::string_view app_id,
                                 std::string_view window_address,
                                 std::string_view output) {
    std::string json = R"({"cmd":"trigger","source":"ime","focus":{)";
    bool first = true;
    append_optional(json, first, "app_id", app_id);
    append_optional(json, first, "window_address", window_address);
    append_optional(json, first, "output", output);
    json += "}}\n";
    return json;
}

bool send_ime_trigger(std::string_view app_id, std::string_view window_address,
                      std::string_view output) {
    const std::string path = trigger_socket_path();
    if (path.empty() || path.size() >= sizeof(sockaddr_un::sun_path)) {
        return false;
    }

    const int fd = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) {
        return false;
    }

    timeval tv{};
    tv.tv_sec = 0;
    tv.tv_usec = 200000;
    ::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    ::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    std::memcpy(addr.sun_path, path.c_str(), path.size() + 1);

    if (::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) < 0) {
        ::close(fd);
        return false;
    }

    const std::string line = trigger_request_json(app_id, window_address, output);
    const char *p = line.data();
    size_t left = line.size();
    while (left > 0) {
        const ssize_t n = ::write(fd, p, left);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            ::close(fd);
            return false;
        }
        p += n;
        left -= static_cast<size_t>(n);
    }
    ::close(fd);
    return true;
}

} // namespace openatat
#endif
