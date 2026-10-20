#pragma once

#include "ui.hpp"
#include "updater.hpp"
#include <functional>
#include <string>

namespace wodex {

struct UpdateActions {
  std::function<void()> change_source;
  std::function<void()> check;
  std::function<void()> download;
  std::function<void()> install;
  std::function<void()> cancel;
  std::function<void()> back;
};

using UpdateButton = std::function<void(const std::string &, const std::string &, SDL_Rect,
                                        std::function<void()>, bool)>;

void render_update_page(Canvas &canvas, const UpdateStatus &status, const std::string &repository,
                        const std::string &current_version, const std::string &notice,
                        bool install_supported, bool demo, const UpdateActions &actions,
                        const UpdateButton &draw_button);

} // namespace wodex
