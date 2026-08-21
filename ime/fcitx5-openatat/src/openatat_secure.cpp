// SPDX-License-Identifier: Apache-2.0
// Optional AT-SPI probe. Capability flags remain the IME-native equivalent.

#include "openatat_secure.h"

#ifdef OPENATAT_HAVE_DBUS
#include <dbus/dbus.h>

#include <cstdint>
#include <cstring>
#include <queue>
#include <string>
#include <utility>

namespace openatat {
namespace {

// AT-SPI Role::PasswordText. 40 is the historical atspi/Odilia value;
// 45 appears in some at-spi2-core headers. Treat both as secure.
constexpr uint32_t kRolePasswordTextA = 40;
constexpr uint32_t kRolePasswordTextB = 45;
constexpr uint32_t kStateFocused = 1u << 12;
constexpr uint32_t kStateDefunct = 1u << 6;
constexpr int kTimeoutMs = 20;
constexpr int kMaxNodes = 80;

DBusConnection *open_atspi_bus() {
    DBusError err;
    dbus_error_init(&err);
    DBusConnection *session = dbus_bus_get_private(DBUS_BUS_SESSION, &err);
    if (session == nullptr) {
        dbus_error_free(&err);
        return nullptr;
    }
    dbus_connection_set_exit_on_disconnect(session, false);

    DBusMessage *msg = dbus_message_new_method_call(
        "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress");
    if (msg == nullptr) {
        dbus_connection_close(session);
        dbus_connection_unref(session);
        return nullptr;
    }
    DBusMessage *reply = dbus_connection_send_with_reply_and_block(
        session, msg, kTimeoutMs, &err);
    dbus_message_unref(msg);
    dbus_connection_close(session);
    dbus_connection_unref(session);
    if (reply == nullptr) {
        dbus_error_free(&err);
        return nullptr;
    }
    const char *addr = nullptr;
    if (!dbus_message_get_args(reply, &err, DBUS_TYPE_STRING, &addr,
                               DBUS_TYPE_INVALID) ||
        addr == nullptr) {
        dbus_message_unref(reply);
        dbus_error_free(&err);
        return nullptr;
    }
    std::string address(addr);
    dbus_message_unref(reply);

    dbus_error_init(&err);
    DBusConnection *a11y = dbus_connection_open_private(address.c_str(), &err);
    if (a11y == nullptr) {
        dbus_error_free(&err);
        return nullptr;
    }
    dbus_connection_set_exit_on_disconnect(a11y, false);
    if (!dbus_bus_register(a11y, &err)) {
        dbus_error_free(&err);
        dbus_connection_close(a11y);
        dbus_connection_unref(a11y);
        return nullptr;
    }
    return a11y;
}

bool get_role_state(DBusConnection *conn, const char *dest, const char *path,
                    uint32_t *role, uint32_t *state0) {
    DBusError err;
    dbus_error_init(&err);
    DBusMessage *msg = dbus_message_new_method_call(
        dest, path, "org.a11y.atspi.Accessible", "GetRole");
    if (msg == nullptr) {
        return false;
    }
    DBusMessage *reply =
        dbus_connection_send_with_reply_and_block(conn, msg, kTimeoutMs, &err);
    dbus_message_unref(msg);
    if (reply == nullptr) {
        dbus_error_free(&err);
        return false;
    }
    dbus_uint32_t r = 0;
    if (!dbus_message_get_args(reply, &err, DBUS_TYPE_UINT32, &r,
                               DBUS_TYPE_INVALID)) {
        dbus_message_unref(reply);
        dbus_error_free(&err);
        return false;
    }
    dbus_message_unref(reply);
    *role = r;

    dbus_error_init(&err);
    msg = dbus_message_new_method_call(dest, path, "org.a11y.atspi.Accessible",
                                       "GetState");
    if (msg == nullptr) {
        return false;
    }
    reply = dbus_connection_send_with_reply_and_block(conn, msg, kTimeoutMs, &err);
    dbus_message_unref(msg);
    if (reply == nullptr) {
        dbus_error_free(&err);
        return false;
    }
    DBusMessageIter iter;
    dbus_message_iter_init(reply, &iter);
    *state0 = 0;
    if (dbus_message_iter_get_arg_type(&iter) == DBUS_TYPE_ARRAY) {
        DBusMessageIter arr;
        dbus_message_iter_recurse(&iter, &arr);
        if (dbus_message_iter_get_arg_type(&arr) == DBUS_TYPE_UINT32) {
            dbus_uint32_t s = 0;
            dbus_message_iter_get_basic(&arr, &s);
            *state0 = s;
        }
    }
    dbus_message_unref(reply);
    return true;
}

void enqueue_children(DBusConnection *conn, const char *dest, const char *path,
                      std::queue<std::pair<std::string, std::string>> &q) {
    DBusError err;
    dbus_error_init(&err);
    DBusMessage *msg = dbus_message_new_method_call(
        dest, path, "org.a11y.atspi.Accessible", "GetChildren");
    if (msg == nullptr) {
        return;
    }
    DBusMessage *reply =
        dbus_connection_send_with_reply_and_block(conn, msg, kTimeoutMs, &err);
    dbus_message_unref(msg);
    if (reply == nullptr) {
        dbus_error_free(&err);
        return;
    }
    DBusMessageIter iter;
    if (!dbus_message_iter_init(reply, &iter) ||
        dbus_message_iter_get_arg_type(&iter) != DBUS_TYPE_ARRAY) {
        dbus_message_unref(reply);
        return;
    }
    DBusMessageIter arr;
    dbus_message_iter_recurse(&iter, &arr);
    while (dbus_message_iter_get_arg_type(&arr) == DBUS_TYPE_STRUCT) {
        DBusMessageIter st;
        dbus_message_iter_recurse(&arr, &st);
        const char *child_dest = nullptr;
        const char *child_path = nullptr;
        if (dbus_message_iter_get_arg_type(&st) == DBUS_TYPE_STRING) {
            dbus_message_iter_get_basic(&st, &child_dest);
            dbus_message_iter_next(&st);
        }
        if (dbus_message_iter_get_arg_type(&st) == DBUS_TYPE_OBJECT_PATH) {
            dbus_message_iter_get_basic(&st, &child_path);
        }
        if (child_dest != nullptr && child_path != nullptr) {
            q.emplace(child_dest, child_path);
        }
        dbus_message_iter_next(&arr);
    }
    dbus_message_unref(reply);
}

} // namespace

FieldKind probe_atspi_field() {
    DBusConnection *conn = open_atspi_bus();
    if (conn == nullptr) {
        return FieldKind::Other;
    }

    std::queue<std::pair<std::string, std::string>> q;
    q.emplace("org.a11y.atspi.Registry",
              "/org/a11y/atspi/accessible/root");
    int seen = 0;
    FieldKind result = FieldKind::Other;
    while (!q.empty() && seen < kMaxNodes) {
        ++seen;
        const auto node = q.front();
        q.pop();
        uint32_t role = 0;
        uint32_t state0 = 0;
        if (!get_role_state(conn, node.first.c_str(), node.second.c_str(), &role,
                            &state0)) {
            continue;
        }
        if ((state0 & kStateDefunct) != 0) {
            continue;
        }
        if ((state0 & kStateFocused) != 0) {
            if (role == kRolePasswordTextA || role == kRolePasswordTextB) {
                result = FieldKind::Secure;
            } else {
                result = FieldKind::AcceptsText;
            }
            break;
        }
        enqueue_children(conn, node.first.c_str(), node.second.c_str(), q);
    }

    dbus_connection_close(conn);
    dbus_connection_unref(conn);
    return result;
}

} // namespace openatat

#else

namespace openatat {

FieldKind probe_atspi_field() { return FieldKind::Other; }

} // namespace openatat

#endif
