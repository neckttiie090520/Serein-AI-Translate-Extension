//! Local Translate: translates a selected message or the composer draft
//! through a self-hosted LibreTranslate-compatible service (for example
//! LTEngine on 127.0.0.1:5050). The plugin never opens a socket itself: it
//! proposes the host-mediated `RequestTranslation` action, Serein performs the
//! local call after Apply, and the finished translation is delivered back into
//! this panel. Nothing is sent to any remote service.

use serein_extension_sdk::{AppAction, AppInvocation, AppOutput, Element, HostEffect, Output};

const LANGUAGES: &[(&str, &str)] = &[
	("en", "English"),
	("th", "Thai"),
	("ja", "Japanese"),
	("ko", "Korean"),
	("zh-Hans", "Chinese (simplified)"),
	("zh-Hant", "Chinese (traditional)"),
	("de", "German"),
	("fr", "French"),
	("es", "Spanish"),
	("ru", "Russian"),
	("pt-BR", "Portuguese (Brazil)"),
	("vi", "Vietnamese"),
	("id", "Indonesian"),
];

const DEFAULT_TARGET: &str = "en";

fn language_options() -> Vec<String> {
	LANGUAGES
		.iter()
		.map(|(code, name)| format!("{code} — {name}"))
		.collect()
}

/// The stored preference is the bare language code; the dropdown shows
/// "code — name" rows, so map in both directions.
fn code_from_option(option: &str) -> String {
	option
		.split_once(" — ")
		.map_or_else(|| option.to_owned(), |(code, _)| code.to_owned())
}

