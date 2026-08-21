// SPDX-License-Identifier: Apache-2.0
// Secure-field probe. Never cache the result across keys.

#ifndef OPENATAT_SECURE_H
#define OPENATAT_SECURE_H

#include "openatat_filter.h"

namespace openatat {

// Best-effort AT-SPI PasswordText walk. Returns Other if the bus is missing
// or the walk times out. Callers must still treat Fcitx5 Password/Sensitive
// capability flags as the IME-equivalent and re-probe every key.
FieldKind probe_atspi_field();

} // namespace openatat

#endif
