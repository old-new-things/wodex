//! Application policy and views shared by desktop and the no_std Switch build.
//! Native code owns OS resources; this module owns every user interaction.
pub(crate) mod canvas;
mod editor;
pub(crate) mod host;
mod view;

use crate::state::{Command, SettingsPatch, WodexCore};
use alloc::{
  format,
  string::{String, ToString},
  vec::Vec,
};
use canvas::Canvas;
use editor::{EditResult, EditTarget, Editor};
use host::{Host, Input, Key, TransportEvent, UpdatePhase, UpdateStatus, CTRL, GUI};
use serde_json::Value;
use view::{Action, Button, Page, Setting, ViewState};

pub struct App<H: Host> {
  host: H,
  core: WodexCore,
  ui: ViewState,
  editor: Option<Editor>,
  buttons: Vec<Button>,
  version: String,
  demo: bool,
  running: bool,
  checked_repository: String,
  stored_key: String,
  last_save: u64,
}

impl<H: Host> App<H> {
  pub fn new(mut host: H, version: &str, demo: bool, show_updates: bool) -> Result<Self, String> {
    let saved = if demo { Vec::new() } else { host.load()? };
    let saved_text = core::str::from_utf8(&saved).unwrap_or("invalid saved data");
    let core = WodexCore::new((!saved.is_empty()).then_some(saved_text));
    let mut app = Self {
      host,
      core,
      ui: ViewState::default(),
      editor: None,
      buttons: Vec::new(),
      version: version.into(),
      demo,
      running: true,
      checked_repository: String::new(),
      stored_key: String::new(),
      last_save: 0,
    };
    if demo {
      app.seed_demo();
    } else {
      if app.core.execute(Command::Export {}).is_err() {
        // Recovery must succeed before resetting the in-memory write guard.
        // Keep arbitrary corrupt bytes intact, including non-UTF-8 and NULs.
        app.host.preserve(&saved)?;
        app.core.execute(Command::ResetStorage {})?;
        app.ui.notice =
          "Saved data could not be read. A recovery copy is beside state.json.".into();
      }
      let key = app.host.environment_key();
      if !key.is_empty() {
        let stored_key = app.core.state().settings.api_key.clone();
        let patch = SettingsPatch { api_key: Some(key), ..Default::default() };
        if app.core.execute(Command::Settings { settings: patch }).is_ok() {
          app.stored_key = stored_key;
          app.ui.environment_key = true;
        } else {
          app.ui.notice = "DEEPSEEK_API_KEY is invalid. Enter a valid key in Settings.".into();
        }
      }
      if app.ui.notice.is_empty() && app.core.state().settings.api_key.is_empty() {
        app.ui.notice = "Welcome to Wodex. Add your DeepSeek API key in Settings to begin.".into();
      }
    }
    if show_updates {
      app.ui.page = Page::Updates;
      app.ui.notice.clear();
      app.ui.focused = "update-source".into();
    }
    Ok(app)
  }

  pub fn running(&self) -> bool {
    self.running && self.host.running()
  }

  pub fn tick(&mut self) {
    while let Some(event) = self.host.poll_transport() {
      match event {
        TransportEvent::Data(data) if self.streaming() => {
          let result = self.core.feed(&data);
          if !result["ok"].as_bool().unwrap_or(false) {
            self.host.cancel();
            self.ui.notice =
              result["error"].as_str().unwrap_or("Could not read the response.").into();
          }
        }
        TransportEvent::Finished { http_status, error } => {
          if self.streaming() {
            self.mutate(Command::Finish {
              http_status: (http_status != 0).then_some(http_status),
              error: (!error.is_empty()).then_some(error),
            });
          } else {
            self.save();
          }
        }
        _ => {}
      }
    }
    if self.ui.page == Page::Updates
      && self.ui.focused == "update-cancel"
      && !self.host.update_active()
    {
      self.ui.focused = match self.host.update_status().phase {
        UpdatePhase::Ready if self.host.update_supported() => "update-install",
        UpdatePhase::Available if self.host.update_supported() => "update-download",
        _ => "update-check",
      }
      .into();
    }
    self.host.begin_frame();
    while let Some(input) = self.host.poll_input() {
      self.input(input);
      if !self.running {
        return;
      }
    }
    if self.streaming() && self.host.now_ms().saturating_sub(self.last_save) > 10_000 {
      self.save();
    }
  }

