//! In-app local translation through a self-hosted, LibreTranslate-compatible
//! service (default `http://127.0.0.1:5050`, e.g. LTEngine). Requests run on
//! background threads over raw HTTP/1.1 POSTs so the UI crate gains no new
//! dependencies.
//!
//! Vencord-parity surface:
//! - Inline translations rendered under messages in the timeline (toggle via
//!   the message Translate button; a detected-language caption is included).
//! - Auto-translate incoming messages (host-side, off by default).
//! - Auto-translate before send (host-side, off by default; the draft is
//!   replaced with the translation and then submitted).
//! - A settings window (chat-bar Globe button) with target languages and both
//!   automation toggles.
//!
//! Settings live in egui memory for the session and persist through the
//! device preferences row (see pply_preferences); no text leaves the machine
//! beyond the configured local service.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// A finished extension translation, ready to deliver back into the plugin
/// that requested it through a fresh panel invocation.
pub(crate) struct ExtensionTranslation {
	pub plugin: String,
	pub action: String,
	pub target: String,
	pub result: Result<Translation, String>,
}

struct PendingTranslation {
	plugin: String,
	action: String,
	target: String,
	outcome: Option<Result<Translation, String>>,
}

static PENDING: OnceLock<Mutex<Vec<PendingTranslation>>> = OnceLock::new();

fn pending() -> &'static Mutex<Vec<PendingTranslation>> {
	PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// Plain-English skip reason for the extension panel; the plugin surface has
/// no access to the host's localized labels.
fn extension_skip_reason(kind: SkipKind) -> String {
	match kind {
		SkipKind::LinksEmojiMentionsOnly => "Not translated: links, emoji and mentions only".into(),
		SkipKind::Credential => "Not translated: credential-like content".into(),
		SkipKind::EmailAddress => "Not translated: email address".into(),
	}
}
/// Host side of the extension `RequestTranslation` action: run the local
/// service call on a background thread and hand the result to the next frame's
/// extension pump, which re-invokes the plugin's panel action with it.
pub(crate) fn spawn_extension_translation(
	ctx: egui::Context,
	plugin: String,
	action: String,
	text: String,
	target: String,
) {
	if pending().lock().expect("local translate").len() >= 8 {
		return;
	}
	// Same policy as the native paths: content with nothing to translate (or
	// credential-shaped) never reaches the model; the plugin gets a note.
	if let Some(kind) = skip_kind(text.trim()) {
		pending()
			.lock()
			.expect("local translate")
			.push(PendingTranslation {
				plugin,
				action,
				target,
				outcome: Some(Err(extension_skip_reason(kind))),
			});
		ctx.request_repaint();
		return;
	}
	let record = PendingTranslation {
		plugin,
		action,
		target: target.clone(),
		outcome: None,
	};
	let index = {
		let mut list = pending().lock().expect("local translate");
		list.push(record);
		list.len() - 1
	};
	let authority = service_authority();
	let body = request_body(&text, &target);
	std::thread::spawn(move || {
		let result = translate_at(&authority, "/translate", &body);
		let repaint = ctx.clone();
		if let Ok(mut list) = pending().lock()
			&& let Some(record) = list.get_mut(index)
		{
			record.outcome = Some(result);
		}
		repaint.request_repaint();
	});
}

/// Drain every finished translation, dropping entries whose plugin vanished.
pub(crate) fn take_finished_translations() -> Vec<ExtensionTranslation> {
	let mut list = pending().lock().expect("local translate");
	let mut finished = Vec::new();
	let mut index = 0;
	while index < list.len() {
		if let Some(outcome) = list[index].outcome.take() {
			let record = list.remove(index);
			finished.push(ExtensionTranslation {
				plugin: record.plugin,
				action: record.action,
				target: record.target,
				result: outcome,
			});
		} else {
			index += 1;
		}
	}
	finished
}

/// Set `SEREIN_LOCAL_TRANSLATE=0` to disable the local service and keep the
/// legacy browser-link behavior for every translation click.
pub fn enabled() -> bool {
	if cfg!(test) {
		return false;
	}
	static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ENABLED.get_or_init(|| std::env::var("SEREIN_LOCAL_TRANSLATE").as_deref() != Ok("0"))
}

fn service_authority() -> String {
	static AUTHORITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
	AUTHORITY
		.get_or_init(|| {
			std::env::var("SEREIN_TRANSLATE_HOST").unwrap_or_else(|_| "127.0.0.1:5050".into())
		})
		.clone()
}

