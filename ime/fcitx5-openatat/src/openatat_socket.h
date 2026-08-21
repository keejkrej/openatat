// SPDX-License-Identifier: Apache-2.0
// Line-delimited JSON to $XDG_RUNTIME_DIR/openatat/trigger.sock

#ifndef OPENATAT_SOCKET_H
#define OPENATAT_SOCKET_H

#include <string>
#include <string_view>

namespace openatat {

std::string trigger_socket_path();
std::string trigger_request_json(std::string_view app_id,
                                 std::string_view window_address,
                                 std::string_view output);

// Fail quietly if the daemon socket is missing. Never throws.
bool send_ime_trigger(std::string_view app_id = {},
                      std::string_view window_address = {},
                      std::string_view output = {});

} // namespace openatat

#endif