  pub fn render(&mut self, canvas: &mut Canvas) -> Result<(), String> {
    let mut status = self.host.update_status();
    if !self.host.update_active()
      && self.core.state().settings.update_repository != self.checked_repository
    {
      status = UpdateStatus {
        message: "Check this version source before downloading an update.".into(),
        ..Default::default()
      };
    }
    self.buttons = view::render(
      canvas,
      self.core.state(),
      &mut self.ui,
      &status,
      &self.version,
      self.demo,
      self.host.update_supported(),
      self.editor.as_ref().map(Editor::view),
    );
    canvas.take_error().map_or(Ok(()), Err)
  }

  fn streaming(&self) -> bool {
    self.core.state().is_streaming
  }

  fn busy(&self) -> bool {
    self.streaming() || self.host.network_active()
  }

  fn save(&mut self) -> bool {
    if self.demo {
      return true;
    }
    let result = self.core.execute(Command::Export {}).and_then(|mut result| {
      if self.ui.environment_key {
        result["data"]["settings"]["api_key"] = Value::String(self.stored_key.clone());
      }
      serde_json::to_string(&result["data"])
        .map_err(|_| String::from("Could not serialize history."))
    });
    match result.and_then(|content| self.host.save(&content)) {
      Ok(()) => {
        self.last_save = self.host.now_ms();
        true
      }
      Err(_) => {
        self.ui.notice =
          "Could not save to storage. Check free space and write permissions.".into();
        false
      }
    }
  }

  fn mutate(&mut self, command: Command) -> Option<Value> {
    let result = self.core.execute(command);
    self.ui.notice = result.as_ref().err().cloned().unwrap_or_default();
    self.save();
    result.ok()
  }

  fn start(&mut self, command: Command) {
    if self.demo {
      self.ui.notice =
        "Offline demo. Relaunch without --demo to connect your DeepSeek account.".into();
      return;
    }
    if self.busy() {
      return;
    }
    let Some(result) = self.mutate(command) else { return };
    let request = &result["request"];
    let (Some(url), Some(key), Some(body)) =
      (request["url"].as_str(), request["api_key"].as_str(), request.get("body"))
    else {
      return;
    };
    if self.host.start(url, key, &body.to_string()).is_err() {
      self.mutate(Command::Finish {
        http_status: None,
        error: Some("Could not start the network request.".into()),
      });
    } else {
      self.ui.notice.clear();
      self.ui.follow_bottom = true;
    }
  }

  fn send(&mut self) {
    if self.ui.draft.is_empty() || self.busy() {
      return;
    }
    self.start(Command::Send { content: self.ui.draft.clone() });
    if self.streaming() {
      self.ui.draft.clear();
    }
  }

  fn input(&mut self, input: Input) {
    if input == Input::Quit {
      self.close();
      return;
    }
    if self.editor.is_some() {
      self.editor_input(input);
      return;
    }
    match input {
      Input::Key { key, modifiers } => self.keyboard(key, modifiers),
      Input::Gamepad(button) => self.gamepad(button),
      Input::Pointer { x, y } => {
        if let Some(button) = self.buttons.iter().find(|button| {
          x >= button.area.x
            && x < button.area.x + button.area.w
            && y >= button.area.y
            && y < button.area.y + button.area.h
        }) {
          let action = button.action.clone();
          self.ui.focused.clone_from(&button.id);
          if button.id != "delete" {
            self.ui.confirm_delete = false;
          }
          self.dispatch(action);
        }
      }
      Input::Scroll(delta) if self.ui.page == Page::Chat => self.scroll(delta),
      _ => {}
    }
  }

  fn editor_input(&mut self, input: Input) {
    let Some(editor) = &mut self.editor else { return };
    let result = match input {
      Input::Text(text) => {
        editor.insert(&text);
        EditResult::Continue
      }
      Input::Composition(text) => {
        editor.composition = text;
        EditResult::Continue
      }
      Input::Key { key, modifiers } => editor.key(key, modifiers),
      _ => EditResult::Continue,
    };
    match result {
      EditResult::Continue => {}
      EditResult::Paste => {
        let clipboard = self.host.clipboard();
        self.editor.as_mut().expect("editor is open").insert(&clipboard);
      }
      EditResult::Cancel => {
        self.editor = None;
        self.host.text_input(false);
      }
      EditResult::Commit => {
        let editor = self.editor.take().expect("editor is open");
        self.host.text_input(false);
        self.commit_edit(editor.target, editor.value);
      }
    }
  }

