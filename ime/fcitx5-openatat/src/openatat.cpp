// SPDX-License-Identifier: Apache-2.0

#include "openatat.h"

#include <fcitx-utils/capabilityflags.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx-utils/log.h>
#include <fcitx-utils/utf8.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputcontextmanager.h>
#include "openatat_secure.h"
#include "openatat_socket.h"

namespace fcitx {
namespace {

bool isNavigationOrEdit(KeySym sym) {
    switch (sym) {
    case FcitxKey_BackSpace:
    case FcitxKey_Delete:
    case FcitxKey_Left:
    case FcitxKey_Right:
    case FcitxKey_Up:
    case FcitxKey_Down:
    case FcitxKey_Home:
    case FcitxKey_End:
    case FcitxKey_Page_Up:
    case FcitxKey_Page_Down:
    case FcitxKey_Return:
    case FcitxKey_KP_Enter:
    case FcitxKey_Tab:
    case FcitxKey_Escape:
        return true;
    default:
        return false;
    }
}

bool hasBlockingModifier(const Key &key) {
    const auto states = key.states();
    return states.test(KeyState::Ctrl) || states.test(KeyState::Alt) ||
           states.test(KeyState::Super) || states.test(KeyState::Mod1);
}

} // namespace

OpenAtatModule::OpenAtatModule(Instance *instance)
    : instance_(instance),
      factory_([](InputContext &) { return new OpenAtatState; }) {
    if (instance_ == nullptr) {
        return;
    }
    instance_->inputContextManager().registerProperty("openatatState",
                                                      &factory_);

    eventWatchers_.emplace_back(instance_->watchEvent(
        EventType::InputContextKeyEvent, EventWatcherPhase::PostInputMethod,
        [this](Event &event) {
            onKey(static_cast<KeyEvent &>(event));
        }));

    eventWatchers_.emplace_back(instance_->watchEvent(
        EventType::InputContextFocusOut, EventWatcherPhase::PostInputMethod,
        [this](Event &event) {
            auto *ic = static_cast<InputContextEvent &>(event).inputContext();
            ic->propertyFor(&factory_)->filter.clear();
        }));

    eventWatchers_.emplace_back(instance_->watchEvent(
        EventType::InputContextReset, EventWatcherPhase::PostInputMethod,
        [this](Event &event) {
            auto *ic = static_cast<InputContextEvent &>(event).inputContext();
            ic->propertyFor(&factory_)->filter.clear();
        }));

    commitConn_ = instance_->connect<Instance::CommitFilter>(
        [this](InputContext *ic, std::string &orig) { onCommit(ic, orig); });
}

OpenAtatModule::~OpenAtatModule() = default;

openatat::FieldKind OpenAtatModule::probeField(InputContext *ic) const {
    // Never cache. Fcitx5 Password/Sensitive is the IME-equivalent of
    // AT-SPI Role::PasswordText; clients set it before the key is delivered.
    if (ic != nullptr &&
        ic->capabilityFlags().test(CapabilityFlag::PasswordOrSensitive)) {
        return openatat::FieldKind::Secure;
    }
    const openatat::FieldKind atspi = openatat::probe_atspi_field();
    if (atspi == openatat::FieldKind::Secure) {
        return openatat::FieldKind::Secure;
    }
    return openatat::FieldKind::AcceptsText;
}

void OpenAtatModule::onKey(KeyEvent &keyEvent) {
    if (keyEvent.isRelease() || keyEvent.filtered()) {
        return;
    }
    InputContext *ic = keyEvent.inputContext();
    if (ic == nullptr) {
        return;
    }
    auto *state = ic->propertyFor(&factory_);
    const openatat::FieldKind kind = probeField(ic);

    // Compose / preedit: two `@` in preedit must not fire or mutate the buffer.
    if (instance_->isComposing(ic)) {
        state->filter.on_key(kind, true, nullptr);
        return;
    }

    if (kind == openatat::FieldKind::Secure) {
        state->filter.on_key(kind, false, nullptr);
        return;
    }

    if (isNavigationOrEdit(keyEvent.key().sym())) {
        state->filter.clear();
        return;
    }
    if (hasBlockingModifier(keyEvent.key())) {
        return;
    }

    const uint32_t unicode = Key::keySymToUnicode(keyEvent.key().sym());
    if (unicode == 0) {
        return;
    }
    const std::string text = utf8::UCS4ToUTF8(unicode);
    const bool hadAt = state->filter.buffer().single_at();
    const openatat::ImeAction action =
        state->filter.on_key(kind, false, text.c_str());
    if (action == openatat::ImeAction::FireTrigger) {
        keyEvent.filterAndAccept();
        fire(ic, hadAt && openatat::is_at(unicode));
    }
}

void OpenAtatModule::onCommit(InputContext *ic, std::string &orig) {
    if (ic == nullptr || orig.empty()) {
        return;
    }
    auto *state = ic->propertyFor(&factory_);
    const openatat::FieldKind kind = probeField(ic);

    if (instance_->isComposing(ic)) {
        state->filter.on_key(kind, true, nullptr);
        return;
    }

    const bool hadAt = state->filter.buffer().single_at();
    const int fromCommit = openatat::trailing_at_count(orig, 2);
    const openatat::ImeAction action =
        state->filter.on_key(kind, false, orig.c_str());
    if (action == openatat::ImeAction::Ignore) {
        // Secure: do not look at (or rewrite) the keystream.
        return;
    }
    if (action == openatat::ImeAction::FireTrigger) {
        orig = openatat::drop_trailing_codepoints(orig, fromCommit);
        fire(ic, hadAt && fromCommit < 2);
    }
}

void OpenAtatModule::fire(InputContext *ic, bool deletePreviousAt) {
    if (deletePreviousAt && ic != nullptr) {
        ic->deleteSurroundingText(-1, 1);
    }
    std::string app;
    if (ic != nullptr) {
        app = ic->program();
    }
    // Missing daemon socket: fail quietly. Daemon may start later.
    if (!openatat::send_ime_trigger(app)) {
        FCITX_DEBUG() << "openatat: trigger.sock missing; swallowed @@ anyway";
    }
}

AddonInstance *OpenAtatFactory::create(AddonManager *manager) {
    return new OpenAtatModule(manager->instance());
}

} // namespace fcitx

#if defined(FCITX_ADDON_FACTORY_V2_BACKWARDS)
FCITX_ADDON_FACTORY_V2_BACKWARDS(openatat, fcitx::OpenAtatFactory);
#elif defined(FCITX_ADDON_FACTORY_V2)
FCITX_ADDON_FACTORY_V2(openatat, fcitx::OpenAtatFactory);
#else
FCITX_ADDON_FACTORY(fcitx::OpenAtatFactory);
#endif
