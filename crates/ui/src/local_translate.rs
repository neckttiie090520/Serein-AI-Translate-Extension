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
//! Settings live in egui memory (persisted by egui across sessions); nothing
//! is written to disk by this module and no text leaves the machine beyond
//! the configured local service.

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
		if let Ok(mut list) = pending().lock() {
			if let Some(record) = list.get_mut(index) {
				record.outcome = Some(result);
			}
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

#[derive(Clone)]
pub(crate) struct Translation {
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

/// Runtime translation settings, persisted by egui across sessions.
#[derive(Clone)]
pub struct Settings {
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
}

impl Default for Settings {
	fn default() -> Self {
		let target = env_target();
		Self {
			incoming_target: target.clone(),
			outgoing_target: target,
			auto_incoming: false,
			auto_outgoing: false,
			outgoing_preview: false,
			context_aware: false,
		}
	}
}

fn with_settings<R>(ctx: &egui::Context, reader: impl FnOnce(&mut Settings) -> R) -> R {
	ctx.data_mut(|data| {
		#[expect(deprecated)]
		let state =
			data.get_temp_mut_or_insert_with(egui::Id::new("local-translate-settings"), Default::default);
		reader(state)
	})
}

/// Snapshot of the current settings.
pub fn settings_snapshot(ctx: &egui::Context) -> Settings {
	with_settings(ctx, |settings| settings.clone())
}

/// True when outgoing drafts should be translated before sending.
pub fn should_translate_send(ctx: &egui::Context) -> bool {
	enabled() && settings_snapshot(ctx).auto_outgoing
}

/// True when before-send translations wait for a preview confirmation.
pub fn send_preview_enabled(ctx: &egui::Context) -> bool {
	let settings = settings_snapshot(ctx);
	enabled() && settings.auto_outgoing && settings.outgoing_preview
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
			ui.add_space(2.0);
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
pub(crate) enum InlineState {
	Pending,
	Done(Result<Translation, String>),
}

/// A renderable snapshot of one message's inline translation.
pub(crate) struct InlineEntry {
	pub state: InlineState,
	pub fallback: Option<(String, bool)>,
}

struct SharedInline {
	state: Arc<Mutex<InlineState>>,
	fallback: Option<(String, bool)>,
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
		dismissed().lock().expect("local translate").insert(message_id);
		return false;
	}
	drop(map);
	dismissed().lock().expect("local translate").remove(&message_id);
	spawn_inline(ctx, message_id, text, &settings_snapshot(ctx).incoming_target, false, fallback);
	true
}

/// Snapshot for timeline rendering, if this message has an entry.
pub fn inline_entry(message_id: u64) -> Option<InlineEntry> {
	let map = inline_map().lock().expect("local translate");
	map.get(&message_id).map(|shared| InlineEntry {
		state: match &*shared.state.lock().expect("local translate") {
			InlineState::Pending => InlineState::Pending,
			InlineState::Done(result) => InlineState::Done(result.clone()),
		},
		fallback: shared.fallback.clone(),
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
	let state = Arc::new(Mutex::new(InlineState::Pending));
	if !reserve_slot(message_id, text, fallback, Arc::clone(&state)) {
		return false;
	}
	let authority = service_authority();
	let body = request_body(text, target);
	let target_owned = target.to_owned();
	let drop_same_language = drop_same_language;
	let repaint = ctx.clone();
	std::thread::spawn(move || {
		let mut result = translate_at(&authority, "/translate", &body);
		// Messages already in the target language are dropped silently: the
		// renderer removes entries whose error string is empty.
		if drop_same_language && let Ok(translation) = &result
			&& translation
				.detected_language
				.as_deref()
				.is_some_and(|detected| detected.eq_ignore_ascii_case(&target_owned))
		{
			result = Err(String::new());
		}
		if let Ok(mut slot) = state.lock() {
			*slot = InlineState::Done(result);
		}
		repaint.request_repaint();
	});
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
				
matches!(&*shared.state.lock().expect("local translate"), InlineState::Done(_))
			
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
	
dismissed().lock().expect("local translate").insert(message_id);
}

/// Whether the reader hid this message's translation.


/// Request the inline translation for a newly rendered message when the
/// auto-incoming toggle is on. Own messages are skipped and each message is
/// only requested once per session.
pub fn auto_incoming(ctx: &egui::Context, message_id: u64, text: &str, own: bool) {
	if own || !enabled() {
		return;
	}
	if dismissed().lock().expect("local translate").contains(&message_id) { 		return; 	}
	let settings = settings_snapshot(ctx);
	if !settings.auto_incoming {
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

/// Queue `text` for translation before send. Returns false when a job is
/// already in flight (or the text is empty); the caller should then keep the
/// draft and retry submitting later.
pub fn queue_send(ctx: &egui::Context, text: &str, context: Option<Vec<String>>) -> bool {
	let text = clamp_text(text.trim());
	if text.is_empty() {
		return false;
	}
	let state = Arc::new(Mutex::new(None));
	{
		let mut slot = send_slot().lock().expect("local translate");
		if slot.is_some() {
			return false;
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
		let result = translate_at(&authority, &path, &body);
		if let Ok(mut slot) = state.lock() {
			*slot = Some(result);
		}
		repaint.request_repaint();
	});
	true
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
		let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		for id in 0..INLINE_CAP as u64 {
			assert!(reserve_slot(
				id,
				"text",
				None,
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
			Arc::new(Mutex::new(InlineState::Pending)),
		));
		let guard = map.lock().expect("local translate");
		assert!(!guard.contains_key(&0), "oldest finished entry must be evicted");
		assert!(guard.contains_key(&999), "new entry must be stored");
		assert_eq!(guard.len(), INLINE_CAP);
		drop(guard);
		map.lock().expect("local translate").clear();
	}

	#[test]
	fn reserve_slot_refuses_when_every_slot_is_in_flight() {
		let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		for id in 0..INLINE_CAP as u64 {
			assert!(reserve_slot(id, "text", None, Arc::new(Mutex::new(InlineState::Pending))));
		}
		assert!(
			!reserve_slot(
				999,
				"text",
				None,
				Arc::new(Mutex::new(InlineState::Pending)),
			),
			"all-pending map must refuse new work"
		);
		map.lock().expect("local translate").clear();
	}

	#[test]
	fn inline_stale_reports_edited_messages() {
		let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		let map = inline_map();
		map.lock().expect("local translate").clear();
		assert!(reserve_slot(
			7,
			"original text",
			None,
			Arc::new(Mutex::new(InlineState::Pending)),
		));
		assert!(!inline_stale(7, "original text"));
	assert!(!inline_stale(7, "original text\n"), "trailing whitespace is not an edit");
		assert!(inline_stale(7, "edited text"), "edited message must be stale");
		assert!(!inline_stale(8, "anything"), "missing entry is not stale");
		map.lock().expect("local translate").clear();
	}

	
#[test]
	
fn dismiss_blocks_until_toggled_again() {
		
let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		
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
}