  fn keyboard(&mut self, key: Key, modifiers: u32) {
    if key == Key::Enter && modifiers & (CTRL | GUI) != 0 {
      self.send();
      return;
    }
    match key {
      Key::Left => self.move_focus(-1, 0),
      Key::Right => self.move_focus(1, 0),
      Key::Up => self.move_focus(0, -1),
      Key::Down => self.move_focus(0, 1),
      Key::Enter | Key::Space => self.activate(),
      Key::X => self.compose(),
      Key::Y => self.ui.show_reasoning = !self.ui.show_reasoning,
      Key::Escape => self.back(),
      Key::F1 | Key::Minus => self.toggle_settings(),
      Key::PageUp => self.scroll(-280),
      Key::PageDown => self.scroll(280),
      Key::End => self.ui.follow_bottom = true,
      Key::LeftBracket => self.cycle_chat(-1),
      Key::RightBracket => self.cycle_chat(1),
      _ => {}
    }
  }

  fn gamepad(&mut self, button: u8) {
    match button {
      0 => self.activate(),
      1 => self.back(),
      2 => self.compose(),
      3 => self.ui.show_reasoning = !self.ui.show_reasoning,
      4 => self.toggle_settings(),
      6 => self.close(),
      9 => self.cycle_chat(-1),
      10 => self.cycle_chat(1),
      11 => self.move_focus(0, -1),
      12 => self.move_focus(0, 1),
      13 => self.move_focus(-1, 0),
      14 => self.move_focus(1, 0),
      _ => {}
    }
  }

  fn scroll(&mut self, delta: i32) {
    self.ui.scroll = self.ui.scroll.saturating_add(delta).max(0);
    self.ui.follow_bottom = false;
  }

  fn move_focus(&mut self, dx: i32, dy: i32) {
    let Some(current) = self.buttons.iter().find(|button| button.id == self.ui.focused) else {
      if let Some(first) = self.buttons.first() {
        self.ui.focused.clone_from(&first.id);
      }
      return;
    };
    let x = current.area.x + current.area.w / 2;
    let y = current.area.y + current.area.h / 2;
    let mut best = f64::MAX;
    let mut candidate = &self.ui.focused;
    for button in &self.buttons {
      let tx = button.area.x + button.area.w / 2 - x;
      let ty = button.area.y + button.area.h / 2 - y;
      let forward = if dx != 0 { tx * dx } else { ty * dy };
      if forward <= 0 {
        continue;
      }
      let sideways = if dx != 0 { ty.abs() } else { tx.abs() } as f64;
      let score = forward as f64 + sideways * 3.0 + sideways * sideways / forward as f64;
      if score < best {
        best = score;
        candidate = &button.id;
      }
    }
    self.ui.focused = candidate.clone();
    if self.ui.focused != "delete" {
      self.ui.confirm_delete = false;
    }
  }

  fn activate(&mut self) {
    if let Some(button) = self.buttons.iter().find(|button| button.id == self.ui.focused) {
      self.dispatch(button.action.clone());
    }
  }