fn env_target() -> String {
	static TARGET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
	TARGET
		.get_or_init(|| std::env::var("SEREIN_TRANSLATE_TARGET").unwrap_or_else(|_| "en".into()))
		.clone()
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_MESSAGE_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
	pub(crate) text: String,
	pub(crate) detected_language: Option<String>,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Selectable target languages for the settings window.
pub const LANGUAGES: &[(&str, &str)] = &[
	("en", "English"),
	("th", "Thai"),
	("ja", "Japanese"),
	("ko", "Korean"),
	("zh", "Chinese"),
	("de", "German"),
	("es", "Spanish"),
	("fr", "French"),
	("pt", "Portuguese"),
	("ru", "Russian"),
	("vi", "Vietnamese"),
	("id", "Indonesian"),
	("ar", "Arabic"),
	("hi", "Hindi"),
];

/// Runtime translation settings; seeded from and persisted through the device
/// preferences row.
#[derive(Clone)]
pub struct Settings {
	/// Master switch: pause the whole feature from the chat-bar globe.
	pub translate_enabled: bool,
	pub incoming_target: String,
	pub outgoing_target: String,
	pub auto_incoming: bool,
	pub auto_outgoing: bool,
	/// When auto-outgoing is on: show the translation for confirmation in the
	/// composer instead of sending immediately.
	pub outgoing_preview: bool,
	/// When on, before-send translations include recent channel messages as
	/// context so pronouns and ambiguity resolve correctly. Slower.
	pub context_aware: bool,
	/// How many recent messages the context-aware mode includes.
	pub context_messages: usize,
}

/// Selectable context depths for the settings window, smallest first.
pub const CONTEXT_MESSAGE_CHOICES: &[usize] = &[2, 4, 6];

/// Nearest offered depth for a stale or out-of-range stored value.
fn clamp_context_messages(value: usize) -> usize {
	CONTEXT_MESSAGE_CHOICES
		.iter()
		.min_by_key(|&&choice| choice.abs_diff(value))
		.copied()
		.unwrap_or(4)
}

impl Default for Settings {
	fn default() -> Self {
		let target = env_target();
		Self {
			translate_enabled: true,
			incoming_target: target.clone(),
			outgoing_target: target,
			auto_incoming: false,
			auto_outgoing: false,
			outgoing_preview: false,
			context_aware: false,
			context_messages: 4,
		}
	}
}

fn with_settings<R>(ctx: &egui::Context, reader: impl FnOnce(&mut Settings) -> R) -> R {
	ctx.data_mut(|data| {
		#[expect(deprecated)]
		let state = data.get_temp_mut_or_insert_with(
			egui::Id::new("local-translate-settings"),
			Default::default,
		);
		reader(state)
	})
}

/// Snapshot of the current settings, with the context depth normalized to an
/// offered choice.
pub fn settings_snapshot(ctx: &egui::Context) -> Settings {
	with_settings(ctx, |settings| {
		settings.context_messages = clamp_context_messages(settings.context_messages);
		settings.clone()
	})
}

/// English display name for a service language code, or the code itself.
pub fn language_name(code: &str) -> &str {
	LANGUAGES
		.iter()
		.find(|(known, _)| *known == code)
		.map(|(_, name)| *name)
		.unwrap_or(code)
}

/// A stored target is kept only when it is still selectable, so a row edited
/// by hand or a language removed from the list falls back to the default.
fn known_target(code: &str, fallback: &str) -> String {
	if LANGUAGES.iter().any(|(known, _)| *known == code) {
		code.to_owned()
	} else {
		fallback.to_owned()
	}
}

fn preferences_from(settings: &Settings) -> model::TranslatePreferences {
	model::TranslatePreferences {
		enabled: settings.translate_enabled,
		incoming_target: settings.incoming_target.clone(),
		outgoing_target: settings.outgoing_target.clone(),
		auto_incoming: settings.auto_incoming,
		auto_outgoing: settings.auto_outgoing,
		outgoing_preview: settings.outgoing_preview,
		context_aware: settings.context_aware,
		context_messages: clamp_context_messages(settings.context_messages),
	}
}

/// Current settings shaped for the device preferences row.
pub fn preferences(ctx: &egui::Context) -> model::TranslatePreferences {
	preferences_from(&settings_snapshot(ctx))
}

/// Seed the session settings from the device preferences row once at startup
/// and return the effective values. Unselectable stored targets are replaced
/// with the default target and the context depth is clamped; None (an older
/// preferences row) keeps the in-memory defaults.
pub fn apply_preferences(
	ctx: &egui::Context,
	saved: Option<&model::TranslatePreferences>,
) -> model::TranslatePreferences {
	let effective = match saved {
		Some(saved) => {
			let defaults = Settings::default();
			Settings {
				translate_enabled: saved.enabled,
				incoming_target: known_target(&saved.incoming_target, &defaults.incoming_target),
				outgoing_target: known_target(&saved.outgoing_target, &defaults.outgoing_target),
				auto_incoming: saved.auto_incoming,
				auto_outgoing: saved.auto_outgoing,
				outgoing_preview: saved.outgoing_preview,
				context_aware: saved.context_aware,
				context_messages: clamp_context_messages(saved.context_messages),
			}
		}
		None => Settings::default(),
	};
	with_settings(ctx, |stored| *stored = effective.clone());
	preferences_from(&effective)
}

/// Set once per process when a translation came back already in the target
/// language, so the UI can explain the no-op instead of looking broken.
static SAME_LANGUAGE_NOTICED: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

fn mark_same_language(ctx: &egui::Context, detected: &str) {
	if !SAME_LANGUAGE_NOTICED.swap(true, std::sync::atomic::Ordering::Relaxed) {
		ctx.data_mut(|data| {
			#[expect(deprecated)]
			data.insert_temp(
				egui::Id::new("local-translate-same-language"),
				detected.to_owned(),
			);
		});
	}
}

// ---------------------------------------------------------------------------
// Skip classification: keep links, emoji, credentials and addresses out of
// the model entirely
// ---------------------------------------------------------------------------

/// Why a message is not worth sending to the translation model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
	/// URLs, mentions, channels and emoji only — no natural language remains.
	LinksEmojiMentionsOnly,
	/// High-confidence key or credential material; kept away from the model.
	Credential,
	/// A single bare email address; translation cannot change it.
	EmailAddress,
}

/// i18n key shown next to a message that was not translated.
pub fn skip_label(kind: SkipKind) -> &'static str {
	match kind {
		SkipKind::LinksEmojiMentionsOnly => "local-translate-skip-decorative",
		SkipKind::Credential => "local-translate-skip-credential",
		SkipKind::EmailAddress => "local-translate-skip-email",
	}
}

/// Classify `text` as not needing translation; None means send it to the
/// model. Credentials win over everything (privacy), then link/mention/
/// emoji-only messages, then bare addresses.
pub fn skip_kind(text: &str) -> Option<SkipKind> {
	let text = text.trim();
	if text.is_empty() {
		return None;
	}
	if looks_like_credential(text) {
		return Some(SkipKind::Credential);
	}
	if text.split_whitespace().all(is_non_verbal_token) {
		return Some(SkipKind::LinksEmojiMentionsOnly);
	}
	if is_bare_email(text) {
		return Some(SkipKind::EmailAddress);
	}
	None
}

