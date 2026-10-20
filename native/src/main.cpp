#include "platform.hpp"
#include "transport.hpp"
#include "ui.hpp"
#include "update_view.hpp"
#include "updater.hpp"
#include "wodex_core.h"
#include <SDL.h>
#include <SDL_ttf.h>
#include <algorithm>
#include <cstdlib>
#include <functional>
#include <iostream>
#include <memory>
#include <nlohmann/json.hpp>
#include <stdexcept>
#include <string>
#include <vector>
#ifdef __SWITCH__
#include <switch.h>
#endif

using Json = nlohmann::json;
using namespace wodex;

namespace {

Json owned_json(char *value) {
  if (!value) {
    throw std::runtime_error("The conversation core ran out of memory.");
  }
  std::unique_ptr<char, decltype(&wodex_string_free)> owned(value, wodex_string_free);
  return Json::parse(owned.get());
}

class Core {
public:
  explicit Core(const std::string &saved)
      : value_(wodex_new(saved.empty() ? nullptr : saved.c_str())) {
    if (!value_) {
      throw std::runtime_error("Cannot initialize conversation core.");
    }
  }
  ~Core() {
    wodex_free(value_);
  }
  Json call(const Json &command) {
    const auto serialized = command.dump();
    return owned_json(wodex_call(value_, serialized.c_str()));
  }
  Json feed(const std::string &data) {
    return owned_json(
        wodex_feed(value_, reinterpret_cast<const uint8_t *>(data.data()), data.size()));
  }

private:
  WodexCore *value_;
};

struct Button {
  std::string id;
  SDL_Rect area;
  std::function<void()> action;
};

struct Editor {
  bool open = false;
  bool secret = false;
  std::string title;
  std::string value;
  std::string composition;
  size_t cursor = 0;
  bool select_all = false;
  std::function<void(std::string)> done;
};

class App {
public:
  App(Platform &platform, SDL_Renderer *renderer, bool demo, std::string update_target,
      bool show_updates)
      : platform_(platform), renderer_(renderer), canvas_(renderer, platform),
        save_path_(platform.data_dir() + "/state.json"), core_(demo ? "" : load_file(save_path_)),
        update_target_(std::move(update_target)), updater_(WODEX_VERSION, update_target_),
        demo_(demo), updates_(show_updates) {
    refresh();
    if (demo_) {
      seed_demo();
    } else {
      auto exported = core_.call({{"op", "export"}});
      if (!exported.value("ok", false)) {
        // Preserve unreadable data under a visible, stable name before allowing saves.
        const auto original = load_file(save_path_);
        preserve_file(save_path_, original);
        core_.call({{"op", "reset_storage"}});
        notice_ = "Saved data could not be read. A recovery copy is beside state.json.";
        refresh();
      }
      if (const auto *key = std::getenv("DEEPSEEK_API_KEY"); key && *key) {
        const auto result = core_.call({{"op", "settings"}, {"settings", {{"api_key", key}}}});
        if (result.value("ok", false)) {
          // The environment key is used in memory; retain the previous disk value.
          stored_key_ = state_["settings"].value("api_key", "");
          environment_key_ = true;
        } else {
          notice_ = "DEEPSEEK_API_KEY is invalid. Enter a valid key in Settings.";
        }
        refresh();
      }
      if (notice_.empty() && state_["settings"].value("api_key", "").empty()) {
        notice_ = "Welcome to Wodex. Add your DeepSeek API key in Settings to begin.";
      }
    }
    for (int index = 0; index < SDL_NumJoysticks(); ++index) {
      open_controller(index);
    }
#ifdef __SWITCH__
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad_);
#endif
    if (updates_) {
      notice_.clear();
      focused_ = "update-source";
    }
  }

  ~App() {
    transport_.cancel();
    updater_.cancel();
    for (auto *controller : controllers_) {
      SDL_GameControllerClose(controller);
    }
  }

  bool running() const {
    return running_ && platform_.running();
  }

  std::optional<PendingUpdate> pending_install() const {
    return pending_update_;
  }