  fn dispatch(&mut self, action: Action) {
    let page = self.ui.page;
    match action {
      Action::NewChat if !self.busy() => {
        if self.mutate(Command::NewChat {}).is_some() {
          self.open_chat();
        }
      }
      Action::SelectChat(id) if !self.busy() => {
        if self.mutate(Command::SelectChat { id }).is_some() {
          self.open_chat();
        }
      }
      Action::Settings => self.toggle_settings(),
      Action::OpenModel => {
        self.ui.page = Page::Settings;
        self.ui.focused = "model-setting".into();
      }
      Action::Prompt(index) if !self.busy() => {
        if let Some((_, prompt)) = view::PROMPTS.get(index) {
          self.edit("Write a message", (*prompt).into(), false, EditTarget::Draft);
        }
      }
      Action::Compose => self.compose(),
      Action::Send => self.send(),
      Action::Stop => self.stop(),
      Action::EditSetting(setting) => self.edit_setting(setting),
      Action::Updates => {
        self.ui.page = Page::Updates;
        self.ui.confirm_delete = false;
        self.ui.notice.clear();
        self.ui.focused = if self.core.state().settings.update_repository.is_empty() {
          "update-source"
        } else {
          "update-check"
        }
        .into();
      }
      Action::Retry => {
        self.ui.page = Page::Chat;
        self.start(Command::Retry {});
      }
      Action::DeleteChat if !self.busy() => {
        if self.ui.confirm_delete {
          self.mutate(Command::DeleteChat { id: self.core.state().active_id.clone() });
          self.ui.confirm_delete = false;
          self.open_chat();
        } else {
          self.ui.confirm_delete = true;
          self.ui.notice = "Select Delete again to permanently remove this chat.".into();
        }
      }
      Action::BackToChat => {
        self.ui.page = Page::Chat;
        self.ui.confirm_delete = false;
        self.ui.focused = "compose".into();
      }
      Action::UpdateCheck if !self.demo && !self.host.update_active() => {
        let repository = self.core.state().settings.update_repository.clone();
        let result = self.host.update_check(&repository);
        if self.update_result(result) {
          self.checked_repository = repository;
          self.ui.focused = "update-cancel".into();
        }
      }
      Action::UpdateDownload
        if !self.demo
          && self.host.update_supported()
          && !self.checked_repository.is_empty()
          && !self.host.update_active()
          && self.host.update_status().phase == UpdatePhase::Available
          && self.core.state().settings.update_repository == self.checked_repository =>
      {
        let result = self.host.update_download();
        if self.update_result(result) {
          self.ui.focused = "update-cancel".into();
        }
      }
      Action::UpdateCancel => self.host.update_cancel(),
      Action::UpdateInstall => self.install(),
      Action::UpdateBack => {
        self.ui.page = Page::Settings;
        self.ui.notice.clear();
        self.ui.focused = "updates".into();
      }
      _ => {}
    }
    // An input burst after changing pages must not activate a stale button.
    if page != self.ui.page {
      self.buttons.clear();
    }
  }

  fn update_result(&mut self, result: Result<(), String>) -> bool {
    match result {
      Ok(()) => {
        self.ui.notice.clear();
        true
      }
      Err(error) => {
        self.ui.notice = error;
        false
      }
    }
  }

  fn install(&mut self) {
    if self.demo
      || !self.host.update_supported()
      || self.checked_repository.is_empty()
      || self.host.update_active()
      || self.host.update_status().phase != UpdatePhase::Ready
      || self.core.state().settings.update_repository != self.checked_repository
    {
      return;
    }
    if self.busy() {
      self.ui.notice = "Stop the current reply before installing the update.".into();
      return;
    }
    if !self.save() {
      return;
    }
    match self.host.prepare_install() {
      Ok(true) => {
        self.host.update_cancel();
        self.running = false;
      }
      Ok(false) => {}
      Err(error) => self.ui.notice = error,
    }
  }

  fn edit_setting(&mut self, setting: Setting) {
    self.ui.confirm_delete = false;
    if self.busy() {
      self.ui.notice = "Stop the current response before changing settings.".into();
      return;
    }
    if setting == Setting::UpdateRepository && (self.demo || self.host.update_active()) {
      return;
    }
    let settings = &self.core.state().settings;
    let (title, value, secret) = match setting {
      Setting::ApiKey => ("DeepSeek API key", settings.api_key.clone(), true),
      Setting::Model => ("Model", settings.model.clone(), false),
      Setting::Thinking => {
        let patch = SettingsPatch { thinking: Some(!settings.thinking), ..Default::default() };
        self.mutate(Command::Settings { settings: patch });
        return;
      }
      Setting::BaseUrl => ("API endpoint", settings.base_url.clone(), false),
      Setting::SystemPrompt => ("Assistant instructions", settings.system_prompt.clone(), false),
      Setting::MaxTokens => ("Response token limit", format!("{}", settings.max_tokens), false),
      Setting::UpdateRepository => {
        ("GitHub repository (owner/repo)", settings.update_repository.clone(), false)
      }
    };
    self.edit(title, value, secret, EditTarget::Setting(setting));
  }

  fn edit(&mut self, title: &str, initial: String, secret: bool, target: EditTarget) {
    if self.host.is_switch() {
      match self.host.edit_system(title, &initial, secret) {
        Ok(value) => self.commit_edit(target, value),
        Err(error) => self.ui.notice = error,
      }
    } else {
      self.editor = Some(Editor::new(title, initial, secret, target));
      self.host.text_input(true);
    }
  }

