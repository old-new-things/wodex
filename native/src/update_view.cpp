#include "update_view.hpp"
#include <algorithm>
#include <iomanip>
#include <sstream>

namespace wodex {
namespace {

std::string size_label(uint64_t bytes) {
  std::ostringstream result;
  result << std::fixed << std::setprecision(1) << bytes / (1024.0 * 1024.0) << " MiB";
  return result.str();
}

std::string phase_title(UpdatePhase phase) {
  switch (phase) {
  case UpdatePhase::Checking:
    return "Checking for updates…";
  case UpdatePhase::Current:
    return "You’re up to date";
  case UpdatePhase::Available:
    return "A new version is available";
  case UpdatePhase::Downloading:
    return "Downloading your update…";
  case UpdatePhase::Ready:
    return "Your update is ready to install";
  case UpdatePhase::Cancelled:
    return "Update cancelled";
  case UpdatePhase::Error:
    return "Update could not be completed";
  case UpdatePhase::Idle:
    return "Keep Wodex up to date";
  }
  return "Updates";
}

} // namespace

void render_update_page(Canvas &canvas, const UpdateStatus &status, const std::string &repository,
                        const std::string &current_version, const std::string &notice,
                        bool install_supported, bool demo, const UpdateActions &actions,
                        const UpdateButton &draw_button) {
  canvas.text("WODEX " + current_version, 318, 30, 15, accent);
  canvas.text("Updates", 312, 68, 36, ink);
  draw_button("update-back", "Back to Settings", {994, 64, 242, 46}, actions.back, false);
  canvas.text("GITHUB RELEASE SOURCE", 318, 128, 14, muted);
  canvas.rect({314, 158, 922, 62}, elevated, 9);
  canvas.clip({332, 172, 690, 34});
  canvas.text(repository.empty() ? "Not configured" : repository, 334, 173, 21,
              repository.empty() ? muted : ink);
  canvas.unclip();
  const bool active =
      status.phase == UpdatePhase::Checking || status.phase == UpdatePhase::Downloading;
  if (!active && !demo) {
    draw_button("update-source", "Change", {1074, 168, 146, 42}, actions.change_source, false);
  }
  canvas.rect({314, 244, 922, 244}, panel, 12);
  canvas.text(phase_title(status.phase), 338, 266, 26,
              status.phase == UpdatePhase::Error ? danger : ink);
  if (!status.version.empty()) {
    canvas.text("Version " + status.version, 340, 312, 18, accent);
  }
  std::string message = status.message;
  if (message.empty()) {
    message = repository.empty() ? "Set a public GitHub repository to publish and receive releases."
                                 : "Check for a new version before downloading.";
  }
  canvas.clip({338, 344, 870, 100});
  canvas.text(preview(message, 600), 340, 344, 18, muted, 864);
  canvas.unclip();
  if (status.phase == UpdatePhase::Downloading || status.phase == UpdatePhase::Ready) {
    canvas.rect({340, 438, 864, 7}, border, 3);
    const double progress =
        status.total ? std::min(1.0, status.downloaded / static_cast<double>(status.total)) : 0.0;
    canvas.rect({340, 438, static_cast<int>(864 * progress), 7}, accent, 3);
    canvas.text(size_label(status.downloaded) + " / " + size_label(status.total), 340, 455, 15,
                muted);
  }

  const auto disabled_button = [&canvas](const std::string &title, SDL_Rect area) {
    canvas.rect(area, panel, 9);
    canvas.text(title, area.x + 16, area.y + 12, 20, muted);
  };
  if (active) {
    draw_button("update-cancel", "Cancel", {314, 512, 210, 50}, actions.cancel, false);
  } else if (!repository.empty() && !demo) {
    draw_button("update-check", "Check for updates", {314, 512, 240, 50}, actions.check, false);
  } else {
    disabled_button("Check for updates", {314, 512, 240, 50});
  }
  if (status.phase == UpdatePhase::Available && install_supported && !demo) {
    draw_button("update-download", "Download", {572, 512, 224, 50}, actions.download, true);
  } else if (status.phase == UpdatePhase::Ready && install_supported && !demo) {
    draw_button("update-install", "Install and exit", {816, 512, 420, 50}, actions.install, true);
  }
  const std::string hint =
      !notice.empty() ? notice
      : demo          ? "Offline demo: update checks are disabled."
      : !install_supported
          ? "Self-install is available when Wodex is launched as an NRO on your Switch."
          : "Installing closes Wodex. Your conversations and the previous app version are kept.";
  canvas.text(preview(hint, 240), 318, 592, 16, notice.empty() ? muted : danger, 910);
}

} // namespace wodex