fn is_non_verbal_token(raw: &str) -> bool {
	if raw.starts_with('<') && raw.ends_with('>') && raw.len() > 2 {
		let inner = &raw[1..raw.len() - 1];
		return inner.starts_with("http")
			|| inner.starts_with('@')
			|| inner.starts_with('#')
			|| inner.starts_with(':')
			|| inner.starts_with("a:");
	}
	let token = raw.trim_matches(|c: char| ".,!?;:'\"()*`~[]".contains(c));
	token.is_empty()
		|| is_url_token(token)
		|| token.starts_with('@')
		|| is_channel_token(token)
		|| token.chars().all(is_emoji_char)
}
fn is_url_token(token: &str) -> bool {
	let lower = token.to_ascii_lowercase();
	lower.starts_with("http://")
		|| lower.starts_with("https://")
		|| lower.starts_with("www.")
		|| lower.starts_with("discord.gg/")
		|| lower.starts_with("discord.com/invite/")
		|| is_bare_domain(token)
}
fn is_bare_domain(token: &str) -> bool {
	let Some((head, tail)) = token.split_once('.') else {
		return false;
	};
	!head.is_empty()
		&& !tail.is_empty()
		&& head.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
		&& tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
fn is_channel_token(token: &str) -> bool {
	match token.strip_prefix('#') {
		Some(rest) => {
			!rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
		}
		None => false,
	}
}
fn is_emoji_char(c: char) -> bool {
	matches!(c as u32,
		0x1F000..=0x1FAFF
		| 0x2600..=0x27BF
		| 0x2B00..=0x2BFF
		| 0xFE0F
		| 0x200D
		| 0x20E3
		| 0x00A9
		| 0x00AE
		| 0x203C
		| 0x2049
		| 0x2122)
}
fn is_bare_email(text: &str) -> bool {
	let Some((user, domain)) = text.split_once('@') else {
		return false;
	};
	!user.is_empty()
		&& domain.contains('.')
		&& !domain.starts_with('.')
		&& !domain.ends_with('.')
		&& !text.contains(char::is_whitespace)
		&& text
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || "._%+-@".contains(c))
}

/// Token prefixes that introduce an opaque key, with the minimum run length
/// of following key characters.
const KEY_PREFIXES: &[(&str, usize)] = &[
	("sk-", 16),
	("ghp_", 30),
	("ghu_", 30),
	("gho_", 30),
	("ghs_", 30),
	("github_pat_", 30),
	("xoxb-", 10),
	("xoxa-", 10),
	("xoxp-", 10),
	("xoxr-", 10),
	("xoxs-", 10),
	("aiza", 30),
];
/// Credential labels where a solid value after ':' or '=' is a secret.
const KEY_LABELS: &[&str] = &[
	"password", "passwd", "secret", "token", "api key", "apikey", "api_key", "api-key",
];

fn looks_like_credential(text: &str) -> bool {
	let lower = text.to_ascii_lowercase();
	if lower.contains("private key-----") {
		return true;
	}
	for (prefix, min) in KEY_PREFIXES {
		let mut search = 0;
		while let Some(found) = lower[search..].find(prefix) {
			let at = search + found;
			let boundary = text[..at]
				.chars()
				.next_back()
				.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'));
			let run = lower[at + prefix.len()..]
				.chars()
				.take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
				.count();
			if boundary && run >= *min {
				return true;
			}
			search = at + 1;
		}
	}
	for label in KEY_LABELS {
		let mut search = 0;
		while let Some(found) = lower[search..].find(label) {
			let mut rest = lower[search + found + label.len()..].chars().peekable();
			while matches!(rest.peek(), Some(' ')) {
				rest.next();
			}
			if matches!(rest.peek(), Some(':') | Some('=')) {
				rest.next();
				while matches!(rest.peek(), Some(' ')) {
					rest.next();
				}
				let value = rest.take_while(|c| !c.is_whitespace()).collect::<String>();
				if value.len() >= 8 {
					return true;
				}
			}
			search = search + found + 1;
		}
	}
	let mut search = 0;
	while let Some(found) = text[search..].find("AKIA") {
		let at = search + found;
		let boundary = text[..at]
			.chars()
			.next_back()
			.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'));
		let run = text[at + 4..]
			.chars()
			.take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
			.count();
		if boundary && run >= 16 {
			return true;
		}
		search = at + 1;
	}
	long_opaque_run(text)
}

/// Pure hex >= 40 or mixed-case base64 >= 48 standing as one token.
fn long_opaque_run(text: &str) -> bool {
	for token in text.split(|c: char| c.is_whitespace() || ".,;:!?()[]{}<>\"'`".contains(c)) {
		if token.len() >= 40 && token.chars().all(|c| c.is_ascii_hexdigit()) {
			return true;
		}
		if token.len() >= 48
			&& token
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
			&& token.chars().any(|c| c.is_ascii_lowercase())
			&& token.chars().any(|c| c.is_ascii_uppercase())
			&& token.chars().any(|c| c.is_ascii_digit())
		{
			return true;
		}
	}
	false
}
/// Take the pending same-language notice, if any; set at most once per run.
pub fn take_same_language_notice(ctx: &egui::Context) -> Option<String> {
	ctx.data_mut(|data| {
		#[expect(deprecated)]
		data.remove_temp::<String>(egui::Id::new("local-translate-same-language"))
	})
}

/// True when outgoing drafts should be translated before sending.
pub fn should_translate_send(ctx: &egui::Context) -> bool {
	let settings = settings_snapshot(ctx);
	enabled() && settings.translate_enabled && settings.auto_outgoing
}

/// True when before-send translations wait for a preview confirmation.
pub fn send_preview_enabled(ctx: &egui::Context) -> bool {
	let settings = settings_snapshot(ctx);
	enabled() && settings.auto_outgoing && settings.outgoing_preview
}