  void tick() {
    for (auto &event : transport_.poll()) {
      if (event.kind == TransportEvent::Data && streaming()) {
        const auto result = core_.feed(event.data);
        if (!result.value("ok", false)) {
          transport_.cancel();
          notice_ = result.value("error", "Could not read the response.");
        }
        refresh();
      } else if (event.kind == TransportEvent::Finished) {
        if (streaming()) {
          mutate({{"op", "finish"}, {"http_status", event.http_status}, {"error", event.error}});
        } else {
          save();
        }
      }
    }
    if (updates_ && focused_ == "update-cancel" && !updater_.active()) {
      const auto phase = updater_.status().phase;
      focused_ = phase == UpdatePhase::Ready                                  ? "update-install"
                 : phase == UpdatePhase::Available && !update_target_.empty() ? "update-download"
                                                                              : "update-check";
    }
    SDL_Event event;
    while (SDL_PollEvent(&event)) {
      if (event.type == SDL_QUIT) {
        close();
      } else if (event.type == SDL_CONTROLLERDEVICEADDED) {
        open_controller(event.cdevice.which);
      } else if (editor_.open) {
        handle_editor(event);
      } else if (event.type == SDL_KEYDOWN) {
        keyboard(event.key.keysym.sym, event.key.keysym.mod);
      } else if (event.type == SDL_CONTROLLERBUTTONDOWN) {
        gamepad(event.cbutton.button);
      } else if (event.type == SDL_MOUSEWHEEL && !settings_) {
        scroll_ = std::max(0, scroll_ - event.wheel.y * 80);
        follow_bottom_ = false;
      } else if (event.type == SDL_MOUSEBUTTONDOWN && event.button.button == SDL_BUTTON_LEFT) {
        // SDL's logical-size event filter already scales mouse/touch positions.
        click(event.button.x, event.button.y);
      } else if (event.type == SDL_FINGERUP) {
        click(static_cast<int>(event.tfinger.x * 1280), static_cast<int>(event.tfinger.y * 720));
      }
      if (!running_)
        return;
    }
#ifdef __SWITCH__
    padUpdate(&pad_);
    const u64 pressed = padGetButtonsDown(&pad_);
    if (pressed & HidNpadButton_Plus)
      close();
    if (!running_)
      return;
    if (pressed & HidNpadButton_Minus)
      toggle_settings();
    if (pressed & HidNpadButton_A)
      activate();
    if (pressed & HidNpadButton_B)
      back();
    if (pressed & HidNpadButton_X)
      compose();
    if (pressed & HidNpadButton_Y)
      show_reasoning_ = !show_reasoning_;
    if (pressed & HidNpadButton_Left)
      move_focus(-1, 0);
    if (pressed & HidNpadButton_Right)
      move_focus(1, 0);
    if (pressed & HidNpadButton_Up)
      move_focus(0, -1);
    if (pressed & HidNpadButton_Down)
      move_focus(0, 1);
    if (pressed & HidNpadButton_L)
      cycle_chat(-1);
    if (pressed & HidNpadButton_R)
      cycle_chat(1);
    const u64 held = padGetButtons(&pad_);
    if (held & HidNpadButton_ZL) {
      scroll_ = std::max(0, scroll_ - 12);
      follow_bottom_ = false;
    }
    if (held & HidNpadButton_ZR) {
      scroll_ += 12;
      follow_bottom_ = false;
    }
    const auto stick = padGetStickPos(&pad_, 1);
    if (std::abs(stick.y) > 8000) {
      scroll_ = std::max(0, scroll_ - stick.y / 2500);
      follow_bottom_ = false;
    }
#endif
    if (SDL_GetTicks64() - last_save_ > 10000 && streaming()) {
      save();
    }
  }

  void render() {
    buttons_.clear();
    canvas_.rect({0, 0, 1280, 720}, background);
    sidebar();
    if (updates_) {
      render_updates();
    } else if (settings_) {
      render_settings();
    } else {
      render_chat();
    }
    canvas_.rect({0, 674, 1280, 46}, background);
    canvas_.rect({24, 674, 1232, 1}, border);
    canvas_.text("A  Select     X  Write     B  Back / stop     −  Settings", 28, 687, 16, muted);
    canvas_.text("L / R  Chats     ZL / ZR  Scroll     +  Exit", 890, 687, 16, muted);
    if (editor_.open) {
      render_editor();
    }
  }

