use super::canvas::{
  preview, Canvas, Color, Rect, ACCENT, BACKGROUND, BORDER, DANGER, DARK_INK, ELEVATED, INK, MUTED,
  PANEL,
};
use super::host::{UpdatePhase, UpdateStatus};
use crate::state::{Message, MessageStatus, Role, State};
use alloc::{format, string::String, vec::Vec};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Page {
  #[default]
  Chat,
  Settings,
  Updates,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Setting {
  ApiKey,
  Model,
  Thinking,
  BaseUrl,
  SystemPrompt,
  MaxTokens,
  UpdateRepository,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
  NewChat,
  SelectChat(String),
  Settings,
  OpenModel,
  Prompt(usize),
  Compose,
  Send,
  Stop,
  EditSetting(Setting),
  Updates,
  Retry,
  DeleteChat,
  BackToChat,
  UpdateCheck,
  UpdateDownload,
  UpdateInstall,
  UpdateCancel,
  UpdateBack,
}

#[derive(Clone, Debug)]
pub struct Button {
  pub id: String,
  pub area: Rect,
  pub action: Action,
}

pub struct EditorView<'a> {
  pub title: &'a str,
  pub value: &'a str,
  pub composition: &'a str,
  pub cursor: usize,
  pub secret: bool,
  pub select_all: bool,
}

pub struct ViewState {
  pub page: Page,
  pub focused: String,
  pub draft: String,
  pub notice: String,
  pub show_reasoning: bool,
  pub follow_bottom: bool,
  pub scroll: i32,
  pub environment_key: bool,
  pub confirm_delete: bool,
}

impl Default for ViewState {
  fn default() -> Self {
    Self {
      page: Page::Chat,
      focused: String::from("compose"),
      draft: String::new(),
      notice: String::new(),
      show_reasoning: false,
      follow_bottom: true,
      scroll: 0,
      environment_key: false,
      confirm_delete: false,
    }
  }
}

pub const PROMPTS: [(&str, &str); 4] = [
  ("01   Understand something", "Explain a topic to me in a simple way: "),
  ("02   Build an idea", "Help me turn this idea into a practical project: "),
  ("03   Find the words", "Help me write a clear message about "),
  ("04   Make a plan", "Help me make a realistic step-by-step plan for "),
];

/// Draw the application from borrowed domain state and return its interactive regions.
/// No chat history is serialized or cloned to build the view.
#[allow(clippy::too_many_arguments)]
pub fn render(
  canvas: &mut Canvas,
  state: &State,
  ui: &mut ViewState,
  update: &UpdateStatus,
  version: &str,
  demo: bool,
  install_supported: bool,
  editor: Option<EditorView<'_>>,
) -> Vec<Button> {
  let mut view = View { canvas, state, ui, buttons: Vec::new(), editor_open: editor.is_some() };
  view.canvas.rect(rect(0, 0, 1280, 720), BACKGROUND, 0);
  view.sidebar(version, demo);
  match view.ui.page {
    Page::Chat => view.chat(demo),
    Page::Settings => view.settings(),
    Page::Updates => view.updates(update, version, demo, install_supported),
  }
  view.canvas.rect(rect(0, 674, 1280, 46), BACKGROUND, 0);
  view.canvas.rect(rect(24, 674, 1232, 1), BORDER, 0);
  view.canvas.text(
    "A  Select     X  Write     B  Back / stop     −  Settings",
    28,
    687,
    16,
    MUTED,
    0,
  );
  view.canvas.text("L / R  Chats     ZL / ZR  Scroll     +  Exit", 890, 687, 16, MUTED, 0);
  if let Some(editor) = editor {
    view.editor(editor);
  }
  view.buttons
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
  Rect { x, y, w, h }
}

struct View<'a> {
  canvas: &'a mut Canvas,
  state: &'a State,
  ui: &'a mut ViewState,
  buttons: Vec<Button>,
  editor_open: bool,
}