/// Whether the feature is active: the environment allows it and the chat-bar
/// master toggle is on.
pub fn active(ctx: &egui::Context) -> bool {
	settings_snapshot(ctx).translate_enabled
}

/// Flip the master translate switch (chat-bar globe click).
pub fn toggle_active(ctx: &egui::Context) {
	with_settings(ctx, |settings| {
		settings.translate_enabled = !settings.translate_enabled
	});
}
/// Open the settings window from the chat-bar button.
pub fn open_settings(ctx: &egui::Context) {
	ctx.data_mut(|data| {
		#[expect(deprecated)]
		data.insert_temp(egui::Id::new("local-translate-settings-open"), true);
	});
}

/// Draw the settings window when open. Hosted by the composer view.
pub fn show_settings(ctx: &egui::Context) {
	let open = ctx.data_mut(|data| {
		#[expect(deprecated)]
		*data.get_temp_mut_or_insert_with(egui::Id::new("local-translate-settings-open"), || false)
	});
	if !open {
		return;
	}
	let mut window_open = true;
	egui::Window::new(crate::i18n::translate("local-translate-settings-title"))
		.open(&mut window_open)
		.resizable(false)
		.min_width(340.0)
		.show(ctx, |ui| {
			let mut settings = settings_snapshot(ctx);
			ui.checkbox(
				&mut settings.translate_enabled,
				crate::i18n::translate("local-translate-settings-master"),
			);
			ui.add_enabled_ui(settings.translate_enabled, |ui| {
				ui.add_space(4.0);
				combo_target(
					ui,
					"local-translate-settings-incoming",
					&mut settings.incoming_target,
				);
				combo_target(
					ui,
					"local-translate-settings-outgoing",
					&mut settings.outgoing_target,
				);
				ui.add_space(6.0);
				ui.separator();
				ui.add_space(6.0);
				ui.checkbox(
					&mut settings.auto_incoming,
					crate::i18n::translate("local-translate-settings-auto-incoming"),
				);
				ui.checkbox(
					&mut settings.auto_outgoing,
					crate::i18n::translate("local-translate-settings-auto-outgoing"),
				);
				ui.add_enabled_ui(settings.auto_outgoing, |ui| {
					ui.checkbox(
						&mut settings.outgoing_preview,
						crate::i18n::translate("local-translate-settings-preview"),
					);
					ui.checkbox(
						&mut settings.context_aware,
						crate::i18n::translate("local-translate-settings-context"),
					);
					ui.add_enabled_ui(settings.context_aware, |ui| {
						ui.label(crate::i18n::translate(
							"local-translate-settings-context-depth",
						));
						let current = format!("{}", settings.context_messages);
						egui::ComboBox::from_id_salt("local-translate-context-depth")
							.selected_text(current)
							.show_ui(ui, |ui| {
								for &choice in CONTEXT_MESSAGE_CHOICES {
									ui.selectable_value(
										&mut settings.context_messages,
										choice,
										format!("{choice}"),
									);
								}
							});
					});
				});
				ui.add_space(6.0);
				let host = service_authority();
				ui.label(
					egui::RichText::new(crate::i18n::translate_args(
						"local-translate-settings-hint",
						&[("host", &host)],
					))
					.small()
					.color(ui.visuals().weak_text_color()),
				);
			});
			with_settings(ui.ctx(), |stored| *stored = settings);
		});
	if !window_open {
		ctx.data_mut(|data| {
			#[expect(deprecated)]
			data.insert_temp(egui::Id::new("local-translate-settings-open"), false);
		});
	}
}

fn combo_target(ui: &mut egui::Ui, label_key: &str, value: &mut String) {
	ui.label(crate::i18n::translate(label_key));
	let current = LANGUAGES
		.iter()
		.find(|(code, _)| *code == value)
		.map(|(_, name)| *name)
		.unwrap_or(value.as_str());
	egui::ComboBox::from_id_salt(label_key)
		.selected_text(current)
		.show_ui(ui, |ui| {
			for &(code, name) in LANGUAGES {
				ui.selectable_value(value, code.to_owned(), name);
			}
		});
}

// ---------------------------------------------------------------------------
// Inline timeline translations
// ---------------------------------------------------------------------------

/// State of one message's inline translation.
#[derive(Clone, Debug, PartialEq)]
pub enum InlineState {
	Pending,
	Done(Result<Translation, String>),
	/// Classified as not worth translating; never sent to the model.
	Skipped(SkipKind),
}

/// A renderable snapshot of one message's inline translation.
pub struct InlineEntry {
	pub state: InlineState,
	pub fallback: Option<(String, bool)>,
	/// Target language code requested for this entry, shown with the result.
	pub target: String,
}

struct SharedInline {
	state: Arc<Mutex<InlineState>>,
	fallback: Option<(String, bool)>,
	/// Target language code requested for this entry, shown with the result.
	target: String,
	/// Message text at translation time, for edit-staleness checks.
	original: String,
	/// Insertion order for oldest-first eviction of finished entries.
	sequence: u64,
}

static INLINE: OnceLock<Mutex<HashMap<u64, SharedInline>>> = OnceLock::new();

fn inline_map() -> &'static Mutex<HashMap<u64, SharedInline>> {
	INLINE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clamp_text(text: &str) -> &str {
	let mut cut = text.len();
	for (index, _) in text.char_indices() {
		if index > MAX_MESSAGE_BYTES {
			cut = index;
			break;
		}
	}
	&text[..cut]
}