  void screenshot(const std::string &path) {
    int width = 0, height = 0;
    if (SDL_GetRendererOutputSize(renderer_, &width, &height) != 0 || width <= 0 || height <= 0) {
      throw std::runtime_error("Cannot determine screenshot size.");
    }
    auto *surface = SDL_CreateRGBSurfaceWithFormat(0, width, height, 32, SDL_PIXELFORMAT_RGBA32);
    if (!surface || SDL_RenderReadPixels(renderer_, nullptr, SDL_PIXELFORMAT_RGBA32,
                                         surface->pixels, surface->pitch) != 0) {
      SDL_FreeSurface(surface);
      throw std::runtime_error("Cannot read screenshot.");
    }
    auto *output = SDL_CreateRGBSurfaceWithFormat(0, 1280, 720, 24, SDL_PIXELFORMAT_RGB24);
    if (!output || SDL_BlitScaled(surface, nullptr, output, nullptr) != 0 ||
        SDL_SaveBMP(output, path.c_str()) != 0) {
      SDL_FreeSurface(output);
      SDL_FreeSurface(surface);
      throw std::runtime_error("Cannot save screenshot.");
    }
    SDL_FreeSurface(output);
    SDL_FreeSurface(surface);
  }

private:
  bool streaming() const {
    return state_.value("is_streaming", false);
  }
  bool busy() const {
    return streaming() || transport_.active();
  }
  void refresh() {
    state_ = core_.call({{"op", "snapshot"}}).at("state");
  }
  bool save() {
    if (demo_)
      return true;
    auto result = core_.call({{"op", "export"}});
    if (!result.value("ok", false))
      return false;
    if (environment_key_)
      result["data"]["settings"]["api_key"] = stored_key_;
    try {
      atomic_write(save_path_, result["data"].dump());
      last_save_ = SDL_GetTicks64();
      return true;
    } catch (const std::exception &) {
      notice_ = "Could not save to storage. Check free space and write permissions.";
      return false;
    }
  }
  Json mutate(const Json &command) {
    auto result = core_.call(command);
    if (!result.value("ok", false)) {
      notice_ = result.value("error", "This action could not be completed.");
    } else {
      notice_.clear();
    }
    refresh();
    save();
    return result;
  }
  void start(const Json &command) {
    if (demo_) {
      notice_ = "Offline demo. Relaunch without --demo to connect your DeepSeek account.";
      return;
    }
    if (busy())
      return;
    const auto result = mutate(command);
    if (!result.value("ok", false) || !result.contains("request"))
      return;
    const auto &request = result["request"];
    try {
      transport_.start(request.at("url"), request.at("api_key"), request.at("body").dump());
      notice_.clear();
      follow_bottom_ = true;
    } catch (const std::exception &) {
      mutate({{"op", "finish"}, {"error", "Could not start the network request."}});
    }
  }
  void send() {
    if (draft_.empty() || busy())
      return;
    const auto text = draft_;
    start({{"op", "send"}, {"content", text}});
    if (streaming())
      draft_.clear();
  }
  const Json &active_chat() const {
    for (const auto &chat : state_["conversations"]) {
      if (chat["id"] == state_["active_id"])
        return chat;
    }
    static const Json empty_chat = {{"title", "New conversation"}, {"messages", Json::array()}};
    return empty_chat;
  }
  void add_button(std::string id, SDL_Rect area, std::function<void()> action) {
    if (focused_.empty())
      focused_ = id;
    if (focused_ == id && !editor_.open) {
      SDL_Rect ring{area.x - 3, area.y - 3, area.w + 6, area.h + 6};
      canvas_.outline(ring, accent);
    }
    buttons_.push_back({std::move(id), area, std::move(action)});
  }
  void button(const std::string &id, const std::string &title, SDL_Rect area,
              std::function<void()> action, bool primary = false) {
    canvas_.rect(area, primary ? accent : elevated, 9);
    canvas_.text(title, area.x + 16, area.y + (area.h - 26) / 2, 20, primary ? dark_ink : ink);
    add_button(id, area, std::move(action));
  }
  void sidebar() {
    canvas_.rect({0, 0, 272, 674}, panel);
    canvas_.rect({30, 28, 36, 36}, accent, 11);
    canvas_.text("w", 38, 26, 30, dark_ink);
    canvas_.text("wodex", 80, 28, 28, ink);
    canvas_.text("A small screen. Big ideas.", 30, 80, 16, muted);
    button(
        "new", "+  New conversation", {28, 130, 216, 48},
        [this] {
          if (busy())
            return;
          if (!mutate({{"op", "new_chat"}}).value("ok", false))
            return;
          settings_ = false;
          updates_ = false;
          scroll_ = 0;
          follow_bottom_ = true;
          draft_.clear();
        },
        true);
    canvas_.text("YOUR CONVERSATIONS", 30, 214, 14, muted);
    const auto &chats = state_["conversations"];
    int active_index = 0;
    for (size_t index = 0; index < chats.size(); ++index) {
      if (chats[index]["id"] == state_["active_id"])
        active_index = static_cast<int>(index);
    }
    const int first = std::max(0, active_index - 5);
    for (int index = first; index < std::min(static_cast<int>(chats.size()), first + 6); ++index) {
      const auto &chat = chats[index];
      const std::string id = chat["id"];
      const int y = 246 + (index - first) * 49;
      if (!settings_ && chat["id"] == state_["active_id"]) {
        canvas_.rect({24, y, 224, 42}, elevated, 8);
        canvas_.rect({24, y + 10, 3, 22}, accent, 1);
      }
      canvas_.clip({36, y, 198, 42});
      canvas_.text(preview(chat.value("title", "New conversation"), 36), 40, y + 9, 18, ink);
      canvas_.unclip();
      add_button("chat-" + id, {28, y, 216, 42}, [this, id] {
        if (busy())
          return;
        mutate({{"op", "select_chat"}, {"id", id}});
        settings_ = false;
        updates_ = false;
        scroll_ = 0;
        follow_bottom_ = true;
        draft_.clear();
      });
    }
    button("settings", "Settings", {28, 586, 216, 46}, [this] { toggle_settings(); });
    canvas_.text(std::string(demo_ ? "OFFLINE DEMO  /  " : "LOCAL HISTORY  /  ") + WODEX_VERSION,
                 30, 647, 13, muted);
  }
  void render_chat() {
    const auto &chat = active_chat();
    const auto &settings = state_["settings"];
    canvas_.text("CHAT / " + preview(chat.value("title", "New conversation"), 48), 312, 30, 16,
                 muted);
    const std::string model = settings.value("model", "deepseek-flash");
    canvas_.rect({934, 23, 304, 36}, elevated, 18);
    canvas_.rect({950, 36, 8, 8}, demo_ ? muted : accent, 4);
    canvas_.text(preview(model, 22) + (settings.value("thinking", false) ? "  ·  THINK" : ""), 968,
                 31, 15, ink);
    add_button("model", {934, 23, 304, 36}, [this] {
      settings_ = true;
      focused_ = "model-setting";
    });
    canvas_.rect({312, 80, 926, 1}, border);
    const auto &messages = chat["messages"];
    if (messages.empty()) {
      canvas_.text("LET CURIOSITY LEAD", 344, 132, 14, accent);
      canvas_.text("What’s on your mind?", 340, 173, 42, ink);
      canvas_.text("Explore an idea, untangle a problem, or start something new.", 344, 242, 20,
                   muted);
      const std::vector<std::pair<std::string, std::string>> prompts = {
          {"01   Understand something", "Explain a topic to me in a simple way: "},
          {"02   Build an idea", "Help me turn this idea into a practical project: "},
          {"03   Find the words", "Help me write a clear message about "},
          {"04   Make a plan", "Help me make a realistic step-by-step plan for "}};
      for (size_t index = 0; index < prompts.size(); ++index) {
        const int x = 344 + static_cast<int>(index % 2) * 430;
        const int y = 311 + static_cast<int>(index / 2) * 90;
        const auto prompt = prompts[index];
        button("prompt-" + std::to_string(index), prompt.first, {x, y, 406, 68}, [this, prompt] {
          edit("Write a message", prompt.second, false, [this](std::string value) {
            draft_ = std::move(value);
            focused_ = "send";
          });
        });
      }
    } else {
      render_messages(messages);
    }
    const auto core_error = state_.value("error", Json());
    const std::string error = core_error.is_string() ? core_error.get<std::string>() : "";
    const std::string status = !notice_.empty() ? notice_ : error;
    if (!status.empty()) {
      canvas_.clip({312, 514, 926, 48});
      canvas_.text(preview(status, 240), 320, 518, 16, danger, 900);
      canvas_.unclip();
    } else if (streaming()) {
      canvas_.text("●  DeepSeek is responding…  B to stop", 320, 525, 16, accent);
    } else {
      canvas_.text(settings.value("thinking", false) ? "Thinking on · Y toggles reasoning detail"
                                                     : "A little curiosity goes a long way.",
                   320, 525, 15, muted);
    }
    canvas_.rect({312, 566, 926, 80}, elevated, 12);
    canvas_.clip({330, 582, 722, 48});
    canvas_.text(draft_.empty() ? "Ask anything…" : preview(draft_, 150), 334, 588, 22,
                 draft_.empty() ? muted : ink, 708);
    canvas_.unclip();
    add_button("compose", {312, 566, 742, 80}, [this] { compose(); });
    button(
        "send", streaming() ? "Stop" : "Send  →", {1066, 580, 156, 52},
        [this] {
          if (streaming())
            stop();
          else
            send();
        },
        true);
  }
  void render_messages(const Json &messages) {
    const int view_height = 412;
    int total = 20;
    for (const auto &message : messages)
      total += message_height(message);
    const int max_scroll = std::max(0, total - view_height);
    scroll_ = follow_bottom_ ? max_scroll : std::min(scroll_, max_scroll);
    canvas_.clip({306, 94, 936, view_height});
    int y = 106 - scroll_;
    for (const auto &message : messages) {
      const int height = message_height(message);
      if (y + height > 94 && y < 506) {
        const bool user = message.value("role", "") == "user";
        const int x = user ? 480 : 322;
        const int width = user ? 738 : 896;
        canvas_.rect({x, y, width, height - 16}, user ? elevated : panel, 10);
        canvas_.text(user ? "YOU" : "DEEPSEEK", x + 20, y + 16, 13, user ? muted : accent);
        int content_y = y + 44;
        const std::string reasoning = message.value("reasoning_content", "");
        if (show_reasoning_ && !reasoning.empty()) {
          canvas_.text("THINKING", x + 20, content_y, 13, muted);
          content_y += 25;
          content_y += canvas_.paragraph(reasoning, x + 20, content_y, 17, muted, width - 40);
          content_y += 12;
        }
        auto content = message.value("content", "");
        if (content.empty())
          content = streaming() ? "Thinking…" : "No answer received.";
        canvas_.paragraph(content, x + 20, content_y, 21, ink, width - 40);
        const auto status = message.value("status", "complete");
        if (status == "interrupted" || status == "error") {
          canvas_.text("Interrupted · Retry from Settings", x + 20, y + height - 44, 14, danger);
        }
      }
      y += height;
    }
    canvas_.unclip();
    if (max_scroll) {
      const int thumb = std::max(24, view_height * view_height / total);
      const int offset = (view_height - thumb) * scroll_ / max_scroll;
      canvas_.rect({1235, 94 + offset, 3, thumb}, muted, 1);
    }
  }
  int message_height(const Json &message) {
    const bool user = message.value("role", "") == "user";
    const int width = user ? 698 : 856;
    auto content = message.value("content", "");
    if (content.empty())
      content = "Thinking…";
    int height = 44 + canvas_.paragraph(content, 0, 0, 21, ink, width, false) + 36;
    const std::string reasoning = message.value("reasoning_content", "");
    if (show_reasoning_ && !reasoning.empty()) {
      height += 37 + canvas_.paragraph(reasoning, 0, 0, 17, muted, width, false);
    }
    const auto status = message.value("status", "complete");
    if (status == "interrupted" || status == "error")
      height += 28;
    return height;
  }
  void render_settings() {
    canvas_.text("MAKE YOURSELF AT HOME", 318, 30, 15, accent);
    canvas_.text("Settings", 312, 68, 36, ink);
    button("updates", "Updates", {1030, 64, 206, 46}, [this] {
      updates_ = true;
      settings_ = false;
      confirm_delete_ = false;
      notice_.clear();
      focused_ = state_["settings"].value("update_repository", "").empty() ? "update-source"
                                                                           : "update-check";
    });
    const auto settings = state_["settings"];
    struct Row {
      std::string key, title, value;
    };
    const std::vector<Row> rows = {
        {"api_key", "DeepSeek API key",
         settings.value("api_key", "").empty()
             ? "Add your key"
             : (environment_key_ ? "••••••••  ·  from environment" : "••••••••  ·  saved locally")},
        {"model", "Model", settings.value("model", "deepseek-flash")},
        {"thinking", "Thinking mode", settings.value("thinking", false) ? "On" : "Off"},
        {"base_url", "API endpoint", settings.value("base_url", "https://api.deepseek.com")},
        {"system_prompt", "Assistant instructions",
         preview(settings.value("system_prompt", ""), 64)},
        {"max_tokens", "Response token limit", std::to_string(settings.value("max_tokens", 4096))}};
    for (size_t index = 0; index < rows.size(); ++index) {
      const auto row = rows[index];
      const int y = 132 + static_cast<int>(index) * 62;
      canvas_.rect({314, y, 922, 52}, elevated, 7);
      canvas_.text(row.title, 332, y + 14, 19, ink);
      canvas_.clip({626, y, 590, 52});
      canvas_.text(row.value.empty() ? "Default" : row.value, 638, y + 15, 17, muted);
      canvas_.unclip();
      add_button(row.key == "model" ? "model-setting" : "setting-" + row.key, {314, y, 922, 52},
                 [this, row, settings] {
                   confirm_delete_ = false;
                   if (busy()) {
                     notice_ = "Stop the current response before changing settings.";
                     return;
                   }
                   if (row.key == "thinking") {
                     mutate({{"op", "settings"},
                             {"settings", {{"thinking", !settings.value("thinking", false)}}}});
                     return;
                   }
                   const auto current = row.key == "max_tokens"
                                            ? std::to_string(settings.value("max_tokens", 4096))
                                            : settings.value(row.key, "");
                   edit(row.title, current, row.key == "api_key", [this, row](std::string value) {
                     Json setting = value;
                     if (row.key == "max_tokens") {
                       try {
                         size_t parsed = 0;
                         const int tokens = std::stoi(value, &parsed);
                         if (parsed != value.size())
                           throw std::invalid_argument("tokens");
                         setting = tokens;
                       } catch (...) {
                         notice_ = "The response limit must be a whole number.";
                         return;
                       }
                     }
                     const auto result =
                         mutate({{"op", "settings"}, {"settings", {{row.key, setting}}}});
                     if (row.key == "api_key" && result.value("ok", false)) {
                       environment_key_ = false;
                       save();
                     }
                   });
                 });
    }
    button("retry", "Retry last response", {314, 526, 272, 46}, [this] {
      settings_ = false;
      start({{"op", "retry"}});
    });
    button("delete", "Delete this chat", {604, 526, 254, 46}, [this] {
      if (busy())
        return;
      if (!confirm_delete_) {
        confirm_delete_ = true;
        notice_ = "Select Delete again to permanently remove this chat.";
        return;
      }
      mutate({{"op", "delete_chat"}, {"id", state_["active_id"]}});
      confirm_delete_ = false;
      settings_ = false;
    });
    button("back", "Back to chat", {876, 526, 360, 46}, [this] { toggle_settings(); }, true);
    const std::string note = notice_.empty() ? "API keys and conversations stay on this device. "
                                               "Messages go to your configured provider."
                                             : notice_;
    canvas_.text(preview(note, 230), 318, 600, 16, notice_.empty() ? muted : danger, 910);
  }

