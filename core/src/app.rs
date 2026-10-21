//! Application policy and views shared by desktop and the no_std Switch build.
//! Native code owns OS resources; this module owns every user interaction.
pub(crate) mod canvas;
mod editor;
pub(crate) mod host;
mod markdown;
mod rich_text;
mod view;

use crate::backend::home_menu::HomeMenuPhase;
use crate::backend::transfer::TransferExports;
use crate::state::transfer::{PreparedTransfer, TransferChannel};
use crate::state::{Command, SettingsPatch, WodexCore, OFFICIAL_UPDATE_REPOSITORY};
use crate::update::UpdateChannel;
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

const STORAGE_UNCERTAIN_NOTICE: &str = "Storage could not be verified after import. Editing and saving are paused. Exit and restart Wodex; a recovery copy of your previous data was kept.";

pub struct App<H: Host> {
  host: H,
  core: WodexCore,
  ui: ViewState,
  editor: Option<Editor>,
  buttons: Vec<Button>,
  version: String,
  demo: bool,
  running: bool,
  checked_source: Option<(String, UpdateChannel)>,
  stored_key: String,
  last_save: u64,
  image_loading: bool,
  image_context: String,
  pending_transfer: Option<(TransferChannel, PreparedTransfer)>,
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
      checked_source: None,
      stored_key: String::new(),
      last_save: 0,
      image_loading: true,
      image_context: String::new(),
      pending_transfer: None,
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
      app.apply_environment_key();
      if app.ui.notice.is_empty() && app.core.state().settings.api_key.is_empty() {
        app.ui.notice =
          "Welcome to Wodex. Choose your provider and add an API key in Settings.".into();
      }
    }
    if show_updates {
      app.ui.page = Page::Updates;
      app.ui.notice.clear();
      app.ui.focused = app.update_focus().into();
    }
    Ok(app)
  }

  pub fn running(&self) -> bool {
    self.running && self.host.running()
  }

  fn apply_environment_key(&mut self) {
    self.ui.environment_key = false;
    self.stored_key.clear();
    let key = self.host.environment_key(self.core.state().settings.provider);
    if key.is_empty() {
      return;
    }
    let stored_key = self.core.state().settings.api_key.clone();
    let patch = SettingsPatch { api_key: Some(key), ..Default::default() };
    if self.core.execute(Command::Settings { settings: patch }).is_ok() {
      self.stored_key = stored_key;
      self.ui.environment_key = true;
    } else {
      self.ui.notice =
        "The selected provider's environment API key is invalid. Enter a valid key in Settings."
          .into();
    }
  }

  pub fn set_image_loading(&mut self, enabled: bool) {
    self.image_loading = enabled;
    if !enabled {
      self.host.cancel_images();
    }
  }

  pub fn show_home_menu(&mut self) {
    if self.ui.storage_locked || self.host.home_menu_active() || self.host.transfer_active() {
      return;
    }
    self.ui.page = Page::HomeMenu;
    self.ui.confirm_delete = false;
    self.ui.notice.clear();
    self.ui.focused = self.home_menu_focus().into();
    self.buttons.clear();
    if self.editor.take().is_some() {
      self.host.text_input(false);
    }
    self.check_home_menu();
  }

  pub fn show_transfer(&mut self) {
    if self.ui.storage_locked || self.host.home_menu_active() {
      return;
    }
    self.ui.page = Page::Transfer;
    self.ui.confirm_delete = false;
    self.ui.notice.clear();
    self.ui.focused =
      if self.host.transfer_active() { "transfer-stop" } else { "transfer-start" }.into();
    self.buttons.clear();
    if self.editor.take().is_some() {
      self.host.text_input(false);
    }
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
    if self.ui.page == Page::HomeMenu
      && self.ui.focused == "home-menu-working"
      && !self.host.home_menu_active()
    {
      self.ui.focused = self.home_menu_focus().into();
      self.ui.notice.clear();
      self.buttons.clear();
    }
    self.poll_transfer();
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
    let load_images =
      self.image_loading && !self.demo && !self.host.transfer_active() && !self.ui.storage_locked;
    let context = if load_images && self.ui.page == Page::Chat {
      self.core.state().active_id.as_str()
    } else {
      ""
    };
    if self.image_context != context {
      self.host.cancel_images();
      canvas.clear_images();
      self.image_context = context.into();
    }
    canvas.set_image_loading(load_images);
    canvas.begin_image_frame();
    while let Some(image) = self.host.poll_image() {
      canvas.set_image_result(&image.url, image.result);
    }
    let mut status = self.host.update_status();
    if !self.host.update_active() && !self.update_source_checked() {
      status = UpdateStatus::default();
    }
    self.ui.transfer.status = self.host.transfer_status();
    self.ui.transfer.blocked =
      self.busy() && !self.host.transfer_active() || self.host.update_active();
    self.ui.transfer.summary =
      self.pending_transfer.as_ref().map(|(_, pending)| pending.summary.clone());
    let home_menu = self.host.home_menu_status();
    let home_menu_blocked = self.busy() || self.host.update_active();
    self.buttons = view::render(
      canvas,
      self.core.state(),
      &mut self.ui,
      &status,
      &home_menu,
      home_menu_blocked,
      &self.version,
      self.demo,
      self.host.update_supported(),
      self.editor.as_ref().map(Editor::view),
    );
    if load_images && self.running && !self.host.update_active() && !self.host.home_menu_active() {
      // Candidates come only from visible cards. A single loader handles one
      // candidate; the next frame discovers any remaining visible images.
      if let Some(url) = canvas.take_image_requests().into_iter().next() {
        if self.host.request_image(&url) {
          canvas.mark_image_loading(&url);
        }
      }
    }
    canvas.take_error().map_or(Ok(()), Err)
  }

  fn streaming(&self) -> bool {
    self.core.state().is_streaming
  }

  fn busy(&self) -> bool {
    self.ui.storage_locked
      || self.streaming()
      || self.host.network_active()
      || self.host.home_menu_active()
      || self.host.transfer_active()
  }

  fn update_source_checked(&self) -> bool {
    let settings = &self.core.state().settings;
    self.checked_source.as_ref().is_some_and(|(repository, channel)| {
      !repository.is_empty()
        && *repository == settings.update_repository
        && *channel == settings.update_channel
    })
  }

  fn update_focus(&self) -> &'static str {
    if self.core.state().settings.update_repository.is_empty() {
      "update-official"
    } else {
      "update-check"
    }
  }

  fn invalidate_update_source(&mut self) {
    self.checked_source = None;
    self.buttons.clear();
    self.ui.focused = self.update_focus().into();
  }

  fn update_settings_editable(&mut self) -> bool {
    if self.demo || self.host.update_active() {
      return false;
    }
    if self.busy() {
      self.ui.notice = "Stop the current response before changing settings.".into();
      return false;
    }
    true
  }

  fn save(&mut self) -> bool {
    if self.ui.storage_locked {
      self.ui.notice = STORAGE_UNCERTAIN_NOTICE.into();
      return false;
    }
    if self.demo {
      return true;
    }
    let result = self.persisted_content();
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

  fn persisted_content(&mut self) -> Result<String, String> {
    let mut result = self.core.execute(Command::Export {})?;
    if self.ui.environment_key {
      result["data"]["settings"]["api_key"] = Value::String(self.stored_key.clone());
    }
    serde_json::to_string(&result["data"]).map_err(|_| String::from("Could not serialize history."))
  }

  fn transfer_exports(&mut self) -> Result<TransferExports, String> {
    let snapshot = WodexCore::new(Some(&self.persisted_content()?));
    Ok(TransferExports {
      config: snapshot.export_transfer(TransferChannel::Config)?,
      sessions: snapshot.export_transfer(TransferChannel::Sessions)?,
    })
  }

  fn start_transfer(&mut self) {
    if self.demo || self.ui.page != Page::Transfer || self.host.transfer_active() {
      return;
    }
    if self.busy() || self.host.update_active() {
      self.ui.notice = "Finish the current response or update before starting PC transfer.".into();
      return;
    }
    if !self.save() {
      return;
    }
    let result = self.transfer_exports().and_then(|exports| self.host.transfer_start(exports));
    if self.update_result(result) {
      self.host.cancel_images();
      self.ui.focused = "transfer-stop".into();
      self.buttons.clear();
    }
  }

  fn stop_transfer(&mut self) {
    self.host.transfer_stop();
    self.pending_transfer = None;
    self.ui.transfer.summary = None;
    self.ui.focused = "transfer-start".into();
    self.buttons.clear();
  }

  fn poll_transfer(&mut self) {
    if !self.host.transfer_active() {
      if self.pending_transfer.take().is_some() || self.ui.focused == "transfer-stop" {
        self.ui.transfer.summary = None;
        self.ui.focused = "transfer-start".into();
        self.buttons.clear();
      }
      return;
    }
    if self.pending_transfer.is_some() {
      return;
    }
    let Some(import) = self.host.transfer_take_import() else { return };
    if self.ui.page != Page::Transfer || self.demo {
      self.stop_transfer();
      return;
    }
    let result = self.persisted_content().and_then(|snapshot| {
      WodexCore::new(Some(&snapshot)).prepare_transfer(import.channel, &import.data)
    });
    match result {
      Ok(prepared) => {
        self.pending_transfer = Some((import.channel, prepared));
        self.ui.notice.clear();
        // Reject is the initial choice; confirming always needs a fresh action.
        self.ui.focused = "transfer-reject".into();
        self.buttons.clear();
      }
      Err(error) => {
        self.ui.notice = format!("Import rejected: {error}");
        self.host.transfer_resolve_import(self.ui.notice.clone(), None);
      }
    }
  }

  fn confirm_transfer(&mut self) {
    if self.demo || self.ui.page != Page::Transfer || !self.host.transfer_active() {
      return;
    }
    let Some((channel, mut prepared)) = self.pending_transfer.take() else { return };
    let result = (|| -> Result<bool, String> {
      let previous = self.persisted_content()?;
      let exported = prepared.core.execute(Command::Export {})?;
      let content = serde_json::to_string(&exported["data"])
        .map_err(|_| String::from("Could not serialize imported data."))?;
      let stored_key = prepared.core.state().settings.api_key.clone();
      if channel == TransferChannel::Sessions && self.ui.environment_key {
        // Sessions leave both saved credentials and the user's runtime
        // override decision unchanged.
        prepared.core.execute(Command::Settings {
          settings: SettingsPatch {
            api_key: Some(self.core.state().settings.api_key.clone()),
            ..Default::default()
          },
        })?;
      }
      self.host.preserve(previous.as_bytes())?;
      // A rename may have committed before syncing its directory fails. Reconcile
      // bytes before choosing which state should remain live or be exported.
      let durable = match self.host.save(&content) {
        Ok(()) => true,
        Err(error) => match self.host.load() {
          Ok(saved) if saved == content.as_bytes() => false,
          Ok(saved) if saved == previous.as_bytes() => return Err(error),
          _ => {
            self.ui.storage_locked = true;
            return Err(error);
          }
        },
      };
      self.core = prepared.core;
      if channel == TransferChannel::Config {
        // Resolve the imported provider's credential afresh. Reusing the old
        // runtime key here could send a DeepSeek key to an unrelated service.
        self.apply_environment_key();
      } else {
        self.stored_key = stored_key;
      }
      self.last_save = self.host.now_ms();
      if channel == TransferChannel::Config {
        self.checked_source = None;
      } else {
        self.ui.draft.clear();
        self.ui.scroll = 0;
        self.ui.follow_bottom = true;
        self.ui.rich_text = Default::default();
      }
      Ok(durable)
    })();
    self.ui.transfer.summary = None;
    self.ui.focused = "transfer-stop".into();
    self.buttons.clear();
    match result {
      Ok(durable) => {
        self.ui.notice = if durable {
          format!("{} imported. A recovery copy of your previous data was kept.", channel.label())
        } else {
          format!("{} imported and read back, but storage durability could not be confirmed. A recovery copy of your previous data was kept.", channel.label())
        };
        match self.transfer_exports() {
          Ok(exports) => self.host.transfer_resolve_import(self.ui.notice.clone(), Some(exports)),
          Err(_) => {
            self.stop_transfer();
            self.ui.notice = "Import saved. Restart PC transfer to refresh the downloads.".into();
          }
        }
      }
      Err(_) => {
        if self.ui.storage_locked {
          self.ui.notice = STORAGE_UNCERTAIN_NOTICE.into();
          self.host.transfer_resolve_import(self.ui.notice.clone(), None);
          self.stop_transfer();
          self.host.cancel_images();
        } else {
          self.ui.notice = "Import was not applied. Could not save safely; check free space and write permissions.".into();
          self.host.transfer_resolve_import(self.ui.notice.clone(), None);
        }
      }
    }
  }

  fn reject_transfer(&mut self) {
    if self.pending_transfer.take().is_some() {
      self.host.transfer_resolve_import(
        "Import rejected on the device. Existing data was kept.".into(),
        None,
      );
      self.ui.notice = "Import rejected. Existing data was kept.".into();
      self.ui.transfer.summary = None;
      self.ui.focused = "transfer-stop".into();
      self.buttons.clear();
    }
  }

  fn mutate(&mut self, command: Command) -> Option<Value> {
    if self.ui.storage_locked || self.host.transfer_active() {
      return None;
    }
    let provider = self.core.state().settings.provider;
    let editing_key =
      matches!(&command, Command::Settings { settings } if settings.api_key.is_some());
    let result = self.core.execute(command);
    self.ui.notice = result.as_ref().err().cloned().unwrap_or_default();
    if result.is_ok() {
      if provider != self.core.state().settings.provider {
        self.apply_environment_key();
      } else if editing_key {
        self.ui.environment_key = false;
        self.stored_key.clear();
      }
    }
    self.save();
    result.ok()
  }

  fn start(&mut self, command: Command) {
    if self.demo {
      self.ui.notice = "Offline demo. Relaunch without --demo to connect your provider.".into();
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
    if self.ui.storage_locked {
      if matches!(input, Input::Gamepad(6) | Input::Key { key: Key::Escape, .. }) {
        self.close();
      }
      return;
    }
    if self.host.home_menu_active() {
      // Installation cannot be cancelled, including by a queued shortcut or
      // pointer event from the frame that started it.
      if input == Input::Gamepad(6) {
        self.close();
      }
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
    let mut best = (true, f64::MAX);
    let mut candidate = &self.ui.focused;
    for button in &self.buttons {
      let tx = button.area.x + button.area.w / 2 - x;
      let ty = button.area.y + button.area.h / 2 - y;
      let forward = if dx != 0 { tx * dx } else { ty * dy };
      if forward <= 0 {
        continue;
      }
      // Wide settings rows overlap both provider buttons. Measure the gap
      // between regions so moving down reaches the next row instead of
      // skipping to a distant button whose center happens to line up.
      let (start, end, target_start, target_end) = if dx != 0 {
        (
          current.area.y,
          current.area.y + current.area.h,
          button.area.y,
          button.area.y + button.area.h,
        )
      } else {
        (
          current.area.x,
          current.area.x + current.area.w,
          button.area.x,
          button.area.x + button.area.w,
        )
      };
      let sideways = (start - target_end).max(target_start - end).max(0) as f64;
      let center_offset = if dx != 0 { ty.abs() } else { tx.abs() } as f64;
      let score = (
        sideways > 0.0,
        forward as f64
          + sideways * 3.0
          + sideways * sideways / forward as f64
          + center_offset * 0.001,
      );
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
    if self.ui.storage_locked || self.host.home_menu_active() {
      return;
    }
    if self.host.transfer_active()
      && !matches!(
        action,
        Action::TransferStop
          | Action::TransferBack
          | Action::TransferConfirm
          | Action::TransferReject
      )
    {
      return;
    }
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
      Action::Provider(provider) => self.change_provider(provider),
      Action::Updates => {
        self.ui.page = Page::Updates;
        self.ui.confirm_delete = false;
        self.ui.notice.clear();
        self.ui.focused = self.update_focus().into();
      }
      Action::Transfer => self.show_transfer(),
      Action::TransferStart => self.start_transfer(),
      Action::TransferStop => self.stop_transfer(),
      Action::TransferConfirm => self.confirm_transfer(),
      Action::TransferReject => self.reject_transfer(),
      Action::TransferBack => {
        self.stop_transfer();
        self.ui.page = Page::Settings;
        self.ui.notice.clear();
        self.ui.focused = "transfer".into();
      }
      Action::HomeMenu => self.show_home_menu(),
      Action::HomeMenuCheck => self.check_home_menu(),
      Action::HomeMenuInstall => self.install_home_menu(),
      Action::HomeMenuBack => {
        self.ui.page = Page::Settings;
        self.ui.notice.clear();
        self.ui.focused = "home-menu".into();
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
        let settings = &self.core.state().settings;
        let repository = settings.update_repository.clone();
        let channel = settings.update_channel;
        self.checked_source = None;
        if !repository.is_empty() {
          let result = self.host.update_check(&repository, channel);
          if self.update_result(result) {
            self.checked_source = Some((repository, channel));
            self.ui.focused = "update-cancel".into();
          }
        }
      }
      Action::UpdateOfficial if self.update_settings_editable() => {
        self.commit_edit(
          EditTarget::Setting(Setting::UpdateRepository),
          OFFICIAL_UPDATE_REPOSITORY.into(),
        );
      }
      Action::UpdateChannel(channel) if self.update_settings_editable() => {
        if self.core.state().settings.update_channel != channel {
          let patch = SettingsPatch { update_channel: Some(channel), ..Default::default() };
          if self.mutate(Command::Settings { settings: patch }).is_some() {
            self.invalidate_update_source();
          }
        }
      }
      Action::UpdateDownload
        if !self.demo
          && self.host.update_supported()
          && self.update_source_checked()
          && !self.host.update_active()
          && self.host.update_status().phase == UpdatePhase::Available =>
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

  fn check_home_menu(&mut self) {
    if self.demo || self.ui.page != Page::HomeMenu || self.host.home_menu_active() {
      return;
    }
    if self.busy() || self.host.update_active() {
      self.ui.notice =
        "Finish the current response or update before checking HOME Menu support.".into();
      return;
    }
    let result = self.host.home_menu_check();
    let started = self.update_result(result);
    self.ui.focused = self.home_menu_focus().into();
    if started {
      self.buttons.clear();
    }
  }

  fn home_menu_focus(&self) -> &'static str {
    if self.demo || self.streaming() || self.host.network_active() || self.host.update_active() {
      return "home-menu-back";
    }
    match self.host.home_menu_status().phase {
      HomeMenuPhase::Checking | HomeMenuPhase::Installing => "home-menu-working",
      HomeMenuPhase::Ready => "home-menu-install",
      HomeMenuPhase::Installed | HomeMenuPhase::Unsupported => "home-menu-back",
      _ => "home-menu-check",
    }
  }

  fn install_home_menu(&mut self) {
    if self.demo
      || self.ui.page != Page::HomeMenu
      || self.host.home_menu_active()
      || self.host.home_menu_status().phase != HomeMenuPhase::Ready
    {
      return;
    }
    if self.busy() || self.host.update_active() {
      self.ui.notice =
        "Finish the current response or update before installing the HOME Menu tile.".into();
      return;
    }
    if !self.save() {
      return;
    }
    let result = self.host.home_menu_install();
    if self.update_result(result) {
      self.ui.focused = "home-menu-working".into();
      self.buttons.clear();
    }
  }

  fn install(&mut self) {
    if self.demo
      || !self.host.update_supported()
      || !self.update_source_checked()
      || self.host.update_active()
      || self.host.update_status().phase != UpdatePhase::Ready
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

  fn change_provider(&mut self, provider: crate::provider::Provider) {
    if self.busy() {
      self.ui.notice = "Stop the current response before changing providers.".into();
      return;
    }
    if self.core.state().settings.provider == provider {
      return;
    }
    let patch = SettingsPatch { provider: Some(provider), ..Default::default() };
    if self.mutate(Command::Settings { settings: patch }).is_some() {
      if self.ui.notice.is_empty() {
        self.ui.notice = if self.ui.environment_key {
          "Provider changed. Its environment key is active; review the model and endpoint."
        } else {
          "Provider changed. Add your API key, then review the model and endpoint."
        }
        .into();
      }
      self.ui.focused = if provider == crate::provider::Provider::DeepSeek {
        "provider-deepseek"
      } else {
        "provider-openai"
      }
      .into();
      self.buttons.clear();
    }
  }

  fn edit_setting(&mut self, setting: Setting) {
    use crate::provider::{Provider, TokenLimitField};
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
      Setting::ApiKey => ("API key", settings.api_key.clone(), true),
      Setting::Model => ("Model", settings.model.clone(), false),
      Setting::Thinking => {
        if settings.provider != Provider::DeepSeek {
          return;
        }
        let patch = SettingsPatch { thinking: Some(!settings.thinking), ..Default::default() };
        self.mutate(Command::Settings { settings: patch });
        return;
      }
      Setting::TokenLimitField => {
        if settings.provider != Provider::OpenAiCompatible {
          return;
        }
        let token_limit_field = match settings.token_limit_field {
          TokenLimitField::MaxTokens => TokenLimitField::MaxCompletionTokens,
          TokenLimitField::MaxCompletionTokens => TokenLimitField::MaxTokens,
        };
        let patch =
          SettingsPatch { token_limit_field: Some(token_limit_field), ..Default::default() };
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
    if self.ui.storage_locked || self.host.home_menu_active() || self.host.transfer_active() {
      return;
    }
    let EditTarget::Setting(setting) = target else {
      self.ui.draft = value;
      self.ui.focused = "send".into();
      return;
    };
    if setting == Setting::UpdateRepository && !self.update_settings_editable() {
      return;
    }
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
      Setting::Thinking | Setting::TokenLimitField => return,
    }
    if self.mutate(Command::Settings { settings: patch }).is_some()
      && setting == Setting::UpdateRepository
    {
      self.invalidate_update_source();
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
    if self.ui.storage_locked || self.host.home_menu_active() {
      return;
    }
    if self.ui.page == Page::Transfer {
      self.stop_transfer();
    }
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
    if self.ui.page == Page::Transfer {
      self.dispatch(Action::TransferBack);
    } else if self.ui.page == Page::HomeMenu {
      if !self.host.home_menu_active() {
        self.dispatch(Action::HomeMenuBack);
      }
    } else if self.ui.page == Page::Updates {
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
    if self.host.home_menu_status().phase == HomeMenuPhase::Installing {
      self.ui.notice = "Installing the HOME Menu tile. Keep Wodex open until it finishes.".into();
      return;
    }
    self.stop_transfer();
    self.host.cancel_images();
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
    self.host.transfer_stop();
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