/// Toggle the inline translation for `message_id`: showing → remove, missing
/// → request from the local service. Returns true when now translating.
pub fn toggle_inline(
	ctx: &egui::Context,
	message_id: u64,
	text: &str,
	fallback: Option<(String, bool)>,
) -> bool {
	let text = clamp_text(text.trim());
	if text.is_empty() {
		return false;
	}
	let mut map = inline_map().lock().expect("local translate");
	if map.remove(&message_id).is_some() {
		dismissed()
			.lock()
			.expect("local translate")
			.insert(message_id);
		return false;
	}
	drop(map);
	dismissed()
		.lock()
		.expect("local translate")
		.remove(&message_id);
	spawn_inline(
		ctx,
		message_id,
		text,
		&settings_snapshot(ctx).incoming_target,
		false,
		fallback,
	);
	true
}

/// Snapshot for timeline rendering, if this message has an entry.
pub fn inline_entry(message_id: u64) -> Option<InlineEntry> {
	let map = inline_map().lock().expect("local translate");
	map.get(&message_id).map(|shared| InlineEntry {
		state: match &*shared.state.lock().expect("local translate") {
			InlineState::Pending => InlineState::Pending,
			InlineState::Done(result) => InlineState::Done(result.clone()),
			InlineState::Skipped(kind) => InlineState::Skipped(*kind),
		},
		fallback: shared.fallback.clone(),
		target: shared.target.clone(),
	})
}

/// Remove the entry for `message_id` (dismiss button, stale text, or
/// same-language drop). Also unmarks the auto-request so the message can be
/// re-requested later (e.g. after an edit).
pub fn clear_inline(message_id: u64) {
	inline_map()
		.lock()
		.expect("local translate")
		.remove(&message_id);
	auto_requested()
		.lock()
		.expect("local translate")
		.remove(&message_id);
}

fn spawn_inline(
	ctx: &egui::Context,
	message_id: u64,
	text: &str,
	target: &str,
	drop_same_language: bool,
	fallback: Option<(String, bool)>,
) -> bool {
	// Auto-requests skip silently; a manual click keeps a visible entry so the
	// button never looks dead. Skipped entries never reach the model.
	let skipped = skip_kind(text);
	if skipped.is_some() && drop_same_language {
		return false;
	}
	let state = Arc::new(Mutex::new(match skipped {
		Some(kind) => InlineState::Skipped(kind),
		None => InlineState::Pending,
	}));
	if !reserve_slot(
		message_id,
		text,
		fallback,
		target.to_owned(),
		Arc::clone(&state),
	) {
		return false;
	}
	if skipped.is_none() {
		let authority = service_authority();
		let body = request_body(text, target);
		let target_owned = target.to_owned();
		let repaint = ctx.clone();
		std::thread::spawn(move || {
			let mut result = translate_at(&authority, "/translate", &body);
			// A result already in the target language is a no-op: auto-requests drop
			// it silently (the renderer removes empty errors) and the first one flags
			// the UI so a manual click that seems to do nothing gets explained.
			if let Ok(translation) = &result
				&& let Some(detected) = translation.detected_language.as_deref()
				&& detected.eq_ignore_ascii_case(&target_owned)
			{
				mark_same_language(&repaint, detected);
				if drop_same_language {
					result = Err(String::new());
				}
			}
			if let Ok(mut slot) = state.lock() {
				*slot = InlineState::Done(result);
			}
			repaint.request_repaint();
		});
	}
	true
}

/// Capacity of the inline translation map. Finished entries are evicted
/// oldest-first past this bound; when every slot is in flight, new requests
/// are refused until one finishes.
const INLINE_CAP: usize = 24;

fn reserve_slot(
	message_id: u64,

	text: &str,

	fallback: Option<(String, bool)>,

	target: String,

	state: Arc<Mutex<InlineState>>,
) -> bool {
	let mut map = inline_map().lock().expect("local translate");

	if map.contains_key(&message_id) {
		return false;
	}

	if map.len() >= INLINE_CAP {
		let oldest_done = map
			.iter()
			.filter(|(_, shared)| {
				matches!(
					&*shared.state.lock().expect("local translate"),
					InlineState::Done(_)
				)
			})
			.min_by_key(|(_, shared)| shared.sequence)
			.map(|(id, _)| *id);

		match oldest_done {
			Some(id) => {
				map.remove(&id);
			}

			None => return false,
		}
	}

	static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

	map.insert(
		message_id,
		SharedInline {
			state,

			fallback,

			target,

			original: text.to_owned(),

			sequence: SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
		},
	);

	true
}

/// True when the stored translation no longer matches the message's current
/// text (the message was edited after translation).
pub fn inline_stale(message_id: u64, current_text: &str) -> bool {
	let map = inline_map().lock().expect("local translate");

	map.get(&message_id)
		.is_some_and(|shared| shared.original != clamp_text(current_text.trim()))
}

// ---------------------------------------------------------------------------
// Auto-translate incoming messages
// ---------------------------------------------------------------------------

static AUTO_REQUESTED: OnceLock<Mutex<HashMap<u64, ()>>> = OnceLock::new();

fn auto_requested() -> &'static Mutex<HashMap<u64, ()>> {
	AUTO_REQUESTED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Message ids hidden by the reader; auto-translate never respawns these
/// until the translate button is pressed again.
static DISMISSED: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();

fn dismissed() -> &'static Mutex<HashSet<u64>> {
	DISMISSED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Whether the reader hid this message's translation.
#[cfg(test)]
pub(crate) fn is_dismissed(message_id: u64) -> bool {
	dismissed()
		.lock()
		.expect("local translate")
		.contains(&message_id)
}

/// Hide the inline translation for `message_id` and remember the choice:
/// auto-translate will not bring it back until the reader asks again.
pub fn dismiss_inline(message_id: u64) {
	inline_map()
		.lock()
		.expect("local translate")
		.remove(&message_id);

	auto_requested()
		.lock()
		.expect("local translate")
		.remove(&message_id);

	dismissed()
		.lock()
		.expect("local translate")
		.insert(message_id);
}