  void render_updates() {
    const auto repository = state_["settings"].value("update_repository", "");
    auto status = updater_.status();
    if (!updater_.active() && repository != checked_repository_) {
      status = {};
      status.message = "Check this release source before downloading an update.";
    }
    const auto update_action = [this](const std::function<void()> &action) {
      if (demo_)
        return;
      try {
        action();
        notice_.clear();
      } catch (const std::exception &error) {
        notice_ = error.what();
      }
    };
    UpdateActions actions;
    actions.change_source = [this, repository] {
      if (updater_.active() || busy() || demo_)
        return;
      edit("GitHub repository (owner/repo)", repository, false, [this](std::string value) {
        const auto result =
            mutate({{"op", "settings"}, {"settings", {{"update_repository", value}}}});
        if (result.value("ok", false)) {
          checked_repository_.clear();
          focused_ = value.empty() ? "update-source" : "update-check";
        }
      });
    };
    actions.check = [this, repository, update_action] {
      update_action([this, repository] {
        updater_.check(repository);
        checked_repository_ = repository;
        focused_ = "update-cancel";
      });
    };
    actions.download = [this, repository, update_action] {
      if (repository != checked_repository_)
        return;
      update_action([this] {
        updater_.download();
        focused_ = "update-cancel";
      });
    };
    actions.cancel = [this] { updater_.cancel(); };
    actions.install = [this, repository] {
      if (demo_ || repository != checked_repository_)
        return;
      if (busy()) {
        notice_ = "Stop the current reply before installing the update.";
        return;
      }
      const auto pending = updater_.pending_install();
      if (!pending || !save())
        return;
      pending_update_ = pending;
      updater_.cancel();
      running_ = false;
    };
    actions.back = [this] {
      updates_ = false;
      settings_ = true;
      notice_.clear();
      focused_ = "updates";
    };
    render_update_page(canvas_, status, repository, WODEX_VERSION, notice_, !update_target_.empty(),
                       demo_, actions,
                       [this](const std::string &id, const std::string &title, SDL_Rect area,
                              std::function<void()> action, bool primary) {
                         button(id, title, area, std::move(action), primary);
                       });
  }
  void edit(const std::string &title, const std::string &initial, bool secret,
            std::function<void(std::string)> done) {
#ifdef __SWITCH__
    try {
      done(platform_.edit_text(title, initial, secret));
    } catch (const std::exception &error) {
      notice_ = error.what();
    }
#else
    editor_ = {true, secret, title, initial, "", initial.size(), false, std::move(done)};
    SDL_StartTextInput();
    SDL_Rect area{300, 280, 680, 160};
    SDL_SetTextInputRect(&area);
#endif
  }
  void compose() {
    if (busy())
      return;
    settings_ = false;
    updates_ = false;
    edit("Write a message", draft_, false, [this](std::string value) {
      draft_ = std::move(value);
      focused_ = "send";
    });
  }
  void render_editor() {
    canvas_.rect({0, 0, 1280, 720}, {0, 0, 0, 175});
    canvas_.rect({232, 158, 816, 398}, panel, 16);
    canvas_.text(editor_.title, 266, 188, 28, ink);
    canvas_.rect({262, 248, 756, 202}, background, 10);
    canvas_.clip({278, 264, 724, 168});
    std::string display;
    if (editor_.secret) {
      display = std::string(std::min<size_t>(editor_.value.size(), 64), '*') + "│";
      if (!editor_.composition.empty())
        display += "***";
    } else {
      display = editor_.value.substr(0, editor_.cursor) + "│" + editor_.composition +
                editor_.value.substr(editor_.cursor);
    }
    canvas_.paragraph(preview(display, 12000), 280, 268, 22, editor_.select_all ? accent : ink,
                      716);
    canvas_.unclip();
    canvas_.text("Enter  Done     Shift + Enter  New line     Esc  Cancel", 266, 476, 17, muted);
    canvas_.text("Ctrl / Cmd + A  Select all     Ctrl / Cmd + V  Paste     Ctrl + U  Clear", 266,
                 514, 14, muted);
  }
  void insert_text(const std::string &value) {
    if (editor_.select_all) {
      editor_.value.clear();
      editor_.cursor = 0;
      editor_.select_all = false;
    }
    if (editor_.value.size() + value.size() <= 16384) {
      editor_.value.insert(editor_.cursor, value);
      editor_.cursor += value.size();
    }
  }
  void handle_editor(const SDL_Event &event) {
    if (event.type == SDL_TEXTINPUT) {
      insert_text(event.text.text);
      editor_.composition.clear();
    } else if (event.type == SDL_TEXTEDITING) {
      editor_.composition = event.edit.text;
    } else if (event.type == SDL_KEYDOWN) {
      const auto key = event.key.keysym.sym;
      const auto mod = event.key.keysym.mod;
      if (key == SDLK_ESCAPE) {
        editor_.open = false;
        editor_.value.clear();
        SDL_StopTextInput();
      } else if ((key == SDLK_RETURN || key == SDLK_KP_ENTER) && editor_.composition.empty()) {
        if (mod & KMOD_SHIFT) {
          if (!editor_.secret)
            insert_text("\n");
        } else {
          auto done = std::move(editor_.done);
          auto value = std::move(editor_.value);
          editor_.open = false;
          SDL_StopTextInput();
          done(std::move(value));
        }
      } else if (key == SDLK_BACKSPACE) {
        if (editor_.select_all) {
          editor_.value.clear();
          editor_.cursor = 0;
          editor_.select_all = false;
        } else if (editor_.cursor) {
          auto prefix = editor_.value.substr(0, editor_.cursor);
          erase_last_character(prefix);
          editor_.value.erase(prefix.size(), editor_.cursor - prefix.size());
          editor_.cursor = prefix.size();
        }
      } else if (key == SDLK_DELETE) {
        if (editor_.select_all) {
          editor_.value.clear();
          editor_.cursor = 0;
          editor_.select_all = false;
        } else if (editor_.cursor < editor_.value.size()) {
          size_t end = editor_.cursor + 1;
          while (end < editor_.value.size() &&
                 (static_cast<unsigned char>(editor_.value[end]) & 0xc0) == 0x80)
            ++end;
          editor_.value.erase(editor_.cursor, end - editor_.cursor);
        }
      } else if (key == SDLK_LEFT) {
        if (editor_.cursor) {
          auto prefix = editor_.value.substr(0, editor_.cursor);
          erase_last_character(prefix);
          editor_.cursor = prefix.size();
        }
        editor_.select_all = false;
      } else if (key == SDLK_RIGHT) {
        if (editor_.cursor < editor_.value.size()) {
          ++editor_.cursor;
          while (editor_.cursor < editor_.value.size() &&
                 (static_cast<unsigned char>(editor_.value[editor_.cursor]) & 0xc0) == 0x80)
            ++editor_.cursor;
        }
        editor_.select_all = false;
      } else if (key == SDLK_HOME || key == SDLK_END) {
        editor_.cursor = key == SDLK_HOME ? 0 : editor_.value.size();
        editor_.select_all = false;
      } else if (key == SDLK_a && (mod & (KMOD_CTRL | KMOD_GUI))) {
        editor_.select_all = true;
      } else if (key == SDLK_v && (mod & (KMOD_CTRL | KMOD_GUI))) {
        char *clipboard = SDL_GetClipboardText();
        if (clipboard) {
          insert_text(clipboard);
          SDL_free(clipboard);
        }
      } else if (key == SDLK_u && (mod & KMOD_CTRL)) {
        editor_.value.clear();
        editor_.cursor = 0;
        editor_.select_all = false;
      }
    }
  }
  void click(int x, int y) {
    const SDL_Point point{x, y};
    for (const auto &button : buttons_) {
      if (SDL_PointInRect(&point, &button.area)) {
        focused_ = button.id;
        if (button.id != "delete")
          confirm_delete_ = false;
        auto action = button.action;
        action();
        break;
      }
    }
  }
  void move_focus(int dx, int dy) {
    if (buttons_.empty())
      return;
    auto current = std::find_if(buttons_.begin(), buttons_.end(),
                                [this](const Button &button) { return button.id == focused_; });
    if (current == buttons_.end()) {
      focused_ = buttons_.front().id;
      return;
    }
    const int x = current->area.x + current->area.w / 2;
    const int y = current->area.y + current->area.h / 2;
    double best = 1e12;
    std::string candidate = focused_;
    for (const auto &button : buttons_) {
      const int tx = button.area.x + button.area.w / 2 - x;
      const int ty = button.area.y + button.area.h / 2 - y;
      const int forward = dx ? tx * dx : ty * dy;
      if (forward <= 0)
        continue;
      const int sideways = dx ? std::abs(ty) : std::abs(tx);
      const double score =
          forward + sideways * 3.0 + sideways * sideways / static_cast<double>(forward);
      if (score < best) {
        best = score;
        candidate = button.id;
      }
    }
    focused_ = candidate;
    if (focused_ != "delete")
      confirm_delete_ = false;
  }
  void activate() {
    for (const auto &button : buttons_) {
      if (button.id == focused_) {
        auto action = button.action;
        action();
        break;
      }
    }
  }
  void keyboard(SDL_Keycode key, Uint16 mod) {
    if ((mod & (KMOD_CTRL | KMOD_GUI)) && key == SDLK_RETURN) {
      send();
      return;
    }
    if (key == SDLK_LEFT)
      move_focus(-1, 0);
    else if (key == SDLK_RIGHT)
      move_focus(1, 0);
    else if (key == SDLK_UP)
      move_focus(0, -1);
    else if (key == SDLK_DOWN)
      move_focus(0, 1);
    else if (key == SDLK_RETURN || key == SDLK_SPACE)
      activate();
    else if (key == SDLK_x)
      compose();
    else if (key == SDLK_y)
      show_reasoning_ = !show_reasoning_;
    else if (key == SDLK_ESCAPE)
      back();
    else if (key == SDLK_F1 || key == SDLK_MINUS)
      toggle_settings();
    else if (key == SDLK_PAGEUP) {
      scroll_ = std::max(0, scroll_ - 280);
      follow_bottom_ = false;
    } else if (key == SDLK_PAGEDOWN) {
      scroll_ += 280;
      follow_bottom_ = false;
    } else if (key == SDLK_END)
      follow_bottom_ = true;
    else if (key == SDLK_LEFTBRACKET)
      cycle_chat(-1);
    else if (key == SDLK_RIGHTBRACKET)
      cycle_chat(1);
  }
  void gamepad(Uint8 key) {
#ifndef __SWITCH__
    if (key == SDL_CONTROLLER_BUTTON_A)
      activate();
    else if (key == SDL_CONTROLLER_BUTTON_B)
      back();
    else if (key == SDL_CONTROLLER_BUTTON_X)
      compose();
    else if (key == SDL_CONTROLLER_BUTTON_Y)
      show_reasoning_ = !show_reasoning_;
    else if (key == SDL_CONTROLLER_BUTTON_DPAD_LEFT)
      move_focus(-1, 0);
    else if (key == SDL_CONTROLLER_BUTTON_DPAD_RIGHT)
      move_focus(1, 0);
    else if (key == SDL_CONTROLLER_BUTTON_DPAD_UP)
      move_focus(0, -1);
    else if (key == SDL_CONTROLLER_BUTTON_DPAD_DOWN)
      move_focus(0, 1);
    else if (key == SDL_CONTROLLER_BUTTON_LEFTSHOULDER)
      cycle_chat(-1);
    else if (key == SDL_CONTROLLER_BUTTON_RIGHTSHOULDER)
      cycle_chat(1);
    else if (key == SDL_CONTROLLER_BUTTON_BACK)
      toggle_settings();
    else if (key == SDL_CONTROLLER_BUTTON_START)
      close();
#else
    (void)key;
#endif
  }
  void open_controller(int index) {
#ifndef __SWITCH__
    if (SDL_IsGameController(index)) {
      if (auto *controller = SDL_GameControllerOpen(index))
        controllers_.push_back(controller);
    }
#else
    (void)index;
#endif
  }
  void cycle_chat(int direction) {
    if (busy())
      return;
    const auto &chats = state_["conversations"];
    if (chats.empty())
      return;
    for (size_t index = 0; index < chats.size(); ++index) {
      if (chats[index]["id"] == state_["active_id"]) {
        const int next = (static_cast<int>(index) + direction + static_cast<int>(chats.size())) %
                         static_cast<int>(chats.size());
        mutate({{"op", "select_chat"}, {"id", chats[next]["id"]}});
        settings_ = false;
        updates_ = false;
        draft_.clear();
        follow_bottom_ = true;
        break;
      }
    }
  }
  void toggle_settings() {
    updates_ = false;
    settings_ = !settings_;
    confirm_delete_ = false;
    focused_ = settings_ ? "setting-api_key" : "compose";
  }
  void stop() {
    transport_.cancel();
    if (streaming())
      mutate({{"op", "cancel"}});
  }
  void back() {
    if (updates_ && updater_.active())
      updater_.cancel();
    else if (updates_) {
      updates_ = false;
      settings_ = true;
      focused_ = "updates";
    } else if (streaming())
      stop();
    else if (settings_)
      toggle_settings();
  }
  void close() {
    updater_.cancel();
    stop();
    save();
    running_ = false;
  }
  void seed_demo() {
    core_.call(
        {{"op", "settings"}, {"settings", {{"api_key", "offline-demo"}, {"thinking", true}}}});
    core_.call({{"op", "send"}, {"content", "How do I turn a small idea into something real?"}});
    core_.feed("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"A useful starting point is "
               "a small, testable first step.\"}}]}\n\n");
    core_.feed("data: {\"choices\":[{\"delta\":{\"content\":\"Start with one thing you wish "
               "existed.\\n\\n1. Write down who it helps and what it makes easier.\\n2. Build the "
               "smallest version that does that one thing.\\n3. Try it in the real world, then let "
               "what you learn guide the next step.\\n\\nA little progress, made tangible, is a "
               "great place to begin.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    core_.call({{"op", "finish"}, {"http_status", 200}});
    refresh();
    notice_ = "Offline demo · sample conversation · no network requests";
    focused_ = "compose";
  }

  Platform &platform_;
  SDL_Renderer *renderer_;
  Canvas canvas_;
  std::string save_path_;
  Core core_;
  Transport transport_;
  std::string update_target_;
  Updater updater_;
  std::optional<PendingUpdate> pending_update_;
  std::string checked_repository_;
  Json state_;
  bool demo_ = false;
  bool running_ = true;
  bool settings_ = false;
  bool updates_ = false;
  bool show_reasoning_ = false;
  bool follow_bottom_ = true;
  bool confirm_delete_ = false;
  bool environment_key_ = false;
  std::string stored_key_;
  std::string draft_;
  std::string notice_;
  std::string focused_ = "compose";
  int scroll_ = 0;
  Uint64 last_save_ = 0;
  std::vector<Button> buttons_;
  std::vector<SDL_GameController *> controllers_;
  Editor editor_;
#ifdef __SWITCH__
  PadState pad_{};
#endif
};

std::string update_install_path(int argc, char **argv) {
#ifdef __SWITCH__
  if (argc < 1 || !argv || !argv[0])
    return {};
  const std::string path = argv[0];
  if (path.size() < 11 || path.size() > 768 || path.compare(0, 6, "sdmc:/") != 0 ||
      path.compare(path.size() - 4, 4, ".nro") != 0 || path.find('\\') != std::string::npos ||
      path.find("//") != std::string::npos || path.find("/../") != std::string::npos ||
      path.find("/./") != std::string::npos ||
      std::any_of(path.begin(), path.end(),
                  [](unsigned char value) { return value < 0x20 || value == 0x7f; }))
    return {};
  const std::string backup_suffix = ".previous.nro";
  if (path.size() >= backup_suffix.size() &&
      path.compare(path.size() - backup_suffix.size(), backup_suffix.size(), backup_suffix) == 0)
    return {};
  return path;
#else
  static_cast<void>(argc);
  static_cast<void>(argv);
  return {};
#endif
}

} // namespace