  fn commit_edit(&mut self, target: EditTarget, value: String) {
    let EditTarget::Setting(setting) = target else {
      self.ui.draft = value;
      self.ui.focused = "send".into();
      return;
    };
    let mut patch = SettingsPatch::default();
    match setting {
      Setting::ApiKey => patch.api_key = Some(value),
      Setting::Model => patch.model = Some(value),
      Setting::BaseUrl => patch.base_url = Some(value),
      Setting::SystemPrompt => patch.system_prompt = Some(value),
      Setting::UpdateRepository => patch.update_repository = Some(value),
      Setting::MaxTokens => {
        let Ok(tokens) = value.parse::<u32>() else {
          self.ui.notice = "The response limit must be a whole number.".into();
          return;
        };
        patch.max_tokens = Some(tokens);
      }
      Setting::Thinking => return,
    }
    if self.mutate(Command::Settings { settings: patch }).is_some() {
      if setting == Setting::ApiKey {
        self.ui.environment_key = false;
        self.save();
      } else if setting == Setting::UpdateRepository {
        self.checked_repository.clear();
        self.buttons.clear();
        self.ui.focused = if self.core.state().settings.update_repository.is_empty() {
          "update-source"
        } else {
          "update-check"
        }
        .into();
      }
    }
  }

  fn compose(&mut self) {
    if !self.busy() {
      self.ui.page = Page::Chat;
      self.buttons.clear();
      self.edit("Write a message", self.ui.draft.clone(), false, EditTarget::Draft);
    }
  }

  fn open_chat(&mut self) {
    self.ui.page = Page::Chat;
    self.ui.scroll = 0;
    self.ui.follow_bottom = true;
    self.ui.draft.clear();
    self.ui.confirm_delete = false;
    self.buttons.clear();
  }

  fn cycle_chat(&mut self, direction: i32) {
    if self.busy() {
      return;
    }
    let state = self.core.state();
    if let Some(index) = state.conversations.iter().position(|chat| chat.id == state.active_id) {
      let next = (index as i32 + direction).rem_euclid(state.conversations.len() as i32) as usize;
      let id = state.conversations[next].id.clone();
      if self.mutate(Command::SelectChat { id }).is_some() {
        self.open_chat();
      }
    }
  }

  fn toggle_settings(&mut self) {
    self.ui.page = if self.ui.page == Page::Settings { Page::Chat } else { Page::Settings };
    self.ui.confirm_delete = false;
    self.ui.focused =
      if self.ui.page == Page::Settings { "setting-api_key" } else { "compose" }.into();
    self.buttons.clear();
  }

  fn stop(&mut self) {
    self.host.cancel();
    if self.streaming() {
      self.mutate(Command::Cancel {});
    }
  }

  fn back(&mut self) {
    if self.ui.page == Page::Updates {
      if self.host.update_active() {
        self.host.update_cancel();
      } else {
        self.dispatch(Action::UpdateBack);
      }
    } else if self.streaming() {
      self.stop();
    } else if self.ui.page == Page::Settings {
      self.toggle_settings();
    }
  }

  fn close(&mut self) {
    self.host.update_cancel();
    self.host.cancel();
    if self.streaming() {
      let _ = self.core.execute(Command::Cancel {});
    }
    self.save();
    self.running = false;
  }

  fn seed_demo(&mut self) {
    let patch = SettingsPatch {
      api_key: Some("offline-demo".into()),
      thinking: Some(true),
      ..Default::default()
    };
    let _ = self.core.execute(Command::Settings { settings: patch });
    let _ = self
      .core
      .execute(Command::Send { content: "How do I turn a small idea into something real?".into() });
    self.core.feed(b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"A useful starting point is a small, testable first step.\"}}]}\n\n");
    self.core.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"Start with one thing you wish existed.\\n\\n1. Write down who it helps and what it makes easier.\\n2. Build the smallest version that does that one thing.\\n3. Try it in the real world, then let what you learn guide the next step.\\n\\nA little progress, made tangible, is a great place to begin.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    let _ = self.core.execute(Command::Finish { http_status: Some(200), error: None });
    self.ui.notice = "Offline demo · sample conversation · no network requests".into();
  }
}

impl<H: Host> Drop for App<H> {
  fn drop(&mut self) {
    self.host.cancel();
    self.host.update_cancel();
    if self.editor.is_some() {
      self.host.text_input(false);
    }
  }
}

mod ffi;
#[cfg(test)]
mod tests;