impl View<'_> {
  fn add_button(&mut self, id: &str, area: Rect, action: Action) {
    if self.ui.focused.is_empty() {
      self.ui.focused = id.into();
    }
    if self.ui.focused == id && !self.editor_open {
      self.canvas.outline(rect(area.x - 3, area.y - 3, area.w + 6, area.h + 6), ACCENT);
    }
    self.buttons.push(Button { id: id.into(), area, action });
  }

  fn button(&mut self, id: &str, title: &str, area: Rect, action: Action, primary: bool) {
    self.canvas.rect(area, if primary { ACCENT } else { ELEVATED }, 9);
    self.canvas.text(
      title,
      area.x + 16,
      area.y + (area.h - 26) / 2,
      20,
      if primary { DARK_INK } else { INK },
      0,
    );
    self.add_button(id, area, action);
  }

  fn sidebar(&mut self, version: &str, demo: bool) {
    self.canvas.rect(rect(0, 0, 272, 674), PANEL, 0);
    self.canvas.rect(rect(30, 28, 36, 36), ACCENT, 11);
    self.canvas.text("w", 38, 26, 30, DARK_INK, 0);
    self.canvas.text("wodex", 80, 28, 28, INK, 0);
    self.canvas.text("A small screen. Big ideas.", 30, 80, 16, MUTED, 0);
    self.button("new", "+  New conversation", rect(28, 130, 216, 48), Action::NewChat, true);
    self.canvas.text("YOUR CONVERSATIONS", 30, 214, 14, MUTED, 0);
    let state = self.state;
    let chats = &state.conversations;
    let active_index = chats.iter().position(|chat| chat.id == self.state.active_id).unwrap_or(0);
    let first = active_index.saturating_sub(5);
    for (index, chat) in chats.iter().enumerate().skip(first).take(6) {
      let y = 246 + (index - first) as i32 * 49;
      if self.ui.page == Page::Chat && chat.id == self.state.active_id {
        self.canvas.rect(rect(24, y, 224, 42), ELEVATED, 8);
        self.canvas.rect(rect(24, y + 10, 3, 22), ACCENT, 1);
      }
      self.canvas.clip(rect(36, y, 198, 42));
      self.canvas.text(&preview(&chat.title, 36), 40, y + 9, 18, INK, 0);
      self.canvas.unclip();
      self.add_button(
        &format!("chat-{}", chat.id),
        rect(28, y, 216, 42),
        Action::SelectChat(chat.id.clone()),
      );
    }
    self.button("settings", "Settings", rect(28, 586, 216, 46), Action::Settings, false);
    let mode = if demo { "OFFLINE DEMO" } else { "LOCAL HISTORY" };
    self.canvas.text(&format!("{mode}  /  {version}"), 30, 647, 13, MUTED, 0);
  }

  fn chat(&mut self, demo: bool) {
    let state = self.state;
    let chat = state.conversations.iter().find(|chat| chat.id == state.active_id);
    let title = chat.map_or("New conversation", |chat| chat.title.as_str());
    self.canvas.text(&format!("CHAT / {}", preview(title, 48)), 312, 30, 16, MUTED, 0);
    let settings = &state.settings;
    self.canvas.rect(rect(934, 23, 304, 36), ELEVATED, 18);
    self.canvas.rect(rect(950, 36, 8, 8), if demo { MUTED } else { ACCENT }, 4);
    let thinking = if settings.thinking { "  ·  THINK" } else { "" };
    self.canvas.text(&format!("{}{thinking}", preview(&settings.model, 22)), 968, 31, 15, INK, 0);
    self.add_button("model", rect(934, 23, 304, 36), Action::OpenModel);
    self.canvas.rect(rect(312, 80, 926, 1), BORDER, 0);
    let messages = state
      .conversations
      .iter()
      .find(|chat| chat.id == self.state.active_id)
      .map_or(&[][..], |chat| chat.messages.as_slice());
    if messages.is_empty() {
      self.canvas.text("LET CURIOSITY LEAD", 344, 132, 14, ACCENT, 0);
      self.canvas.text("What’s on your mind?", 340, 173, 42, INK, 0);
      self.canvas.text(
        "Explore an idea, untangle a problem, or start something new.",
        344,
        242,
        20,
        MUTED,
        0,
      );
      for (index, (title, _)) in PROMPTS.iter().enumerate() {
        let x = 344 + (index % 2) as i32 * 430;
        let y = 311 + (index / 2) as i32 * 90;
        self.button(
          &format!("prompt-{index}"),
          title,
          rect(x, y, 406, 68),
          Action::Prompt(index),
          false,
        );
      }
    } else {
      self.messages(messages);
    }
    let status = if self.ui.notice.is_empty() {
      self.state.error.as_deref().unwrap_or("")
    } else {
      &self.ui.notice
    };
    if !status.is_empty() {
      self.canvas.clip(rect(312, 514, 926, 48));
      self.canvas.text(&preview(status, 240), 320, 518, 16, DANGER, 900);
      self.canvas.unclip();
    } else if self.state.is_streaming {
      self.canvas.text("●  DeepSeek is responding…  B to stop", 320, 525, 16, ACCENT, 0);
    } else {
      let hint = if self.state.settings.thinking {
        "Thinking on · Y toggles reasoning detail"
      } else {
        "A little curiosity goes a long way."
      };
      self.canvas.text(hint, 320, 525, 15, MUTED, 0);
    }
    self.canvas.rect(rect(312, 566, 926, 80), ELEVATED, 12);
    self.canvas.clip(rect(330, 582, 722, 48));
    if self.ui.draft.is_empty() {
      self.canvas.text("Ask anything…", 334, 588, 22, MUTED, 708);
    } else {
      self.canvas.text(&preview(&self.ui.draft, 150), 334, 588, 22, INK, 708);
    }
    self.canvas.unclip();
    self.add_button("compose", rect(312, 566, 742, 80), Action::Compose);
    let (title, action) =
      if self.state.is_streaming { ("Stop", Action::Stop) } else { ("Send  →", Action::Send) };
    self.button("send", title, rect(1066, 580, 156, 52), action, true);
  }

  fn message_height(&mut self, message: &Message) -> i32 {
    let width = if message.role == Role::User { 698 } else { 856 };
    let content = message_content(message, self.state.is_streaming);
    let mut height = 44 + self.canvas.paragraph(content, 0, 0, 21, INK, width, false) + 36;
    if self.ui.show_reasoning && !message.reasoning_content.is_empty() {
      height +=
        37 + self.canvas.paragraph(&message.reasoning_content, 0, 0, 17, MUTED, width, false);
    }
    if incomplete(message) {
      height += 28;
    }
    height
  }

  fn messages(&mut self, messages: &[Message]) {
    let view_height = 412;
    // Measure once: layout and draw must agree even when a message has no content yet.
    let heights: Vec<i32> = messages.iter().map(|message| self.message_height(message)).collect();
    let total = heights.iter().fold(20_i32, |total, height| total.saturating_add(*height));
    let max_scroll = (total - view_height).max(0);
    self.ui.scroll =
      if self.ui.follow_bottom { max_scroll } else { self.ui.scroll.clamp(0, max_scroll) };
    self.canvas.clip(rect(306, 94, 936, view_height));
    let mut y = 106 - self.ui.scroll;
    for (message, height) in messages.iter().zip(heights) {
      if y + height > 94 && y < 506 {
        let user = message.role == Role::User;
        let (x, width) = if user { (480, 738) } else { (322, 896) };
        self.canvas.rect(rect(x, y, width, height - 16), if user { ELEVATED } else { PANEL }, 10);
        self.canvas.text(
          if user { "YOU" } else { "DEEPSEEK" },
          x + 20,
          y + 16,
          13,
          if user { MUTED } else { ACCENT },
          0,
        );
        let mut content_y = y + 44;
        if self.ui.show_reasoning && !message.reasoning_content.is_empty() {
          self.canvas.text("THINKING", x + 20, content_y, 13, MUTED, 0);
          content_y += 25;
          content_y += self.canvas.paragraph(
            &message.reasoning_content,
            x + 20,
            content_y,
            17,
            MUTED,
            width - 40,
            true,
          );
          content_y += 12;
        }
        self.canvas.paragraph(
          message_content(message, self.state.is_streaming),
          x + 20,
          content_y,
          21,
          INK,
          width - 40,
          true,
        );
        if incomplete(message) {
          self.canvas.text(
            "Interrupted · Retry from Settings",
            x + 20,
            y + height - 44,
            14,
            DANGER,
            0,
          );
        }
      }
      y += height;
    }
    self.canvas.unclip();
    if max_scroll > 0 {
      let thumb = (view_height * view_height / total).max(24);
      let offset =
        (i64::from(view_height - thumb) * i64::from(self.ui.scroll) / i64::from(max_scroll)) as i32;
      self.canvas.rect(rect(1235, 94 + offset, 3, thumb), MUTED, 1);
    }
  }

  fn settings(&mut self) {
    self.canvas.text("MAKE YOURSELF AT HOME", 318, 30, 15, ACCENT, 0);
    self.canvas.text("Settings", 312, 68, 36, INK, 0);
    self.button("updates", "Updates", rect(1030, 64, 206, 46), Action::Updates, false);
    let state = self.state;
    let settings = &state.settings;
    let key = if settings.api_key.is_empty() {
      "Add your key"
    } else if self.ui.environment_key {
      "••••••••  ·  from environment"
    } else {
      "••••••••  ·  saved locally"
    };
    let prompt = preview(&settings.system_prompt, 64);
    let token_limit = format!("{}", settings.max_tokens);
    let rows = [
      (Setting::ApiKey, "setting-api_key", "DeepSeek API key", key),
      (Setting::Model, "model-setting", "Model", settings.model.as_str()),
      (
        Setting::Thinking,
        "setting-thinking",
        "Thinking mode",
        if settings.thinking { "On" } else { "Off" },
      ),
      (Setting::BaseUrl, "setting-base_url", "API endpoint", settings.base_url.as_str()),
      (Setting::SystemPrompt, "setting-system_prompt", "Assistant instructions", prompt.as_str()),
      (Setting::MaxTokens, "setting-max_tokens", "Response token limit", token_limit.as_str()),
    ];
    for (index, (setting, id, title, value)) in rows.into_iter().enumerate() {
      let y = 132 + index as i32 * 62;
      self.canvas.rect(rect(314, y, 922, 52), ELEVATED, 7);
      self.canvas.text(title, 332, y + 14, 19, INK, 0);
      self.canvas.clip(rect(626, y, 590, 52));
      self.canvas.text(if value.is_empty() { "Default" } else { value }, 638, y + 15, 17, MUTED, 0);
      self.canvas.unclip();
      self.add_button(id, rect(314, y, 922, 52), Action::EditSetting(setting));
    }
    self.button("retry", "Retry last response", rect(314, 526, 272, 46), Action::Retry, false);
    self.button("delete", "Delete this chat", rect(604, 526, 254, 46), Action::DeleteChat, false);
    self.button("back", "Back to chat", rect(876, 526, 360, 46), Action::BackToChat, true);
    let note = if self.ui.notice.is_empty() {
      "API keys and conversations stay on this device. Messages go to your configured provider."
    } else {
      &self.ui.notice
    };
    self.canvas.text(
      &preview(note, 230),
      318,
      600,
      16,
      if self.ui.notice.is_empty() { MUTED } else { DANGER },
      910,
    );
  }

  fn updates(&mut self, status: &UpdateStatus, version: &str, demo: bool, install_supported: bool) {
    self.canvas.text(&format!("WODEX {version}"), 318, 30, 15, ACCENT, 0);
    self.canvas.text("Updates", 312, 68, 36, INK, 0);
    self.button(
      "update-back",
      "Back to Settings",
      rect(994, 64, 242, 46),
      Action::UpdateBack,
      false,
    );
    self.canvas.text("GITHUB RELEASE SOURCE", 318, 128, 14, MUTED, 0);
    self.canvas.rect(rect(314, 158, 922, 62), ELEVATED, 9);
    self.canvas.clip(rect(332, 172, 690, 34));
    let state = self.state;
    let repository = &state.settings.update_repository;
    self.canvas.text(
      if repository.is_empty() { "Not configured" } else { repository },
      334,
      173,
      21,
      if repository.is_empty() { MUTED } else { INK },
      0,
    );
    self.canvas.unclip();
    let active = matches!(status.phase, UpdatePhase::Checking | UpdatePhase::Downloading);
    if !active && !demo {
      self.button(
        "update-source",
        "Change",
        rect(1074, 168, 146, 42),
        Action::EditSetting(Setting::UpdateRepository),
        false,
      );
    }
    self.canvas.rect(rect(314, 244, 922, 244), PANEL, 12);
    self.canvas.text(
      phase_title(status.phase),
      338,
      266,
      26,
      if status.phase == UpdatePhase::Error { DANGER } else { INK },
      0,
    );
    if !status.version.is_empty() {
      self.canvas.text(&format!("Version {}", status.version), 340, 312, 18, ACCENT, 0);
    }
    let message = if !status.message.is_empty() {
      &status.message
    } else if repository.is_empty() {
      "Set a public GitHub repository to publish and receive releases."
    } else {
      "Check for a new version before downloading."
    };
    self.canvas.clip(rect(338, 344, 870, 100));
    self.canvas.text(&preview(message, 600), 340, 344, 18, MUTED, 864);
    self.canvas.unclip();
    if matches!(status.phase, UpdatePhase::Downloading | UpdatePhase::Ready) {
      self.canvas.rect(rect(340, 438, 864, 7), BORDER, 3);
      let progress = if status.total == 0 {
        0
      } else {
        ((status.downloaded.min(status.total) as u128 * 864) / status.total as u128) as i32
      };
      self.canvas.rect(rect(340, 438, progress, 7), ACCENT, 3);
      self.canvas.text(
        &format!("{} / {}", size_label(status.downloaded), size_label(status.total)),
        340,
        455,
        15,
        MUTED,
        0,
      );
    }
    if active {
      self.button("update-cancel", "Cancel", rect(314, 512, 210, 50), Action::UpdateCancel, false);
    } else if !repository.is_empty() && !demo {
      self.button(
        "update-check",
        "Check for updates",
        rect(314, 512, 240, 50),
        Action::UpdateCheck,
        false,
      );
    } else {
      let area = rect(314, 512, 240, 50);
      self.canvas.rect(area, PANEL, 9);
      self.canvas.text("Check for updates", area.x + 16, area.y + 12, 20, MUTED, 0);
    }
    if install_supported && !demo {
      match status.phase {
        UpdatePhase::Available => self.button(
          "update-download",
          "Download",
          rect(572, 512, 224, 50),
          Action::UpdateDownload,
          true,
        ),
        UpdatePhase::Ready => self.button(
          "update-install",
          "Install and exit",
          rect(816, 512, 420, 50),
          Action::UpdateInstall,
          true,
        ),
        _ => {}
      }
    }
    let hint = if !self.ui.notice.is_empty() {
      self.ui.notice.as_str()
    } else if demo {
      "Offline demo: update checks are disabled."
    } else if !install_supported {
      "Self-install is available when Wodex is launched as an NRO on your Switch."
    } else {
      "Installing closes Wodex. Your conversations and the previous app version are kept."
    };
    self.canvas.text(
      &preview(hint, 240),
      318,
      592,
      16,
      if self.ui.notice.is_empty() { MUTED } else { DANGER },
      910,
    );
  }

  fn editor(&mut self, editor: EditorView<'_>) {
    self.canvas.rect(rect(0, 0, 1280, 720), Color { r: 0, g: 0, b: 0, a: 175 }, 0);
    self.canvas.rect(rect(232, 158, 816, 398), PANEL, 16);
    self.canvas.text(editor.title, 266, 188, 28, INK, 0);
    self.canvas.rect(rect(262, 248, 756, 202), BACKGROUND, 10);
    self.canvas.clip(rect(278, 264, 724, 168));
    let display = if editor.secret {
      format!(
        "{}│{}",
        "*".repeat(editor.value.len().min(64)),
        if editor.composition.is_empty() { "" } else { "***" }
      )
    } else {
      let mut cursor = editor.cursor.min(editor.value.len());
      while !editor.value.is_char_boundary(cursor) {
        cursor -= 1;
      }
      format!("{}│{}{}", &editor.value[..cursor], editor.composition, &editor.value[cursor..])
    };
    self.canvas.paragraph(
      &preview(&display, 12000),
      280,
      268,
      22,
      if editor.select_all { ACCENT } else { INK },
      716,
      true,
    );
    self.canvas.unclip();
    self.canvas.text(
      "Enter  Done     Shift + Enter  New line     Esc  Cancel",
      266,
      476,
      17,
      MUTED,
      0,
    );
    self.canvas.text(
      "Ctrl / Cmd + A  Select all     Ctrl / Cmd + V  Paste     Ctrl + U  Clear",
      266,
      514,
      14,
      MUTED,
      0,
    );
  }
}

