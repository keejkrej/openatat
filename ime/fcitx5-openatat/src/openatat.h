// SPDX-License-Identifier: Apache-2.0
// Fcitx5 module: detect committed `@@`, swallow it, notify openatatd.
// The overlay stays in the native applet. This addon does not own UI.

#ifndef OPENATAT_FCITX5_H
#define OPENATAT_FCITX5_H

#include <memory>
#include <vector>

#include <fcitx-utils/handlertable.h>
#include <fcitx-utils/signals.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/instance.h>
#include <fcitx/inputcontextproperty.h>

#include "openatat_filter.h"

namespace fcitx {

class InputContext;
class KeyEvent;

class OpenAtatState : public InputContextProperty {
public:
    openatat::ImeFilter filter;
};

class OpenAtatModule : public AddonInstance {
public:
    explicit OpenAtatModule(Instance *instance);
    ~OpenAtatModule() override;

private:
    openatat::FieldKind probeField(InputContext *ic) const;
    void onKey(KeyEvent &keyEvent);
    void onCommit(InputContext *ic, std::string &orig);
    void fire(InputContext *ic, bool deletePreviousAt);

    Instance *instance_;
    FactoryFor<OpenAtatState> factory_;
    std::vector<std::unique_ptr<HandlerTableEntry<EventHandler>>> eventWatchers_;
    Connection commitConn_;
};

class OpenAtatFactory : public AddonFactory {
public:
    AddonInstance *create(AddonManager *manager) override;
};

} // namespace fcitx

#endif