/// Request the inline translation for a newly rendered message when the
/// auto-incoming toggle is on. Own messages are skipped and each message is
/// only requested once per session.
pub fn auto_incoming(ctx: &egui::Context, message_id: u64, text: &str, own: bool) {
	if own || !enabled() {
		return;
	}
	if dismissed()
		.lock()
		.expect("local translate")
		.contains(&message_id)
	{
		return;
	}
	let settings = settings_snapshot(ctx);
	if !settings.translate_enabled || !settings.auto_incoming {
		return;
	}
	let text = clamp_text(text.trim());
	if text.is_empty() {
		return;
	}
	{
		let map = inline_map().lock().expect("local translate");
		if map.contains_key(&message_id) {
			return;
		}
	}
	{
		let mut requested = auto_requested().lock().expect("local translate");

		if requested.contains_key(&message_id) {
			return;
		}

		if requested.len() >= 4096 {
			requested.clear();
		}
	}

	if spawn_inline(ctx, message_id, text, &settings.incoming_target, true, None) {
		auto_requested()
			.lock()
			.expect("local translate")
			.insert(message_id, ());
	}
}

// ---------------------------------------------------------------------------
// Translate before send
// ---------------------------------------------------------------------------

struct SendJob {
	state: Arc<Mutex<Option<Result<Translation, String>>>>,
}

static SEND_SLOT: OnceLock<Mutex<Option<SendJob>>> = OnceLock::new();

fn send_slot() -> &'static Mutex<Option<SendJob>> {
	SEND_SLOT.get_or_init(|| Mutex::new(None))
}

/// Outcome of queueing a before-send translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
	Started,
	/// A job is already in flight; keep the draft and retry.
	Busy,
	/// Nothing worth translating — send the draft as it is.
	Skipped(SkipKind),
}

/// Queue `text` for translation before send. `Skipped` texts never reach the
/// model; the caller should send them unchanged. `Busy` means a job is
/// already in flight and the caller should keep the draft and retry.
pub fn queue_send(ctx: &egui::Context, text: &str, context: Option<Vec<String>>) -> SendOutcome {
	let text = clamp_text(text.trim());
	if let Some(kind) = skip_kind(text) {
		return SendOutcome::Skipped(kind);
	}
	if text.is_empty() {
		return SendOutcome::Busy;
	}
	let state = Arc::new(Mutex::new(None));
	{
		let mut slot = send_slot().lock().expect("local translate");
		if slot.is_some() {
			return SendOutcome::Busy;
		}
		*slot = Some(SendJob {
			state: Arc::clone(&state),
		});
	}
	let authority = service_authority();
	let target = settings_snapshot(ctx).outgoing_target;
	let (path, body) = match context.filter(|lines| !lines.is_empty()) {
		Some(lines) => (
			"/translate-context",
			request_body_context(text, &target, &lines),
		),
		None => ("/translate", request_body(text, &target)),
	};
	let repaint = ctx.clone();
	std::thread::spawn(move || {
		let result = translate_at(&authority, path, &body);
		if let Ok(mut slot) = state.lock() {
			*slot = Some(result);
		}
		repaint.request_repaint();
	});
	SendOutcome::Started
}

/// Take the finished before-send translation, if any. A none-result leaves
/// any in-flight job alone.
pub fn take_finished_send() -> Option<Result<Translation, String>> {
	let mut slot = send_slot().lock().expect("local translate");
	let job = slot.as_ref()?;
	let result = job.state.lock().expect("local translate").take();
	if result.is_some() {
		*slot = None;
	}
	result
}

// ---------------------------------------------------------------------------
// HTTP + JSON plumbing
// ---------------------------------------------------------------------------

fn request_body(text: &str, target: &str) -> String {
	format!(
		"{{\"q\":\"{}\",\"source\":\"auto\",\"target\":\"{}\"}}",
		json_escape(text),
		json_escape(target)
	)
}

fn request_body_context(text: &str, target: &str, context: &[String]) -> String {
	let mut out = String::from("{\"q\":\"");
	out.push_str(&json_escape(text));
	out.push_str("\",\"source\":\"auto\",\"target\":\"");
	out.push_str(&json_escape(target));
	out.push_str("\",\"context\":[");
	let mut first = true;
	for line in context {
		let trimmed = clamp_text(line.trim());
		if trimmed.is_empty() {
			continue;
		}
		if !first {
			out.push(',');
		}
		first = false;
		out.push('"');
		out.push_str(&json_escape(trimmed));
		out.push('"');
	}
	out.push_str("]}");
	out
}

fn translate_at(authority: &str, path: &str, body: &str) -> Result<Translation, String> {
	translate_http(authority, path, body)
}
fn json_escape(text: &str) -> String {
	let mut out = String::with_capacity(text.len() + 8);
	for character in text.chars() {
		match character {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => {
				out.push_str(&format!("\\u{:04x}", c as u32));
			}
			c => out.push(c),
		}
	}
	out
}

fn json_unescape(raw: &str) -> String {
	let mut out = String::with_capacity(raw.len());
	let mut characters = raw.chars();
	while let Some(character) = characters.next() {
		if character != '\\' {
			out.push(character);
			continue;
		}
		match characters.next() {
			Some('"') => out.push('"'),
			Some('\\') => out.push('\\'),
			Some('/') => out.push('/'),
			Some('n') => out.push('\n'),
			Some('r') => out.push('\r'),
			Some('t') => out.push('\t'),
			Some('b') => out.push('\u{8}'),
			Some('f') => out.push('\u{c}'),
			Some('u') => {
				let mut code = 0u32;
				for _ in 0..4 {
					match characters.next().and_then(|c| c.to_digit(16)) {
						Some(digit) => code = code * 16 + digit,
						None => return out,
					}
				}
				if (0xd800..0xdc00).contains(&code) {
					let mut peekable = characters.clone();
					if peekable.next() == Some('\\') && peekable.next() == Some('u') {
						let mut low = 0u32;
						let mut valid = true;
						for _ in 0..4 {
							match peekable.next().and_then(|c| c.to_digit(16)) {
								Some(digit) => low = low * 16 + digit,
								None => {
									valid = false;
									break;
								}
							}
						}
						if valid && (0xdc00..0xe000).contains(&low) {
							characters = peekable;
							code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
						}
					}
				}
				if let Some(character) = char::from_u32(code) {
					out.push(character);
				}
			}
			Some(other) => out.push(other),
			None => break,
		}
	}
	out
}