fn incomplete(message: &Message) -> bool {
  matches!(message.status, MessageStatus::Interrupted | MessageStatus::Error)
}

fn message_content(message: &Message, streaming: bool) -> &str {
  if !message.content.is_empty() {
    &message.content
  } else if streaming {
    "Thinking…"
  } else {
    "No answer received."
  }
}

fn phase_title(phase: UpdatePhase) -> &'static str {
  match phase {
    UpdatePhase::Checking => "Checking for updates…",
    UpdatePhase::Current => "You’re up to date",
    UpdatePhase::Available => "A new version is available",
    UpdatePhase::Downloading => "Downloading your update…",
    UpdatePhase::Ready => "Your update is ready to install",
    UpdatePhase::Cancelled => "Update cancelled",
    UpdatePhase::Error => "Update could not be completed",
    UpdatePhase::Idle => "Keep Wodex up to date",
  }
}

fn size_label(bytes: u64) -> String {
  let tenths = (u128::from(bytes) * 10 + 512 * 1024) / (1024 * 1024);
  format!("{}.{} MiB", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::canvas::test_backend::canvas;
  use crate::state::{Conversation, WodexCore};
  use alloc::vec;

  fn status(phase: UpdatePhase) -> UpdateStatus {
    UpdateStatus {
      phase,
      message: String::new(),
      version: String::new(),
      notes: String::new(),
      downloaded: 0,
      total: 0,
    }
  }

  #[test]
  fn update_controls_require_source_and_native_install_support() {
    let (_backend, mut canvas) = canvas();
    let mut state = WodexCore::default().state().clone();
    let mut ui = ViewState { page: Page::Updates, ..ViewState::default() };
    let buttons =
      render(&mut canvas, &state, &mut ui, &status(UpdatePhase::Idle), "0.2.0", false, true, None);
    assert!(buttons
      .iter()
      .any(|button| button.action == Action::EditSetting(Setting::UpdateRepository)));
    assert!(!buttons.iter().any(|button| button.action == Action::UpdateCheck));
    assert!(!buttons.iter().any(|button| button.action == Action::UpdateInstall));

    state.settings.update_repository = "example/wodex".into();
    let buttons = render(
      &mut canvas,
      &state,
      &mut ui,
      &status(UpdatePhase::Ready),
      "0.2.0",
      false,
      false,
      None,
    );
    assert!(buttons.iter().any(|button| button.action == Action::UpdateCheck));
    assert!(!buttons.iter().any(|button| button.action == Action::UpdateInstall));
    let buttons =
      render(&mut canvas, &state, &mut ui, &status(UpdatePhase::Ready), "0.2.0", false, true, None);
    assert!(buttons.iter().any(|button| button.action == Action::UpdateInstall));
    drop(canvas);
  }

  #[test]
  fn active_update_and_demo_cannot_change_source_or_start_another_transfer() {
    let (_backend, mut canvas) = canvas();
    let mut state = WodexCore::default().state().clone();
    state.settings.update_repository = "example/wodex".into();
    let mut ui = ViewState { page: Page::Updates, ..ViewState::default() };
    let buttons = render(
      &mut canvas,
      &state,
      &mut ui,
      &status(UpdatePhase::Downloading),
      "0.2.0",
      false,
      true,
      None,
    );
    assert!(buttons.iter().any(|button| button.action == Action::UpdateCancel));
    assert!(!buttons.iter().any(|button| matches!(
      button.action,
      Action::UpdateCheck
        | Action::UpdateDownload
        | Action::UpdateInstall
        | Action::EditSetting(Setting::UpdateRepository)
    )));
    let buttons = render(
      &mut canvas,
      &state,
      &mut ui,
      &status(UpdatePhase::Available),
      "0.2.0",
      true,
      true,
      None,
    );
    assert!(!buttons.iter().any(|button| matches!(
      button.action,
      Action::UpdateCheck
        | Action::UpdateDownload
        | Action::UpdateInstall
        | Action::EditSetting(Setting::UpdateRepository)
    )));
    drop(canvas);
  }

  #[test]
  fn long_chinese_message_remains_scrollable_and_renders_its_tail() {
    let (backend, mut canvas) = canvas();
    let mut state = WodexCore::default().state().clone();
    let mut content = "中文内容，继续向下滚动。\n".repeat(120);
    content.push_str("这是真正的最后一行。终点。");
    state.conversations[0].messages.push(Message {
      role: Role::Assistant,
      content,
      reasoning_content: String::new(),
      status: MessageStatus::Complete,
      finish_reason: None,
      usage: None,
    });
    let mut ui = ViewState::default();
    render(&mut canvas, &state, &mut ui, &status(UpdatePhase::Idle), "0.2.0", false, false, None);
    assert!(ui.scroll > 2000);
    assert!(backend.drawn_text.iter().any(|text| text.contains("最后一行")));
    ui.follow_bottom = false;
    ui.scroll = 80;
    render(&mut canvas, &state, &mut ui, &status(UpdatePhase::Idle), "0.2.0", false, false, None);
    assert_eq!(ui.scroll, 80);
    assert!(canvas.take_error().is_none());
    drop(canvas);
  }

  #[test]
  fn sidebar_shows_active_chat_in_last_six_and_editor_masks_secret_composition() {
    let (backend, mut canvas) = canvas();
    let mut state = WodexCore::default().state().clone();
    state.conversations = (1..=10)
      .map(|index| Conversation {
        id: format!("chat-{index}"),
        title: format!("Conversation {index}"),
        messages: vec![],
      })
      .collect();
    state.active_id = "chat-9".into();
    let mut ui = ViewState::default();
    let buttons = render(
      &mut canvas,
      &state,
      &mut ui,
      &status(UpdatePhase::Idle),
      "0.2.0",
      false,
      false,
      Some(EditorView {
        title: "API key",
        value: "sk-secret-中文",
        composition: "秘密",
        cursor: 4,
        secret: true,
        select_all: false,
      }),
    );
    let chats: Vec<&str> = buttons
      .iter()
      .filter_map(|button| match &button.action {
        Action::SelectChat(id) => Some(id.as_str()),
        _ => None,
      })
      .collect();
    assert_eq!(chats, ["chat-4", "chat-5", "chat-6", "chat-7", "chat-8", "chat-9"]);
    assert!(!backend
      .drawn_text
      .iter()
      .any(|text| text.contains("sk-secret") || text.contains("秘密")));
    assert!(backend.drawn_text.iter().any(|text| text.contains('│') && text.contains("***")));
    drop(canvas);
  }
}