int main(int argc, char **argv) {
  bool demo = false;
  bool show_updates = false;
  std::string screenshot_path;
  for (int index = 1; index < argc; ++index) {
    const std::string argument = argv[index];
    if (argument == "--demo")
      demo = true;
    else if (argument == "--updates")
      show_updates = true;
    else if (argument == "--screenshot" && index + 1 < argc)
      screenshot_path = argv[++index];
    else if (argument == "--help") {
      std::cout << "Wodex · a native DeepSeek client for Nintendo Switch\n"
                   "  --demo                  Sample chat, no network or saves\n"
                   "  --updates               Open update settings\n"
                   "  --screenshot FILE.bmp   Render once and save a 1280×720 image\n"
                   "Environment: DEEPSEEK_API_KEY, WODEX_DATA_DIR, WODEX_FONT, WODEX_CA_BUNDLE\n";
      return 0;
    } else {
      std::cerr << "Unknown argument. Use --help.\n";
      return 1;
    }
  }
  try {
    std::optional<PendingUpdate> pending_update;
    {
      std::string resource_dir;
#ifndef __SWITCH__
      std::unique_ptr<char, decltype(&SDL_free)> base_path(SDL_GetBasePath(), SDL_free);
      if (!base_path)
        throw std::runtime_error("Cannot locate the application resource directory.");
      resource_dir = std::string(base_path.get()) + "resources";
#endif
      Platform platform(resource_dir);
      if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMECONTROLLER) != 0)
        throw std::runtime_error(SDL_GetError());
      if (TTF_Init() != 0)
        throw std::runtime_error(TTF_GetError());
      SDL_SetHint(SDL_HINT_RENDER_SCALE_QUALITY, "1");
      SDL_SetHint(SDL_HINT_TOUCH_MOUSE_EVENTS, "0");
      SDL_SetHint(SDL_HINT_MOUSE_TOUCH_EVENTS, "0");
      std::unique_ptr<SDL_Window, decltype(&SDL_DestroyWindow)> window(
          SDL_CreateWindow("Wodex · DeepSeek for Switch", SDL_WINDOWPOS_CENTERED,
                           SDL_WINDOWPOS_CENTERED, 1280, 720,
                           SDL_WINDOW_RESIZABLE | SDL_WINDOW_ALLOW_HIGHDPI),
          SDL_DestroyWindow);
      if (!window)
        throw std::runtime_error(SDL_GetError());
      auto *renderer_ptr = SDL_CreateRenderer(window.get(), -1,
                                              SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC);
      if (!renderer_ptr)
        renderer_ptr = SDL_CreateRenderer(window.get(), -1, SDL_RENDERER_SOFTWARE);
      std::unique_ptr<SDL_Renderer, decltype(&SDL_DestroyRenderer)> renderer(renderer_ptr,
                                                                             SDL_DestroyRenderer);
      if (!renderer)
        throw std::runtime_error(SDL_GetError());
      SDL_RenderSetLogicalSize(renderer.get(), 1280, 720);
      SDL_SetRenderDrawBlendMode(renderer.get(), SDL_BLENDMODE_BLEND);
      {
        App app(platform, renderer.get(), demo, update_install_path(argc, argv), show_updates);
        do {
          app.tick();
          app.render();
          if (!screenshot_path.empty()) {
            app.screenshot(screenshot_path);
            break;
          }
          SDL_RenderPresent(renderer.get());
          SDL_Delay(8);
        } while (app.running());
        pending_update = app.pending_install();
      }
      renderer.reset();
      window.reset();
      TTF_Quit();
      SDL_Quit();
    }
    // Platform destruction above unmounts RomFS after App closes every font and
    // joins its workers. The executable must not be renamed while assets use it.
    if (pending_update)
      install_update(*pending_update);
    return 0;
  } catch (const std::exception &error) {
    std::cerr << "Wodex: " << error.what() << '\n';
#ifdef __SWITCH__
    consoleInit(nullptr);
    std::cout << "Wodex could not start.\n" << error.what() << "\nPress + to exit.\n";
    PadState pad;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    while (appletMainLoop()) {
      padUpdate(&pad);
      if (padGetButtonsDown(&pad) & HidNpadButton_Plus)
        break;
      consoleUpdate(nullptr);
    }
    consoleExit(nullptr);
#endif
    return 1;
  }
}
