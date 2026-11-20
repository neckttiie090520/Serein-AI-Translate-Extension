/// Device-local settings for the in-app local translation feature; the codes
/// are service language codes. Targets are validated against the selectable
/// language list when the settings are restored, so a row edited by hand or a
/// language removed from the list falls back to the default target.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TranslatePreferences {
	/// Master switch mirrored from the chat-bar globe; off pauses the whole
	/// feature without touching the individual target settings.
	pub enabled: bool,
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
impl Default for TranslatePreferences {
	fn default() -> Self {
		Self {
			enabled: true,
			incoming_target: "en".into(),
			outgoing_target: "en".into(),
			auto_incoming: false,
			auto_outgoing: false,
			outgoing_preview: false,
			context_aware: false,
			context_messages: 4,
		}
	}
}
impl TranslatePreferences {
	/// Shape guard for the bounded preferences row; language-list membership
	/// is checked separately where the list lives.
	pub fn is_valid(&self) -> bool {
		let code = |value: &str| {
			(1..=8).contains(&value.len())
				&& value
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
		};
		code(&self.incoming_target)
			&& code(&self.outgoing_target)
			&& (1..=100).contains(&self.context_messages)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_are_english_and_valid() {
		let defaults = TranslatePreferences::default();
		assert_eq!(defaults.incoming_target, "en");
		assert_eq!(defaults.outgoing_target, "en");
		assert_eq!(defaults.context_messages, 4);
		assert!(!defaults.auto_incoming && !defaults.auto_outgoing);
		assert!(!defaults.outgoing_preview && !defaults.context_aware);
		assert!(defaults.enabled);
		assert!(defaults.is_valid());
	}

	#[test]
	fn rejects_malformed_codes_and_depths() {
		let mut saved = TranslatePreferences::default();
		for invalid in ["", "ENGLISH", "abcdefghi", "th;drop table"] {
			saved.incoming_target = invalid.into();
			assert!(!saved.is_valid(), "{invalid} must not validate");
		}
		saved.incoming_target = "en".into();
		saved.context_messages = 0;
		assert!(!saved.is_valid());
		saved.context_messages = 101;
		assert!(!saved.is_valid());
		saved.context_messages = 6;
		assert!(saved.is_valid());
	}
}