fn target_language(input: &AppInvocation) -> String {
	if let Some(value) = input.invocation.value("target") {
		let code = code_from_option(value);
		if LANGUAGES.iter().any(|(known, _)| *known == code) {
			return code;
		}
	}
	let saved = input
		.invocation
		.storage
		.as_deref()
		.and_then(|stored| serde_json::from_str::<Storage>(stored).ok())
		.filter(|saved| LANGUAGES.iter().any(|(known, _)| *known == saved.target));
	if let Some(saved) = saved {
		return saved.target;
	}
	DEFAULT_TARGET.into()
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Storage {
	target: String,
}

fn storage_output(target: &str) -> Option<String> {
	serde_json::to_string(&Storage {
		target: target.into(),
	})
	.ok()
}

fn heading(text: &str) -> Element {
	Element::Heading { text: text.into() }
}

fn text(text: String) -> Element {
	Element::Text { text }
}

fn preview(full: &str) -> String {
	let mut out: String = full.chars().take(240).collect();
	if full.chars().count() > 240 {
		out.push('…');
	}
	out
}

fn proposal_panel(source: &str, draft: bool, target: &str) -> AppOutput {
	let selected = LANGUAGES
		.iter()
		.find(|(code, _)| *code == target)
		.map_or(DEFAULT_TARGET, |(code, _)| *code);
	let option = LANGUAGES
		.iter()
		.find(|(code, _)| *code == selected)
		.map_or_else(
			|| format!("{selected} — {selected}"),
			|(_, name)| format!("{selected} — {name}"),
		);
	let mut panel = vec![
		heading("Local Translate"),
		text(preview(source)),
		Element::Separator,
		Element::Select {
			id: "target".into(),
			label: "Translate into".into(),
			options: language_options(),
			value: option,
		},
		Element::Button {
			id: "request".into(),
			label: if draft {
				"Translate the draft locally".into()
			} else {
				"Translate this message locally".into()
			},
		},
		text(
			"Serein performs the request to the configured local service after Apply. \
			The result opens back in this panel."
				.into(),
		),
	];
	if draft {
		panel.insert(
			1,
			text("The composer draft is translated; the result can be copied back.".into()),
		);
	}
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

fn result_panel(
	source: &str,
	translation: &str,
	detected: Option<&str>,
	error: Option<&str>,
	target: &str,
) -> AppOutput {
	let mut panel = vec![
		heading("Local Translate"),
		text(preview(source)),
		Element::Separator,
	];
	if let Some(error) = error {
		panel.push(text(format!("The local service failed: {error}")));
		panel.push(text(
			"Check that the local translation service is running, then try again.".into(),
		));
	} else {
		panel.push(text(translation.to_owned()));
		if let Some(detected) = detected {
			panel.push(text(format!("Detected source language: {detected}")));
		}
		panel.push(text(format!("Translated into {target}.")));
		panel.push(Element::Button {
			id: "copy".into(),
			label: "Copy the translation".into(),
		});
	}
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let target = target_language(&input);
	match input.invocation.action.as_str() {
		"translate" | "translate-draft" => {
			let source = if input.invocation.action == "translate" {
				input.invocation.selected_message.clone()
			} else {
				input.invocation.composer.clone()
			};
			let Some(source) = source.filter(|source| !source.trim().is_empty()) else {
				return AppOutput {
					output: Output {
						panel: vec![
							heading("Local Translate"),
							text("Nothing to translate: the message or draft is empty.".into()),
						],
						..Default::default()
					},
					..Default::default()
				};
			};
			// The finished translation is delivered back into the panel action.
			if let Some(translation) = input.invocation.value("translation") {
				return result_panel(
					&source,
					translation,
					input.invocation.value("detected"),
					input.invocation.value("error"),
					input.invocation.value("target").unwrap_or(&target),
				);
			}
			if input.invocation.value("request").is_some() {
				return AppOutput {
					output: Output {
						panel: vec![
							heading("Local Translate"),
							text(format!(
								"Requesting a {} translation through the local service…",
								input.invocation.value("target").unwrap_or(&target)
							)),
						],
						..Default::default()
					},
					effects: vec![HostEffect::AppAction {
						action: AppAction::RequestTranslation {
							text: source,
							target,
						},
					}],
				};
			}
			proposal_panel(
				&source,
				input.invocation.action == "translate-draft",
				&target,
			)
		}
		"panel" => {
			// Delivered translation results arrive on the panel action with values.
			if let Some(translation) = input.invocation.value("translation") {
				let mut output = result_panel(
					input.invocation.value("source").unwrap_or(""),
					translation,
					input.invocation.value("detected"),
					input.invocation.value("error"),
					input.invocation.value("target").unwrap_or(&target),
				);
				if input.invocation.value("copy").is_some() {
					output.effects = vec![HostEffect::CopyText {
						text: translation.to_owned(),
					}];
				}
				return output;
			}
			if let Some(error) = input.invocation.value("error") {
				return result_panel(
					input.invocation.value("source").unwrap_or(""),
					"",
					None,
					Some(error),
					input.invocation.value("target").unwrap_or(&target),
				);
			}
			let selected = LANGUAGES
				.iter()
				.find(|(code, _)| *code == target)
				.map_or_else(
					|| format!("{target} — {target}"),
					|(_, name)| format!("{target} — {name}"),
				);
			AppOutput {
				output: Output {
					panel: vec![
						heading("Local Translate"),
						text(
							"Use a message's Translate action or the composer tool to translate \
							through the configured local service. The preference below is saved \
							for this account."
								.into(),
						),
						Element::Select {
							id: "target".into(),
							label: "Preferred target language".into(),
							options: language_options(),
							value: selected,
						},
					],
					storage: storage_output(&target),
					..Default::default()
				},
				..Default::default()
			}
		}
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::dispatch_typed;

	fn invocation(action: &str) -> AppInvocation {
		serde_json::from_value(serde_json::json!({
			"action": action,
			"values": {},
			"selected_message": null,
			"composer": null,
			"storage": null
		}))
		.unwrap()
	}

	#[test]
	fn message_action_proposes_local_translation() {
		let mut input = invocation("translate");
		input.invocation.selected_message = Some("Hello there".into());
		let output = handle(input);
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text == "Hello there"
		)));
		assert!(output.effects.is_empty());
	}

	#[test]
	fn request_button_proposes_host_translation_effect() {
		let mut input = invocation("translate");
		input.invocation.selected_message = Some("General Kenobi".into());
		input.invocation.values.insert("request".into(), "1".into());
		input
			.invocation
			.values
			.insert("target".into(), "th — Thai".into());
		let output = handle(input);
		assert!(matches!(
			&output.effects[..],
			[HostEffect::AppAction {
				action: AppAction::RequestTranslation { text, target }
			}] if text == "General Kenobi" && target == "th"
		));
	}

	#[test]
	fn delivered_translation_renders_result_and_copy() {
		let mut input = invocation("panel");
		input
			.invocation
			.values
			.insert("translation".into(), "สวัสดี".into());
		input
			.invocation
			.values
			.insert("source".into(), "Hello".into());
		input
			.invocation
			.values
			.insert("detected".into(), "English".into());
		input.invocation.values.insert("target".into(), "th".into());
		let output = handle(input.clone());
		assert!(
			output
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Text { text } if text == "สวัสดี"))
		);
		assert!(output.effects.is_empty());
		input.invocation.values.insert("copy".into(), "1".into());
		let output = handle(input);
		assert!(matches!(
			&output.effects[..],
			[HostEffect::CopyText { text }] if text == "สวัสดี"
		));
	}

	#[test]
	fn delivered_error_renders_guidance() {
		let mut input = invocation("panel");
		input
			.invocation
			.values
			.insert("error".into(), "connection refused".into());
		input.invocation.values.insert("source".into(), "Hi".into());
		let output = handle(input);
		assert!(output.output.panel.iter().any(
			|element| matches!(element, Element::Text { text } if text.contains("connection refused"))
		));
		assert!(output.effects.is_empty());
	}

	#[test]
	fn settings_panel_persists_target_preference() {
		let output = handle(invocation("panel"));
		let storage = output.output.storage.expect("storage output");
		let stored: Storage = serde_json::from_str(&storage).unwrap();
		assert_eq!(stored.target, "en");
	}

	#[test]
	fn stored_preference_is_used_and_invalid_falls_back() {
		let mut input = invocation("translate");
		input.invocation.selected_message = Some("Bonjour".into());
		input.invocation.storage = Some(r#"{"target":"th"}"#.into());
		input.invocation.values.insert("request".into(), "1".into());
		let output = handle(input.clone());
		assert!(matches!(
			&output.effects[..],
			[HostEffect::AppAction {
				action: AppAction::RequestTranslation { target, .. }
			}] if target == "th"
		));
		input.invocation.storage = Some(r#"{"target":"klingon"}"#.into());
		let output = handle(input);
		assert!(matches!(
			&output.effects[..],
			[HostEffect::AppAction {
				action: AppAction::RequestTranslation { target, .. }
			}] if target == "en"
		));
	}

	#[test]
	fn composer_tool_reads_draft() {
		let mut input = invocation("translate-draft");
		input.invocation.composer = Some("good morning everyone".into());
		let output = handle(input);
		assert!(output.output.panel.iter().any(
			|element| matches!(element, Element::Text { text } if text == "good morning everyone")
		));
	}

	#[test]
	fn empty_sources_explain_themselves() {
		let output = handle(invocation("translate"));
		assert!(
			output
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Text { text } if text.contains("empty")))
		);
	}

	#[test]
	fn abi_roundtrip_through_dispatch_typed() {
		let source = r#"{"action":"translate","values":{"request":"1","target":"th — Thai"},"selected_message":"hi","composer":null,"storage":null}"#;
		let result = dispatch_typed(source.as_bytes(), handle).unwrap();
		let result: AppOutput = serde_json::from_slice(&result).unwrap();
		assert!(matches!(
			&result.effects[..],
			[HostEffect::AppAction {
				action: AppAction::RequestTranslation { target, .. }
			}] if target == "th"
		));
	}
}