/// Read one JSON string value for `key` from a flat response object.
fn json_string_value(body: &str, key: &str) -> Option<String> {
	let needle = format!("\"{key}\"");
	let key_index = body.find(&needle)?;
	let rest = &body[key_index + needle.len()..];
	let colon = rest.find(':')?;
	let rest = &rest[colon + 1..];
	let start = rest.find('"')?;
	let rest = &rest[start + 1..];
	let mut escaped = false;
	for (offset, character) in rest.char_indices() {
		if escaped {
			escaped = false;
		} else if character == '\\' {
			escaped = true;
		} else if character == '"' {
			return Some(json_unescape(&rest[..offset]));
		}
	}
	None
}

fn translate_http(authority: &str, path: &str, body: &str) -> Result<Translation, String> {
	let mut stream = TcpStream::connect(authority).map_err(|error| {
		format!(
			"{} ({error})",
			crate::i18n::translate("local-translate-unreachable")
		)
	})?;
	stream
		.set_read_timeout(Some(READ_TIMEOUT))
		.map_err(|error| error.to_string())?;
	stream
		.set_write_timeout(Some(CONNECT_TIMEOUT))
		.map_err(|error| error.to_string())?;
	let request = format!(
		"POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
		body.len()
	);
	stream
		.write_all(request.as_bytes())
		.map_err(|error| error.to_string())?;
	let mut response = Vec::new();
	stream
		.read_to_end(&mut response)
		.map_err(|error| error.to_string())?;
	let response = String::from_utf8_lossy(&response);
	if !(response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")) {
		return Err(response
			.split_once("\r\n")
			.map(|(line, _)| line.to_owned())
			.unwrap_or_else(|| crate::i18n::translate("local-translate-failed")));
	}
	let payload = response
		.split_once("\r\n\r\n")
		.map(|(_, payload)| payload)
		.unwrap_or(&response);
	let text = json_string_value(payload, "translatedText")
		.ok_or_else(|| crate::i18n::translate("local-translate-failed"))?;
	Ok(Translation {
		text,
		detected_language: json_string_value(payload, "language"),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The inline map is process-global; serialize tests that mutate it.
	static TEST_LOCK: Mutex<()> = Mutex::new(());

	#[test]
	fn reserve_slot_evicts_the_oldest_finished_entry_when_full() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		for id in 0..INLINE_CAP as u64 {
			assert!(reserve_slot(
				id,
				"text",
				None,
				"en".into(),
				Arc::new(Mutex::new(InlineState::Done(Ok(Translation {
					text: "done".into(),
					detected_language: None,
				})))),
			));
		}
		assert!(reserve_slot(
			999,
			"text",
			None,
			"en".into(),
			Arc::new(Mutex::new(InlineState::Pending)),
		));
		let guard = map.lock().expect("local translate");
		assert!(
			!guard.contains_key(&0),
			"oldest finished entry must be evicted"
		);
		assert!(guard.contains_key(&999), "new entry must be stored");
		assert_eq!(guard.len(), INLINE_CAP);
		drop(guard);
		map.lock().expect("local translate").clear();
	}

	#[test]
	fn reserve_slot_refuses_when_every_slot_is_in_flight() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		for id in 0..INLINE_CAP as u64 {
			assert!(reserve_slot(
				id,
				"text",
				None,
				"en".into(),
				Arc::new(Mutex::new(InlineState::Pending))
			));
		}
		assert!(
			!reserve_slot(
				999,
				"text",
				None,
				"en".into(),
				Arc::new(Mutex::new(InlineState::Pending)),
			),
			"all-pending map must refuse new work"
		);
		map.lock().expect("local translate").clear();
	}

	#[test]
	fn inline_stale_reports_edited_messages() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		assert!(reserve_slot(
			7,
			"original text",
			None,
			"en".into(),
			Arc::new(Mutex::new(InlineState::Pending)),
		));
		assert!(!inline_stale(7, "original text"));
		assert!(
			!inline_stale(7, "original text\n"),
			"trailing whitespace is not an edit"
		);
		assert!(
			inline_stale(7, "edited text"),
			"edited message must be stale"
		);
		assert!(!inline_stale(8, "anything"), "missing entry is not stale");
		map.lock().expect("local translate").clear();
	}

	#[test]
	fn clamp_context_messages_picks_the_nearest_offered_depth() {
		assert_eq!(clamp_context_messages(2), 2);
		assert_eq!(clamp_context_messages(3), 2);
		assert_eq!(clamp_context_messages(0), 2);
		assert_eq!(clamp_context_messages(4), 4);
		assert_eq!(clamp_context_messages(5), 4);
		assert_eq!(clamp_context_messages(6), 6);
		assert_eq!(clamp_context_messages(99), 6);
	}

	#[test]
	fn dismiss_blocks_until_toggled_again() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());

		let ctx = egui::Context::default();

		inline_map().lock().expect("local translate").clear();

		dismiss_inline(90_001);

		assert!(is_dismissed(90_001));

		assert!(
			toggle_inline(&ctx, 90_001, "hello world", None),
			"an explicit re-request must clear the dismissed mark"
		);

		assert!(!is_dismissed(90_001));

		assert!(inline_entry(90_001).is_some());

		assert!(!toggle_inline(&ctx, 90_001, "hello world", None));

		assert!(inline_entry(90_001).is_none());

		assert!(is_dismissed(90_001));

		inline_map().lock().expect("local translate").clear();

		dismissed().lock().expect("local translate").clear();
	}

	#[test]
	fn apply_preferences_validates_stored_targets_and_depth() {
		let ctx = egui::Context::default();
		let saved = model::TranslatePreferences {
			enabled: true,
			incoming_target: "xx".into(),
			outgoing_target: "th".into(),
			auto_incoming: true,
			outgoing_preview: true,
			context_messages: 99,
			..model::TranslatePreferences::default()
		};
		let effective = apply_preferences(&ctx, Some(&saved));
		let snapshot = settings_snapshot(&ctx);
		assert_eq!(snapshot.outgoing_target, "th");
		assert_eq!(snapshot.context_messages, 6);
		assert!(snapshot.auto_incoming && snapshot.outgoing_preview);
		// An unselectable stored target falls back to the default, never "xx".
		assert_ne!(snapshot.incoming_target, "xx");
		assert_eq!(effective, preferences(&ctx));
	}

	#[test]
	fn apply_preferences_round_trips_saved_settings() {
		let ctx = egui::Context::default();
		let saved = model::TranslatePreferences {
			enabled: true,
			incoming_target: "th".into(),
			outgoing_target: "ko".into(),
			auto_incoming: true,
			auto_outgoing: true,
			outgoing_preview: false,
			context_aware: true,
			context_messages: 2,
		};
		apply_preferences(&ctx, Some(&saved));
		assert_eq!(preferences(&ctx), saved);
	}

	#[test]
	fn same_language_notice_is_taken_once() {
		let ctx = egui::Context::default();
		mark_same_language(&ctx, "en");
		assert_eq!(take_same_language_notice(&ctx), Some("en".to_owned()));
		assert_eq!(take_same_language_notice(&ctx), None);
	}

	#[test]
	fn language_name_falls_back_to_the_code() {
		assert_eq!(language_name("th"), "Thai");
		assert_eq!(language_name("zz"), "zz");
	}
	#[test]
	fn skip_kind_flags_link_mention_and_emoji_only_text() {
		assert_eq!(
			skip_kind("https://example.com/some/path?x=1"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
		assert_eq!(
			skip_kind("www.google.com"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
		assert_eq!(
			skip_kind("<:kekw:111111111111111111> \u{1F525}\u{1F525} https://cdn.example/x.png"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
		assert_eq!(
			skip_kind("@user <@123456789> @here"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
		assert_eq!(
			skip_kind("discord.gg/abc123"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
		assert_eq!(
			skip_kind("\u{1F525} \u{2764}\u{FE0F} \u{1F602} <a:dance:999999999999999999>"),
			Some(SkipKind::LinksEmojiMentionsOnly)
		);
	}

	#[test]
	fn skip_kind_keeps_normal_text_with_links_inside() {
		assert_eq!(skip_kind("check this https://example.com please"), None);
		assert_eq!(skip_kind("hello how are you"), None);
		assert_eq!(skip_kind("pass the salt please"), None);
		assert_eq!(skip_kind("email me at bob@example.com later"), None);
	}

	#[test]
	fn skip_kind_flags_credential_shapes() {
		assert_eq!(
			skip_kind("sk-abc123def456ghi789jklmno"),
			Some(SkipKind::Credential)
		);
		assert_eq!(
			skip_kind("here is my token ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH1234"),
			Some(SkipKind::Credential)
		);
		assert_eq!(
			skip_kind("AKIAIOSFODNN7EXAMPLE"),
			Some(SkipKind::Credential)
		);
		assert_eq!(
			skip_kind("-----BEGIN PRIVATE KEY-----"),
			Some(SkipKind::Credential)
		);
		assert_eq!(
			skip_kind("password: hunter2hunter2"),
			Some(SkipKind::Credential)
		);
		assert_eq!(
			skip_kind(&"a".repeat(41).replace('a', "f")),
			Some(SkipKind::Credential)
		);
		assert_eq!(skip_kind("pass: the"), None);
	}

	#[test]
	fn skip_kind_flags_bare_email_addresses() {
		assert_eq!(
			skip_kind("someone@example.com"),
			Some(SkipKind::EmailAddress)
		);
		assert_eq!(
			skip_kind("first.last+tag@mail.co.uk"),
			Some(SkipKind::EmailAddress)
		);
		assert_eq!(skip_kind("my mail: someone@example.com"), None);
	}

	#[test]
	fn manual_skip_creates_a_skipped_entry_without_calling_the_engine() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		inline_map().lock().expect("local translate").clear();
		dismissed().lock().expect("local translate").clear();
		let ctx = egui::Context::default();
		assert!(toggle_inline(
			&ctx,
			701,
			"https://example.com/only-a-link",
			None,
		));
		let entry = inline_entry(701).expect("manual skip keeps an entry");
		assert_eq!(
			entry.state,
			InlineState::Skipped(SkipKind::LinksEmojiMentionsOnly)
		);
		clear_inline(701);
	}
	#[test]
	fn extension_requests_skip_non_translatable_text() {
		let _guard = TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		pending().lock().expect("local translate").clear();
		let ctx = egui::Context::default();
		spawn_extension_translation(
			ctx.clone(),
			"plugin".into(),
			"action".into(),
			"https://example.com/only".into(),
			"th".into(),
		);
		let finished = take_finished_translations();
		assert_eq!(finished.len(), 1);
		let err = finished[0].result.clone().unwrap_err();
		assert!(err.contains("link"), "reason should mention links: {err}");
		assert!(take_finished_translations().is_empty());

		spawn_extension_translation(
			ctx,
			"plugin".into(),
			"action".into(),
			"password: hunter2hunter2".into(),
			"th".into(),
		);
		let finished = take_finished_translations();
		assert_eq!(finished.len(), 1);
		let err = finished[0].result.clone().unwrap_err();
		assert!(
			err.contains("credential"),
			"reason should mention credentials: {err}"
		);
		pending().lock().expect("local translate").clear();
	}
}
